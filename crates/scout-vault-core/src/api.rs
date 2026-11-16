//! HTTP client for Bitwarden-compatible servers (official and Vaultwarden).
//!
//! It only carries **encrypted** data: nothing here decrypts. Typical flow:
//!
//! ```no_run
//! # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
//! use scout_vault_core::api::{Client, DeviceInfo, ServerUrls};
//! use scout_vault_core::crypto::MasterKey;
//! use scout_vault_core::vault::{Keys, Vault};
//!
//! let (email, password) = ("me@example.com", "master password");
//! let client = Client::new(ServerUrls::self_hosted("http://localhost:8000"), DeviceInfo::new("Scout Vault"))?;
//! let kdf = client.prelogin(email).await?;
//! let master_key = MasterKey::derive(password, email, kdf)?; // slow: off the UI thread
//! let tokens = client.login_password(email, &master_key.server_hash(password), None, None).await?;
//! let sync = client.sync(&tokens.access_token).await?;
//! let keys = Keys::unlock(&master_key, &sync.profile)?;
//! let vault = Vault::decrypt(&sync, &keys);
//! # Ok(()) }
//! ```

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::StatusCode;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[cfg(test)]
use crate::json::camel_case;

use crate::kdf::{Kdf, KdfError, PreloginResponse};

/// Client name accepted by the server (the official server's `BitwardenClient` enum).
const CLIENT_ID: &str = "desktop";
/// Client used for SSO login: the server only accepts redirecting to
/// `http://localhost:8065..8070` for the `cli`/`desktop` clients, and `desktop`
/// prefers `bitwarden://`, which belongs to the official app.
pub const SSO_CLIENT_ID: &str = "cli";
/// Version announced to the server. Vaultwarden uses it to decide, for example,
/// whether to send SSH keys in the sync. The official server **rejects old versions**
/// ("Please update your app to continue using Bitwarden"): in Nov 2026
/// `2024.12.0` was already rejected and `2025.1.0` onwards accepted. Keep it close
/// to the current version of the official desktop app.
const CLIENT_VERSION: &str = "2026.10.0";

/// Protocol `DeviceType`: 6 = Windows, 7 = macOS, 8 = Linux (desktop).
const DEVICE_TYPE: u8 = if cfg!(target_os = "windows") {
    6
} else if cfg!(target_os = "macos") {
    7
} else {
    8
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerUrls {
    pub api: String,
    pub identity: String,
    /// Web vault (used for SSO login).
    #[serde(default)]
    pub vault: String,
}

impl ServerUrls {
    pub fn bitwarden_us() -> Self {
        Self {
            api: "https://api.bitwarden.com".into(),
            identity: "https://identity.bitwarden.com".into(),
            vault: "https://vault.bitwarden.com".into(),
        }
    }

    pub fn bitwarden_eu() -> Self {
        Self {
            api: "https://api.bitwarden.eu".into(),
            identity: "https://identity.bitwarden.eu".into(),
            vault: "https://vault.bitwarden.eu".into(),
        }
    }

    /// Self-hosted server (Vaultwarden or Bitwarden self-hosted): `<base>/api` and `<base>/identity`.
    pub fn self_hosted(base: &str) -> Self {
        let base = base.trim().trim_end_matches('/');
        Self {
            api: format!("{base}/api"),
            identity: format!("{base}/identity"),
            vault: base.to_owned(),
        }
    }
}

/// Code returned by the browser at the end of SSO login.
#[derive(Debug, Clone)]
pub struct SsoCode {
    pub code: String,
    /// PKCE verifier (the `code_challenge` was its SHA-256).
    pub verifier: String,
    pub redirect_uri: String,
}

/// How the account opens the vault after login (`UserDecryptionOptions`).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct UserDecryptionOptions {
    #[serde(rename = "hasMasterPassword", default)]
    pub has_master_password: Option<bool>,
    #[serde(rename = "keyConnectorOption", default)]
    pub key_connector: Option<Value>,
    #[serde(rename = "trustedDeviceOption", default)]
    pub trusted_device: Option<Value>,
}

/// This device's identity towards the server. The `identifier` must be
/// **persisted** and reused — a new one on every login triggers "new device"
/// alerts and email verification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub identifier: String,
    pub name: String,
}

