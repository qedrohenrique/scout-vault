//! Company SSO login, following the same path as Bitwarden's official CLI:
//!
//! 1. Opens the web vault at `<vault>/#/sso?clientId=cli&redirectUri=…&state=…&codeChallenge=…&identifier=…`.
//! 2. The company authenticates the person in the browser and the server redirects to
//!    `http://localhost:<8065..8070>/?code=…&state=…` — only these ports are accepted
//!    by the server for the `cli` client.
//! 3. A minimal HTTP server here receives the code, checks the `state` and answers with
//!    a page saying to go back to Scout Vault.
//! 4. The code is exchanged for tokens (`grant_type=authorization_code`, PKCE S256).

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use scout_vault_core::api::SsoCode;
use sha2::{Digest, Sha256};

/// Ports registered on the server for the `cli` client.
pub const PORTS: std::ops::RangeInclusive<u16> = 8065..=8070;
/// How long to wait for the person to finish logging in in the browser.
pub const TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// 64 random alphanumeric characters (PKCE verifier and `state`).
pub fn random_token() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut bytes = [0u8; 64];
    getrandom::fill(&mut bytes).expect("RNG do sistema");
    // 256 isn't a multiple of 62, but the residual bias is irrelevant for a 64-char token.
    bytes
        .iter()
        .map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char)
        .collect()
}

/// PKCE `code_challenge` (S256): unpadded base64url of the verifier's SHA-256.
pub fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// Percent-encoding for query string values.
pub fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => match u8::from_str_radix(&s[i + 1..i + 3], 16) {
                Ok(v) => {
                    out.push(v);
                    i += 3;
                    continue;
                }
                Err(_) => out.push(b'%'),
            },
            b'+' => out.push(b' '),
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Value of a query string parameter (`a=1&b=2`).
pub fn query_param(query: &str, name: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == name).then(|| decode(v))
    })
}

pub fn sso_url(
    vault: &str,
    redirect_uri: &str,
    state: &str,
    challenge: &str,
    identifier: &str,
) -> String {
    format!(
        "{}/#/sso?clientId=cli&redirectUri={}&state={}&codeChallenge={}&identifier={}",
        vault.trim_end_matches('/'),
        encode(redirect_uri),
        state,
        challenge,
        encode(identifier)
    )
}

/// First free port between 8065 and 8070.
pub fn bind() -> Result<(TcpListener, u16), String> {
    for port in PORTS {
        if let Ok(listener) = TcpListener::bind(("127.0.0.1", port)) {
            return Ok((listener, port));
        }
    }
    Err(t!("sso.ports_busy").into())
}

/// Opens an address in the default browser.
pub fn open_browser(url: &str) -> Result<(), String> {
    #[cfg(windows)]
    let result = std::process::Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler", url])
        .spawn();
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(url).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let result = std::process::Command::new("xdg-open").arg(url).spawn();
    result
        .map(|_| ())
        .map_err(|e| t!("sso.browser_failed", e = e))
}

/// Simple page shown in the browser tab at the end of SSO.
fn page(title: &str, body: &str) -> String {
    format!(
        "<!doctype html><meta charset=utf-8><title>Scout Vault</title><body style=\"font-family:system-ui;padding:3em;text-align:center\"><h2>{title}</h2><p>{body}</p>"
    )
}

fn respond(mut stream: TcpStream, status: &str, body: &str) {
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

/// Waits for the browser redirect with `code` and `state`. The `state` that
/// comes back may carry `_identifier=<org>` at the end; only the part before it counts.
pub fn wait_for_code(
    listener: &TcpListener,
    state: &str,
    timeout: Duration,
) -> Result<String, String> {
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let deadline = Instant::now() + timeout;
    loop {
        match listener.accept() {
            Ok((mut stream, _)) => {
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                let mut buf = [0u8; 8192];
                let n = stream.read(&mut buf).unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]);
                // "GET /?code=…&state=… HTTP/1.1"
                let target = request.split_whitespace().nth(1).unwrap_or("");
                let Some((_, query)) = target.split_once('?') else {
                    // Requests without a query (e.g. favicon) don't count.
                    respond(stream, "404 Not Found", "");
                    continue;
                };
                let code = query_param(query, "code");
                let got_state = query_param(query, "state");
                let state_ok = got_state
                    .as_deref()
                    .map(|s| s.split("_identifier=").next() == Some(state))
                    .unwrap_or(false);
                return match (code, state_ok) {
                    (Some(code), true) => {
                        respond(
                            stream,
                            "200 OK",
                            &page(t!("sso.page_ok_title"), t!("sso.page_ok_body")),
                        );
                        Ok(code)
                    }
                    _ => {
                        respond(
                            stream,
                            "400 Bad Request",
                            &page(t!("sso.page_fail_title"), t!("sso.page_fail_body")),
                        );
                        Err(t!("sso.mismatch").into())
                    }
                };
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Err(t!("sso.timeout").into());
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(format!("Falha ao receber o retorno do SSO: {e}")),
        }
    }
}

/// Does the browser part and returns the code to exchange for tokens.
/// Blocks until the person finishes (or [`TIMEOUT`]) — call it off the UI thread.
pub fn authorize(vault: &str, identifier: &str) -> Result<SsoCode, String> {
    let (listener, port) = bind()?;
    let redirect_uri = format!("http://localhost:{port}");
    let verifier = random_token();
    let state = random_token();
    let url = sso_url(
        vault,
        &redirect_uri,
        &state,
        &pkce_challenge(&verifier),
        identifier,
    );
    open_browser(&url)?;
    let code = wait_for_code(&listener, &state, TIMEOUT)?;
    Ok(SsoCode {
        code,
        verifier,
        redirect_uri,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_matches_rfc7636_example() {
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn random_tokens_are_long_and_different() {
        let (a, b) = (random_token(), random_token());
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric()));
        assert_ne!(a, b);
    }

    #[test]
    fn url_is_built_and_encoded() {
        let url = sso_url(
            "https://vault.bitwarden.com/",
            "http://localhost:8065",
            "st",
            "ch",
            "Minha Empresa",
        );
        assert_eq!(
            url,
            "https://vault.bitwarden.com/#/sso?clientId=cli&redirectUri=http%3A%2F%2Flocalhost%3A8065&state=st&codeChallenge=ch&identifier=Minha%20Empresa"
        );
        assert_eq!(
            query_param("a=1&code=x%2By&state=s", "code").as_deref(),
            Some("x+y")
        );
    }

    fn send(port: u16, path: &str) -> String {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(s, "GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
        let mut out = String::new();
        let _ = s.read_to_string(&mut out);
        out
    }

    #[test]
    fn callback_with_matching_state_returns_code() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let client = std::thread::spawn(move || {
            send(port, "/favicon.ico");
            send(port, "/?code=abc%3D&state=st123_identifier=empresa")
        });
        let code = wait_for_code(&listener, "st123", Duration::from_secs(5)).unwrap();
        assert_eq!(code, "abc=");
        assert!(client.join().unwrap().starts_with("HTTP/1.1 200"));
    }

    #[test]
    fn callback_with_wrong_state_is_rejected() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let client = std::thread::spawn(move || send(port, "/?code=abc&state=outro"));
        assert!(wait_for_code(&listener, "st123", Duration::from_secs(5)).is_err());
        assert!(client.join().unwrap().starts_with("HTTP/1.1 400"));
    }
}
