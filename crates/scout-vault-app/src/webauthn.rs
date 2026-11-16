//! Second factor with a security key (FIDO2/WebAuthn) or Windows Hello,
//! using the native Windows API (`webauthn.dll`) — the same one browsers use.
//!
//! The server sends the assertion options in `TwoFactorProviders2["7"]`
//! (`challenge`, `allowCredentials`, `rpId`, `extensions.appid`…). We build the
//! `clientDataJSON` with the **web vault origin** (that is the origin the server
//! accepts), ask Windows for the signature and return the JSON the server
//! expects in `twoFactorToken`: `id`, `rawId`, `type`, `extensions` and
//! `response.{authenticatorData, clientDataJson, signature}` in unpadded base64url.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use serde_json::{json, Value};

/// What the Windows API returns from an assertion.
pub struct Assertion {
    pub credential_id: Vec<u8>,
    pub authenticator_data: Vec<u8>,
    pub signature: Vec<u8>,
    /// The key used the legacy AppID (U2F) instead of the `rpId`.
    pub used_app_id: bool,
}

/// Relevant assertion options, read from the server's JSON.
#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    pub challenge: String,
    pub rp_id: Option<String>,
    pub credential_ids: Vec<Vec<u8>>,
    pub timeout_ms: u32,
    pub app_id: Option<String>,
}

fn b64url_decode(s: &str) -> Option<Vec<u8>> {
    URL_SAFE_NO_PAD
        .decode(s.trim_end_matches('='))
        .ok()
        .or_else(|| base64::engine::general_purpose::STANDARD.decode(s).ok())
}