impl DeviceInfo {
    pub fn new(name: &str) -> Self {
        Self {
            identifier: uuid::Uuid::new_v4().to_string(),
            name: name.to_owned(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TwoFactorProvider {
    Authenticator,
    Email,
    Duo,
    Yubikey,
    U2f,
    Remember,
    OrganizationDuo,
    WebAuthn,
    Other(u8),
}

impl TwoFactorProvider {
    pub fn code(self) -> u8 {
        match self {
            Self::Authenticator => 0,
            Self::Email => 1,
            Self::Duo => 2,
            Self::Yubikey => 3,
            Self::U2f => 4,
            Self::Remember => 5,
            Self::OrganizationDuo => 6,
            Self::WebAuthn => 7,
            Self::Other(n) => n,
        }
    }

    pub fn from_code(code: u8) -> Self {
        match code {
            0 => Self::Authenticator,
            1 => Self::Email,
            2 => Self::Duo,
            3 => Self::Yubikey,
            4 => Self::U2f,
            5 => Self::Remember,
            6 => Self::OrganizationDuo,
            7 => Self::WebAuthn,
            n => Self::Other(n),
        }
    }
}

/// What the server returns when it asks for the second factor.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TwoFactorChallenge {
    pub providers: Vec<TwoFactorProvider>,
    /// Session token the official server requires to resend the code by
    /// email (`ssoEmail2faSessionToken`). Vaultwarden doesn't send it.
    pub email_session_token: Option<String>,
    /// Data for each method (`TwoFactorProviders2`), by method code:
    /// security key options (7), Duo `AuthUrl` (2/6), masked email (1)…
    pub provider_data: Vec<(TwoFactorProvider, Value)>,
}

/// Second factor sent along with the login.
#[derive(Debug, Clone)]
pub struct SecondFactor {
    pub provider: TwoFactorProvider,
    pub token: String,
    /// Asks the server for a token to skip 2FA on this device next time.
    pub remember: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("erro de rede: {0}")]
    Http(#[from] reqwest::Error),
    #[error("resposta inesperada do servidor: {0}")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Kdf(#[from] KdfError),
    #[error("credenciais inválidas: {0}")]
    InvalidCredentials(String),
    #[error("segundo fator necessário")]
    TwoFactorRequired(TwoFactorChallenge),
    #[error("código do segundo fator inválido: {0}")]
    InvalidTwoFactorCode(String),
    #[error("o servidor pediu verificação de novo dispositivo (código enviado por e-mail)")]
    NewDeviceVerificationRequired,
    #[error("código de verificação de novo dispositivo inválido")]
    InvalidNewDeviceCode,
    #[error("o servidor exige um captcha, que só os apps oficiais resolvem")]
    CaptchaRequired,
    /// Too many attempts in a row (HTTP 429); the server allows it again after a while.
    #[error("muitas tentativas; espere um pouco")]
    RateLimited,
    #[error("sessão expirada ou token inválido")]
    Unauthorized,
    #[error("servidor respondeu {status}: {body}")]
    Status { status: u16, body: String },
}

/// Result of a successful login.
#[derive(Debug, Clone, Deserialize)]
pub struct Tokens {
    pub access_token: String,
    /// Access token lifetime, in seconds.
    #[serde(default)]
    pub expires_in: u64,
    pub refresh_token: String,
    /// Present when the login used `SecondFactor { remember: true, .. }`.
    #[serde(rename = "twoFactorToken", default)]
    pub two_factor_token: Option<String>,
    /// User key encrypted with the master key. The sync also brings it in `profile.key`;
    /// this one is a fallback in case the sync stops bringing it.
    #[serde(default)]
    pub key: Option<String>,
    #[serde(rename = "userDecryptionOptions", default)]
    pub user_decryption_options: Option<UserDecryptionOptions>,
}

impl Tokens {
    /// Account email, read from the access token's JWT (without checking the signature:
    /// it is only used for the KDF; the server is the one that validates the token).
    pub fn email(&self) -> Option<String> {
        let payload = self.access_token.split('.').nth(1)?;
        let bytes = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
        let claims: Value = serde_json::from_slice(&bytes).ok()?;
        claims.get("email")?.as_str().map(str::to_owned)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct RefreshedTokens {
    pub access_token: String,
    #[serde(default)]
    pub expires_in: u64,
    /// Some servers rotate the refresh token; if one comes, replace the old one.
    #[serde(default)]
    pub refresh_token: Option<String>,
}

/// Response to registering a new attachment.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentUpload {
    pub attachment_id: String,
    #[serde(default)]
    pub url: Option<String>,
    /// 0 = directly on the server (Vaultwarden, self-hosted); 1 = Azure (cloud).
    #[serde(default)]
    pub file_upload_type: u8,
}

pub struct Client {
    http: reqwest::Client,
    urls: ServerUrls,
    device: DeviceInfo,
    /// OAuth `client_id`: the one used for login must be used for refresh too.
    client_id: &'static str,
}

impl Client {
    pub fn new(urls: ServerUrls, device: DeviceInfo) -> Result<Self, ApiError> {
        let mut headers = HeaderMap::new();
        headers.insert("Bitwarden-Client-Name", HeaderValue::from_static(CLIENT_ID));
        headers.insert(
            "Bitwarden-Client-Version",
            HeaderValue::from_static(CLIENT_VERSION),
        );
        headers.insert("Device-Type", HeaderValue::from(u16::from(DEVICE_TYPE)));

        let http = reqwest::Client::builder()
            .user_agent(concat!("scout-vault/", env!("CARGO_PKG_VERSION")))
            .default_headers(headers)
            .build()?;
        Ok(Self {
            http,
            urls,
            device,
            client_id: CLIENT_ID,
        })
    }

    /// Client for SSO login (and for renewing its tokens).
    pub fn for_sso(urls: ServerUrls, device: DeviceInfo) -> Result<Self, ApiError> {
        let mut client = Self::new(urls, device)?;
        client.client_id = SSO_CLIENT_ID;
        Ok(client)
    }

    pub fn device(&self) -> &DeviceInfo {
        &self.device
    }

    /// This client uses the SSO login `client_id`.
    pub fn is_sso(&self) -> bool {
        self.client_id == SSO_CLIENT_ID
    }

    pub fn urls(&self) -> &ServerUrls {
        &self.urls
    }

    /// `POST /identity/accounts/prelogin` — finds out the account's KDF.
    pub async fn prelogin(&self, email: &str) -> Result<Kdf, ApiError> {
        let res = self
            .http
            .post(format!("{}/accounts/prelogin", self.urls.identity))
            .json(&serde_json::json!({ "email": email }))
            .send()
            .await?;
        let raw: PreloginResponse = read_json(res).await?;
        Ok(raw.try_into()?)
    }

    /// `POST /identity/connect/token` with `grant_type=password`.
    ///
    /// `server_hash` comes from [`MasterKey::server_hash`](crate::crypto::MasterKey::server_hash).
    /// If the server answers [`ApiError::TwoFactorRequired`], call again with `second_factor`;
    /// if it answers [`ApiError::NewDeviceVerificationRequired`], with `new_device_otp`.
    pub async fn login_password(
        &self,
        email: &str,
        server_hash: &str,
        second_factor: Option<&SecondFactor>,
        new_device_otp: Option<&str>,
    ) -> Result<Tokens, ApiError> {
        let mut form: Vec<(&str, String)> = vec![
            ("grant_type", "password".into()),
            ("username", email.into()),
            ("password", server_hash.into()),
            ("scope", "api offline_access".into()),
            ("client_id", self.client_id.into()),
            ("deviceType", DEVICE_TYPE.to_string()),
            ("deviceIdentifier", self.device.identifier.clone()),
            ("deviceName", self.device.name.clone()),
            ("devicePushToken", String::new()),
        ];
        if let Some(sf) = second_factor {
            form.push(("twoFactorProvider", sf.provider.code().to_string()));
            form.push(("twoFactorToken", sf.token.clone()));
            form.push(("twoFactorRemember", u8::from(sf.remember).to_string()));
        }
        if let Some(otp) = new_device_otp {
            form.push(("newDeviceOtp", otp.into()));
        }

        let res = self
            .http
            .post(format!("{}/connect/token", self.urls.identity))
            .header("Auth-Email", URL_SAFE_NO_PAD.encode(email))
            .header("Device-Identifier", &self.device.identifier)
            .form(&form)
            .send()
            .await?;

        if res.status().is_success() {
            return read_json(res).await;
        }
        let status = res.status().as_u16();
        let body = res.text().await?;
        Err(classify_login_error(status, &body))
    }

    /// `POST /identity/connect/token` with `grant_type=authorization_code` (end of SSO).
    pub async fn login_sso(&self, sso: &SsoCode) -> Result<Tokens, ApiError> {
        let form: Vec<(&str, String)> = vec![
            ("grant_type", "authorization_code".into()),
            ("code", sso.code.clone()),
            ("code_verifier", sso.verifier.clone()),
            ("redirect_uri", sso.redirect_uri.clone()),
            ("scope", "api offline_access".into()),
            ("client_id", self.client_id.into()),
            ("deviceType", DEVICE_TYPE.to_string()),
            ("deviceIdentifier", self.device.identifier.clone()),
            ("deviceName", self.device.name.clone()),
            ("devicePushToken", String::new()),
        ];
        let res = self
            .http
            .post(format!("{}/connect/token", self.urls.identity))
            .header("Device-Identifier", &self.device.identifier)
            .form(&form)
            .send()
            .await?;
        if res.status().is_success() {
            return read_json(res).await;
        }
        let status = res.status().as_u16();
        let body = res.text().await?;
        Err(classify_login_error(status, &body))
    }

    /// Asks the server to send the 2FA code by email.
    ///
    /// The official server authenticates the request with the `session_token` that came in
    /// [`TwoFactorChallenge::email_session_token`]; Vaultwarden, with the password
    /// hash. We send both.
    pub async fn send_email_login_code(
        &self,
        email: &str,
        server_hash: &str,
        session_token: Option<&str>,
    ) -> Result<(), ApiError> {
        let mut body = serde_json::json!({
            "email": email,
            "masterPasswordHash": server_hash,
            "deviceIdentifier": self.device.identifier,
        });
        if let Some(token) = session_token {
            body["ssoEmail2FaSessionToken"] = token.into();
        }
        let res = self
            .http
            .post(format!("{}/two-factor/send-email-login", self.urls.api))
            .header("Auth-Email", URL_SAFE_NO_PAD.encode(email))
            .header("Device-Identifier", &self.device.identifier)
            .json(&body)
            .send()
            .await?;
        ensure_success(res).await.map(|_| ())
    }

    /// Exchanges the refresh token for a new access token.
    pub async fn refresh(&self, refresh_token: &str) -> Result<RefreshedTokens, ApiError> {
        let res = self
            .http
            .post(format!("{}/connect/token", self.urls.identity))
            .form(&[
                ("grant_type", "refresh_token"),
                ("client_id", self.client_id),
                ("refresh_token", refresh_token),
            ])
            .send()
            .await?;
        if matches!(
            res.status(),
            StatusCode::BAD_REQUEST | StatusCode::UNAUTHORIZED
        ) {
            return Err(ApiError::Unauthorized);
        }
        read_json(res).await
    }

    /// `POST /api/ciphers` — creates a personal item. Returns the item as stored on the server.
    pub async fn create_cipher(
        &self,
        access_token: &str,
        body: &Value,
    ) -> Result<crate::sync::CipherRaw, ApiError> {
        let res = self
            .http
            .post(format!("{}/ciphers", self.urls.api))
            .bearer_auth(access_token)
            .json(body)
            .send()
            .await?;
        read_json(res).await
    }

    /// `POST /api/ciphers/create` — creates an item **in an organization**, already in its collections.
    pub async fn create_cipher_in_org(
        &self,
        access_token: &str,
        body: &Value,
        collection_ids: &[String],
    ) -> Result<crate::sync::CipherRaw, ApiError> {
        let res = self
            .http
            .post(format!("{}/ciphers/create", self.urls.api))
            .bearer_auth(access_token)
            .json(&serde_json::json!({ "cipher": body, "collectionIds": collection_ids }))
            .send()
            .await?;
        read_json(res).await
    }

    /// Changes the collections of an organization item. Newer servers use
    /// `collections_v2`; older ones, `collections`.
    pub async fn update_collections(
        &self,
        access_token: &str,
        id: &str,
        collection_ids: &[String],
    ) -> Result<(), ApiError> {
        let body = serde_json::json!({ "collectionIds": collection_ids });
        for path in ["collections_v2", "collections"] {
            let res = self
                .http
                .put(format!("{}/ciphers/{id}/{path}", self.urls.api))
                .bearer_auth(access_token)
                .json(&body)
                .send()
                .await?;
            if matches!(res.status().as_u16(), 404 | 405) && path == "collections_v2" {
                continue;
            }
            return ensure_success(res).await.map(|_| ());
        }
        Ok(())
    }

    /// `PUT /api/ciphers/{id}` — saves an item edit.
    pub async fn update_cipher(
        &self,
        access_token: &str,
        id: &str,
        body: &Value,
    ) -> Result<crate::sync::CipherRaw, ApiError> {
        let res = self
            .http
            .put(format!("{}/ciphers/{id}", self.urls.api))
            .bearer_auth(access_token)
            .json(body)
            .send()
            .await?;
        read_json(res).await
    }

    /// `PUT /api/ciphers/{id}/partial` — changes only the item's folder (and favorite),
    /// without re-encrypting anything. Also works for organization items (the folder is personal).
    pub async fn move_cipher_to_folder(
        &self,
        access_token: &str,
        id: &str,
        folder_id: Option<&str>,
        favorite: bool,
    ) -> Result<(), ApiError> {
        let res = self
            .http
            .put(format!("{}/ciphers/{id}/partial", self.urls.api))
            .bearer_auth(access_token)
            .json(&serde_json::json!({ "folderId": folder_id, "favorite": favorite }))
            .send()
            .await?;
        ensure_success(res).await.map(|_| ())
    }

    /// `POST /api/folders` — creates a folder. `name` is already encrypted with the user key.
    pub async fn create_folder(
        &self,
        access_token: &str,
        name: &str,
    ) -> Result<crate::sync::FolderRaw, ApiError> {
        let res = self
            .http
            .post(format!("{}/folders", self.urls.api))
            .bearer_auth(access_token)
            .json(&serde_json::json!({ "name": name }))
            .send()
            .await?;
        read_json(res).await
    }

    /// `PUT /api/folders/{id}` — renames (encrypted name).
    pub async fn rename_folder(
        &self,
        access_token: &str,
        id: &str,
        name: &str,
    ) -> Result<crate::sync::FolderRaw, ApiError> {
        let res = self
            .http
            .put(format!("{}/folders/{id}", self.urls.api))
            .bearer_auth(access_token)
            .json(&serde_json::json!({ "name": name }))
            .send()
            .await?;
        read_json(res).await
    }

    /// `DELETE /api/folders/{id}` — deletes the folder; its items end up with no folder.
    pub async fn delete_folder(&self, access_token: &str, id: &str) -> Result<(), ApiError> {
        let res = self
            .http
            .delete(format!("{}/folders/{id}", self.urls.api))
            .bearer_auth(access_token)
            .send()
            .await?;
        ensure_success(res).await.map(|_| ())
    }

    /// `PUT /api/ciphers/{id}/delete` — moves to the trash (can be restored).
    pub async fn trash_cipher(&self, access_token: &str, id: &str) -> Result<(), ApiError> {
        let res = self
            .http
            .put(format!("{}/ciphers/{id}/delete", self.urls.api))
            .bearer_auth(access_token)
            .send()
            .await?;
        ensure_success(res).await.map(|_| ())
    }

    /// `PUT /api/ciphers/{id}/restore` — takes it out of the trash.
    pub async fn restore_cipher(&self, access_token: &str, id: &str) -> Result<(), ApiError> {
        let res = self
            .http
            .put(format!("{}/ciphers/{id}/restore", self.urls.api))
            .bearer_auth(access_token)
            .send()
            .await?;
        ensure_success(res).await.map(|_| ())
    }

    /// `DELETE /api/ciphers/{id}` — deletes **permanently** (only items already in the trash).
    pub async fn delete_cipher(&self, access_token: &str, id: &str) -> Result<(), ApiError> {
        let res = self
            .http
            .delete(format!("{}/ciphers/{id}", self.urls.api))
            .bearer_auth(access_token)
            .send()
            .await?;
        ensure_success(res).await.map(|_| ())
    }

    /// Download address of an attachment (`GET /api/ciphers/{id}/attachment/{att}`).
    pub async fn attachment_url(
        &self,
        access_token: &str,
        cipher_id: &str,
        attachment_id: &str,
    ) -> Result<String, ApiError> {
        #[derive(Deserialize)]
        struct Res {
            url: String,
        }
        let res = self
            .http
            .get(format!(
                "{}/ciphers/{cipher_id}/attachment/{attachment_id}",
                self.urls.api
            ))
            .bearer_auth(access_token)
            .send()
            .await?;
        let r: Res = read_json(res).await?;
        Ok(self.absolute(&r.url))
    }

    /// A server-relative address becomes absolute.
    fn absolute(&self, url: &str) -> String {
        if url.starts_with("http://") || url.starts_with("https://") {
            return url.to_owned();
        }
        let base = self.urls.api.trim_end_matches('/').trim_end_matches("/api");
        format!("{base}/{}", url.trim_start_matches('/'))
    }

    /// Downloads the encrypted file (the address is already signed; no token).
    pub async fn download(&self, url: &str) -> Result<Vec<u8>, ApiError> {
        let res = self.http.get(url).send().await?;
        let res = ensure_success(res).await?;
        Ok(res.bytes().await?.to_vec())
    }

    /// Registers a new attachment and returns where to upload the file.
    pub async fn create_attachment(
        &self,
        access_token: &str,
        cipher_id: &str,
        key: &str,
        file_name: &str,
        file_size: usize,
    ) -> Result<AttachmentUpload, ApiError> {
        let res = self
            .http
            .post(format!(
                "{}/ciphers/{cipher_id}/attachment/v2",
                self.urls.api
            ))
            .bearer_auth(access_token)
            .json(&serde_json::json!({
                "key": key,
                "fileName": file_name,
                "fileSize": file_size,
                "adminRequest": false,
            }))
            .send()
            .await?;
        let mut up: AttachmentUpload = read_json(res).await?;
        up.url = up.url.as_deref().map(|u| self.absolute(u));
        Ok(up)
    }

    /// Uploads the encrypted file to the registered attachment.
    pub async fn upload_attachment(
        &self,
        access_token: &str,
        cipher_id: &str,
        upload: &AttachmentUpload,
        file_name: &str,
        data: Vec<u8>,
    ) -> Result<(), ApiError> {
        let res = if upload.file_upload_type == 1 {
            // Bitwarden cloud: straight to storage (Azure), with a signed URL.
            let url = upload.url.clone().ok_or(ApiError::Status {
                status: 0,
                body: "o servidor não mandou o endereço de envio".into(),
            })?;
            self.http
                .put(url)
                .header("x-ms-blob-type", "BlockBlob")
                .header("x-ms-version", "2020-04-08")
                .body(data)
                .send()
                .await?
        } else {
            let part = reqwest::multipart::Part::bytes(data)
                .file_name(file_name.to_owned())
                .mime_str("application/octet-stream")?;
            let form = reqwest::multipart::Form::new().part("data", part);
            self.http
                .post(format!(
                    "{}/ciphers/{cipher_id}/attachment/{}",
                    self.urls.api, upload.attachment_id
                ))
                .bearer_auth(access_token)
                .multipart(form)
                .send()
                .await?
        };
        ensure_success(res).await.map(|_| ())
    }

    /// `DELETE /api/ciphers/{id}/attachment/{att}`.
    pub async fn delete_attachment(
        &self,
        access_token: &str,
        cipher_id: &str,
        attachment_id: &str,
    ) -> Result<(), ApiError> {
        let res = self
            .http
            .delete(format!(
                "{}/ciphers/{cipher_id}/attachment/{attachment_id}",
                self.urls.api
            ))
            .bearer_auth(access_token)
            .send()
            .await?;
        ensure_success(res).await.map(|_| ())
    }

    /// `GET /api/sync` — the whole vault, still encrypted.
    /// `GET /api/accounts/revision-date` — when the vault last changed
    /// (ms since 1970). Cheap: tells whether a sync is worth it.
    pub async fn revision_date(&self, access_token: &str) -> Result<i64, ApiError> {
        let res = self
            .http
            .get(format!("{}/accounts/revision-date", self.urls.api))
            .bearer_auth(access_token)
            .send()
            .await?;
        let value: serde_json::Value = read_json(res).await?;
        value
            .as_i64()
            .or_else(|| value.as_f64().map(|f| f as i64))
            .ok_or_else(|| ApiError::Status {
                status: 200,
                body: format!("revision-date inesperado: {value}"),
            })
    }

    pub async fn sync(&self, access_token: &str) -> Result<crate::sync::SyncResponse, ApiError> {
        let res = self
            .http
            .get(format!("{}/sync?excludeDomains=true", self.urls.api))
            .bearer_auth(access_token)
            .send()
            .await?;
        read_json(res).await
    }
}

async fn ensure_success(res: reqwest::Response) -> Result<reqwest::Response, ApiError> {
    let status = res.status();
    if status == StatusCode::UNAUTHORIZED {
        return Err(ApiError::Unauthorized);
    }
    if !status.is_success() {
        let body = res.text().await.unwrap_or_default();
        return Err(ApiError::Status {
            status: status.as_u16(),
            body,
        });
    }
    Ok(res)
}

async fn read_json<T: DeserializeOwned>(res: reqwest::Response) -> Result<T, ApiError> {
    let res = ensure_success(res).await?;
    let bytes = res.bytes().await?;
    parse_normalized(&bytes)
}

pub(crate) fn parse_normalized<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, ApiError> {
    Ok(crate::json::from_slice_normalized(bytes)?)
}

fn classify_login_error(status: u16, body: &str) -> ApiError {
    if status == 429 {
        return ApiError::RateLimited;
    }
    #[derive(Deserialize)]
    struct ErrorModel {
        message: Option<String>,
    }
    #[derive(Deserialize)]
    struct LoginErrorBody {
        #[serde(default)]
        error: String,
        #[serde(default)]
        error_description: Option<String>,
        #[serde(rename = "errorModel", default)]
        error_model: Option<ErrorModel>,
        #[serde(rename = "twoFactorProviders", default)]
        two_factor_providers: Option<Vec<Value>>,
        #[serde(rename = "twoFactorProviders2", default)]
        two_factor_providers2: Option<serde_json::Map<String, Value>>,
        #[serde(
            rename = "ssoEmail2faSessionToken",
            alias = "ssoEmail2FaSessionToken",
            default
        )]
        email_session_token: Option<String>,
    }

    let Ok(parsed) = parse_normalized::<LoginErrorBody>(body.as_bytes()) else {
        return ApiError::Status {
            status,
            body: body.to_owned(),
        };
    };

    if let Some(providers) = parsed.two_factor_providers {
        // The server sends the codes as numbers or as strings ("0").
        let providers = providers
            .iter()
            .filter_map(|v| match v {
                Value::Number(n) => n.as_u64(),
                Value::String(s) => s.parse().ok(),
                _ => None,
            })
            .filter_map(|n| u8::try_from(n).ok())
            .map(TwoFactorProvider::from_code)
            .collect();
        let provider_data = parsed
            .two_factor_providers2
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(k, v)| Some((TwoFactorProvider::from_code(k.parse().ok()?), v)))
            .collect();
        return ApiError::TwoFactorRequired(TwoFactorChallenge {
            providers,
            email_session_token: parsed.email_session_token,
            provider_data,
        });
    }

    let message = parsed
        .error_model
        .and_then(|m| m.message)
        .or(parsed.error_description.clone())
        .unwrap_or_else(|| parsed.error.clone());

    let lower = message.to_lowercase();
    let description = parsed
        .error_description
        .as_deref()
        .unwrap_or_default()
        .to_lowercase();
    // Messages from the official server (DeviceValidator/BaseRequestValidator) and from Vaultwarden.
    if lower.contains("invalid new device otp") {
        return ApiError::InvalidNewDeviceCode;
    }
    if lower.contains("device verification") {
        return ApiError::NewDeviceVerificationRequired;
    }
    if description.contains("captcha") || lower.contains("captcha") {
        return ApiError::CaptchaRequired;
    }
    if lower.contains("two-step token is invalid")
        || lower.contains("invalid totp code")
        || lower.contains("totp code is not a number")
    {
        return ApiError::InvalidTwoFactorCode(message);
    }
    if parsed.error == "invalid_grant" || parsed.error.is_empty() {
        return ApiError::InvalidCredentials(message);
    }
    ApiError::Status {
        status,
        body: body.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camel_case_rules() {
        assert_eq!(camel_case("PrivateKey"), "privateKey");
        assert_eq!(camel_case("privateKey"), "privateKey");
        assert_eq!(camel_case("SSN"), "ssn");
        assert_eq!(camel_case("access_token"), "access_token");
        assert_eq!(camel_case("Id"), "id");
    }

    #[test]
    fn self_hosted_urls() {
        assert_eq!(
            ServerUrls::self_hosted(" https://vault.exemplo.com/ "),
            ServerUrls {
                api: "https://vault.exemplo.com/api".into(),
                identity: "https://vault.exemplo.com/identity".into(),
                vault: "https://vault.exemplo.com".into(),
            }
        );
    }

    #[test]
    fn two_factor_as_strings_pascal_case() {
        let body = r#"{"error":"invalid_grant","error_description":"Two factor required.",
                       "TwoFactorProviders":["0","1"],"TwoFactorProviders2":{"0":null,"1":{"Email":"e***@x.com"}}}"#;
        match classify_login_error(400, body) {
            ApiError::TwoFactorRequired(c) => {
                assert_eq!(
                    c.providers,
                    [TwoFactorProvider::Authenticator, TwoFactorProvider::Email]
                );
                assert_eq!(c.email_session_token, None);
                // TwoFactorProviders2 comes with numeric keys as text.
                assert!(c
                    .provider_data
                    .iter()
                    .any(|(p, v)| *p == TwoFactorProvider::Email && v["email"] == "e***@x.com"));
            }
            other => panic!("esperava 2FA, veio {other:?}"),
        }
    }

    #[test]
    fn two_factor_as_numbers() {
        let body = r#"{"error":"invalid_grant","twoFactorProviders":[0,7]}"#;
        assert!(matches!(
            classify_login_error(400, body),
            ApiError::TwoFactorRequired(c) if c.providers == [TwoFactorProvider::Authenticator, TwoFactorProvider::WebAuthn]
        ));
    }

    #[test]
    fn wrong_password_official_and_vaultwarden() {
        let official = r#"{"error":"invalid_grant","error_description":"invalid_username_or_password",
                           "ErrorModel":{"Message":"Username or password is incorrect. Try again.","Object":"error"}}"#;
        assert!(matches!(
            classify_login_error(400, official),
            ApiError::InvalidCredentials(m) if m.starts_with("Username or password")
        ));

        let vaultwarden = r#"{"error":"","error_description":"","ErrorModel":{"Message":"Username or password is incorrect. Try again"}}"#;
        assert!(matches!(
            classify_login_error(400, vaultwarden),
            ApiError::InvalidCredentials(_)
        ));
    }