pub fn parse_options(v: &Value) -> Result<Options, String> {
    let challenge = v
        .get("challenge")
        .and_then(Value::as_str)
        .ok_or(t!("webauthn.no_challenge"))?
        .to_owned();
    let credential_ids = v
        .get("allowCredentials")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|c| c.get("id").and_then(Value::as_str).and_then(b64url_decode))
                .collect()
        })
        .unwrap_or_default();
    Ok(Options {
        challenge,
        rp_id: v.get("rpId").and_then(Value::as_str).map(str::to_owned),
        credential_ids,
        timeout_ms: v
            .get("timeout")
            .and_then(Value::as_u64)
            .map(|t| t.clamp(30_000, 300_000) as u32)
            .unwrap_or(120_000),
        app_id: v
            .get("extensions")
            .and_then(|e| e.get("appid"))
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

/// `clientDataJSON` of an assertion (`webauthn.get`) for the given origin.
pub fn client_data_json(challenge: &str, origin: &str) -> String {
    json!({
        "type": "webauthn.get",
        "challenge": challenge,
        "origin": origin.trim_end_matches('/'),
        "crossOrigin": false,
    })
    .to_string()
}

/// The `twoFactorToken` in the web vault's format.
pub fn token_json(a: &Assertion, client_data: &str) -> String {
    let id = URL_SAFE_NO_PAD.encode(&a.credential_id);
    let extensions = if a.used_app_id {
        json!({ "appid": true })
    } else {
        json!({})
    };
    json!({
        "id": id,
        "rawId": id,
        "type": "public-key",
        "extensions": extensions,
        "response": {
            "authenticatorData": URL_SAFE_NO_PAD.encode(&a.authenticator_data),
            "clientDataJson": URL_SAFE_NO_PAD.encode(client_data.as_bytes()),
            "signature": URL_SAFE_NO_PAD.encode(&a.signature),
        },
    })
    .to_string()
}

/// Host of a URL (`https://vault.bitwarden.com/` → `vault.bitwarden.com`).
pub fn host_of(url: &str) -> String {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    host.rsplit_once(':')
        .filter(|(_, port)| port.chars().all(|c| c.is_ascii_digit()))
        .map(|(h, _)| h)
        .unwrap_or(host)
        .to_owned()
}

/// Asks for the signature (touch the key / Windows Hello) and returns the ready token.
/// Blocks until the person responds — call it off the UI thread.
pub fn sign(options: &Value, vault_origin: &str) -> Result<String, String> {
    let opts = parse_options(options)?;
    let rp_id = opts.rp_id.clone().unwrap_or_else(|| host_of(vault_origin));
    let client_data = client_data_json(&opts.challenge, vault_origin);
    let assertion = platform::get_assertion(&rp_id, &client_data, &opts)?;
    Ok(token_json(&assertion, &client_data))
}

#[cfg(windows)]
mod platform {
    use windows::core::{BOOL, HSTRING, PCWSTR};
    use windows::Win32::Networking::WindowsWebServices::*;
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

    use super::{Assertion, Options};

    pub fn get_assertion(
        rp_id: &str,
        client_data: &str,
        opts: &Options,
    ) -> Result<Assertion, String> {
        let mut client_data_bytes = client_data.as_bytes().to_vec();
        let client = WEBAUTHN_CLIENT_DATA {
            dwVersion: WEBAUTHN_CLIENT_DATA_CURRENT_VERSION,
            cbClientDataJSON: client_data_bytes.len() as u32,
            pbClientDataJSON: client_data_bytes.as_mut_ptr(),
            pwszHashAlgId: WEBAUTHN_HASH_ALGORITHM_SHA_256,
        };

        let mut ids: Vec<Vec<u8>> = opts.credential_ids.clone();
        let mut creds: Vec<WEBAUTHN_CREDENTIAL> = ids
            .iter_mut()
            .map(|id| WEBAUTHN_CREDENTIAL {
                dwVersion: WEBAUTHN_CREDENTIAL_CURRENT_VERSION,
                cbId: id.len() as u32,
                pbId: id.as_mut_ptr(),
                pwszCredentialType: WEBAUTHN_CREDENTIAL_TYPE_PUBLIC_KEY,
            })
            .collect();

        let app_id = opts.app_id.as_deref().map(HSTRING::from);
        let mut used_app_id = BOOL(0);
        let options = WEBAUTHN_AUTHENTICATOR_GET_ASSERTION_OPTIONS {
            // Version 2: the first with the legacy AppID (U2F); works since Windows 10 1903.
            dwVersion: WEBAUTHN_AUTHENTICATOR_GET_ASSERTION_OPTIONS_VERSION_2,
            dwTimeoutMilliseconds: opts.timeout_ms,
            CredentialList: WEBAUTHN_CREDENTIALS {
                cCredentials: creds.len() as u32,
                pCredentials: creds.as_mut_ptr(),
            },
            dwAuthenticatorAttachment: WEBAUTHN_AUTHENTICATOR_ATTACHMENT_ANY,
            dwUserVerificationRequirement: WEBAUTHN_USER_VERIFICATION_REQUIREMENT_DISCOURAGED,
            pwszU2fAppId: app_id
                .as_ref()
                .map(|s| PCWSTR(s.as_ptr()))
                .unwrap_or(PCWSTR::null()),
            pbU2fAppId: if app_id.is_some() {
                &mut used_app_id
            } else {
                std::ptr::null_mut()
            },
            ..Default::default()
        };

        let rp = HSTRING::from(rp_id);
        // SAFETY: every pointer points to buffers that live until the end of this function;
        // the returned assertion is copied and freed with WebAuthNFreeAssertion.
        unsafe {
            let hwnd = GetForegroundWindow();
            let ptr = WebAuthNAuthenticatorGetAssertion(hwnd, &rp, &client, Some(&options))
                .map_err(|e| {
                    if e.code().0 as u32 == 0x800704C7 {
                        t!("webauthn.canceled").to_owned()
                    } else {
                        t!("webauthn.no_answer", e = e.message())
                    }
                })?;
            let a = &*ptr;
            let copy = |p: *mut u8, n: u32| std::slice::from_raw_parts(p, n as usize).to_vec();
            let result = Assertion {
                credential_id: copy(a.Credential.pbId, a.Credential.cbId),
                authenticator_data: copy(a.pbAuthenticatorData, a.cbAuthenticatorData),
                signature: copy(a.pbSignature, a.cbSignature),
                used_app_id: used_app_id.as_bool(),
            };
            WebAuthNFreeAssertion(ptr);
            Ok(result)
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::{Assertion, Options};

    pub fn get_assertion(_: &str, _: &str, _: &Options) -> Result<Assertion, String> {
        Err(t!("webauthn.windows_only").into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_server_options() {
        let v = json!({
            "challenge": "q83vEjRWeJA",
            "timeout": 60000,
            "rpId": "vault.bitwarden.com",
            "allowCredentials": [{"type": "public-key", "id": "AQID"}],
            "userVerification": "discouraged",
            "extensions": {"appid": "https://vault.bitwarden.com/app-id.json"},
        });
        let o = parse_options(&v).unwrap();
        assert_eq!(o.challenge, "q83vEjRWeJA");
        assert_eq!(o.rp_id.as_deref(), Some("vault.bitwarden.com"));
        assert_eq!(o.credential_ids, vec![vec![1, 2, 3]]);
        assert_eq!(o.timeout_ms, 60000);
        assert_eq!(
            o.app_id.as_deref(),
            Some("https://vault.bitwarden.com/app-id.json")
        );
    }

    #[test]
    fn token_has_the_shape_the_server_reads() {
        let cd = client_data_json("q83vEjRWeJA", "https://vault.bitwarden.com/");
        let v: Value = serde_json::from_str(&cd).unwrap();
        assert_eq!(v["type"], "webauthn.get");
        assert_eq!(v["origin"], "https://vault.bitwarden.com");

        let token = token_json(
            &Assertion {
                credential_id: vec![1, 2, 3],
                authenticator_data: vec![4],
                signature: vec![5],
                used_app_id: true,
            },
            &cd,
        );
        let t: Value = serde_json::from_str(&token).unwrap();
        assert_eq!(t["id"], "AQID");
        assert_eq!(t["rawId"], "AQID");
        assert_eq!(t["type"], "public-key");
        assert_eq!(t["extensions"]["appid"], true);
        assert_eq!(t["response"]["signature"], "BQ");
        let cd_back = URL_SAFE_NO_PAD
            .decode(t["response"]["clientDataJson"].as_str().unwrap())
            .unwrap();
        assert_eq!(cd_back, cd.as_bytes());
    }

    #[test]
    fn host_is_extracted() {
        assert_eq!(
            host_of("https://vault.bitwarden.com/"),
            "vault.bitwarden.com"
        );
        assert_eq!(host_of("http://localhost:8000"), "localhost");
    }
}