    #[test]
    fn new_device_verification() {
        let body = r#"{"error":"invalid_grant","error_description":"Device verification required.",
                       "ErrorModel":{"Message":"new device verification required"}}"#;
        assert!(matches!(
            classify_login_error(400, body),
            ApiError::NewDeviceVerificationRequired
        ));
    }

    #[test]
    fn official_two_factor_carries_email_session_token() {
        let body = r#"{"error":"invalid_grant","error_description":"Two factor required.",
                       "TwoFactorProviders":["1"],"SsoEmail2faSessionToken":"sess-123"}"#;
        match classify_login_error(400, body) {
            ApiError::TwoFactorRequired(c) => {
                assert_eq!(c.providers, [TwoFactorProvider::Email]);
                assert_eq!(c.email_session_token.as_deref(), Some("sess-123"));
            }
            other => panic!("esperava 2FA, veio {other:?}"),
        }
    }

    #[test]
    fn official_new_device_messages() {
        let required = r#"{"error":"device_error","error_description":"New device verification required",
                           "ErrorModel":{"Message":"new device verification required","Object":"error"}}"#;
        assert!(matches!(
            classify_login_error(400, required),
            ApiError::NewDeviceVerificationRequired
        ));
        let invalid = r#"{"error":"device_error","error_description":"Invalid New Device OTP",
                          "ErrorModel":{"Message":"invalid new device otp","Object":"error"}}"#;
        assert!(matches!(
            classify_login_error(400, invalid),
            ApiError::InvalidNewDeviceCode
        ));
    }

    #[test]
    fn wrong_two_factor_code_is_not_wrong_password() {
        let official = r#"{"error":"invalid_grant","error_description":"invalid_username_or_password",
                           "ErrorModel":{"Message":"Two-step token is invalid. Try again.","Object":"error"}}"#;
        assert!(matches!(
            classify_login_error(400, official),
            ApiError::InvalidTwoFactorCode(_)
        ));
        let vaultwarden = r#"{"error":"","ErrorModel":{"Message":"Invalid TOTP code! Server time: 12:00:00 IP: 1.2.3.4"}}"#;
        assert!(matches!(
            classify_login_error(400, vaultwarden),
            ApiError::InvalidTwoFactorCode(_)
        ));
    }

    #[test]
    fn captcha_is_reported() {
        let body = r#"{"error":"invalid_grant","error_description":"Captcha required.","HCaptcha_SiteKey":"x"}"#;
        assert!(matches!(
            classify_login_error(400, body),
            ApiError::CaptchaRequired
        ));
    }

    #[test]
    fn tokens_parse() {
        let body = br#"{"access_token":"a","expires_in":3600,"token_type":"Bearer",
                        "refresh_token":"r","Key":"2.x|y|z","TwoFactorToken":"t"}"#;
        let t: Tokens = parse_normalized(body).unwrap();
        assert_eq!(
            (
                t.access_token.as_str(),
                t.refresh_token.as_str(),
                t.expires_in
            ),
            ("a", "r", 3600)
        );
        assert_eq!(t.two_factor_token.as_deref(), Some("t"));
        assert_eq!(t.key.as_deref(), Some("2.x|y|z"));
        assert!(t.user_decryption_options.is_none());
    }

    #[test]
    fn too_many_attempts_is_not_a_wrong_password() {
        let e = classify_login_error(429, r#"{"message":"Too many login requests"}"#);
        assert!(matches!(e, ApiError::RateLimited), "{e:?}");
    }

    #[test]
    fn sso_tokens_expose_email_and_decryption_options() {
        // Sample JWT: payload {"email":"eu@exemplo.com"} (signature irrelevant here).
        let payload = URL_SAFE_NO_PAD.encode(br#"{"sub":"1","email":"eu@exemplo.com"}"#);
        let body = format!(
            r#"{{"access_token":"h.{payload}.s","refresh_token":"r","Key":"2.a|b|c",
                "UserDecryptionOptions":{{"HasMasterPassword":false,"KeyConnectorOption":{{"KeyConnectorUrl":"https://kc"}}}}}}"#
        );
        let t: Tokens = parse_normalized(body.as_bytes()).unwrap();
        assert_eq!(t.email().as_deref(), Some("eu@exemplo.com"));
        let o = t.user_decryption_options.unwrap();
        assert_eq!(o.has_master_password, Some(false));
        assert!(o.key_connector.is_some() && o.trusted_device.is_none());
    }
}
