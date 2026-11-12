//! Cliente HTTP para servidores compatíveis com Bitwarden (oficial e Vaultwarden).
//!
//! Só transporta dados **cifrados**: nada aqui decifra. Fluxo típico:
//!
//! ```no_run
//! # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
//! use cofre_core::api::{Client, DeviceInfo, ServerUrls};
//! use cofre_core::crypto::MasterKey;
//! use cofre_core::vault::{Keys, Vault};
//!
//! let (email, password) = ("eu@exemplo.com", "senha mestra");
//! let client = Client::new(ServerUrls::self_hosted("http://localhost:8000"), DeviceInfo::new("Cofre"))?;
//! let kdf = client.prelogin(email).await?;
//! let master_key = MasterKey::derive(password, email, kdf)?; // lento: fora da thread de UI
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

/// Nome de cliente aceito pelo servidor (enum `BitwardenClient` do servidor oficial).
const CLIENT_ID: &str = "desktop";
/// Versão anunciada ao servidor. O Vaultwarden usa isso para decidir, por exemplo,
/// se envia chaves SSH no sync.
const CLIENT_VERSION: &str = "2024.12.0";

/// `DeviceType` do protocolo: 6 = Windows, 7 = macOS, 8 = Linux (desktop).
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
}

impl ServerUrls {
    pub fn bitwarden_us() -> Self {
        Self {
            api: "https://api.bitwarden.com".into(),
            identity: "https://identity.bitwarden.com".into(),
        }
    }

    pub fn bitwarden_eu() -> Self {
        Self {
            api: "https://api.bitwarden.eu".into(),
            identity: "https://identity.bitwarden.eu".into(),
        }
    }

    /// Servidor próprio (Vaultwarden ou Bitwarden self-hosted): `<base>/api` e `<base>/identity`.
    pub fn self_hosted(base: &str) -> Self {
        let base = base.trim().trim_end_matches('/');
        Self {
            api: format!("{base}/api"),
            identity: format!("{base}/identity"),
        }
    }
}

/// Identidade deste dispositivo perante o servidor. O `identifier` deve ser
/// **persistido** e reutilizado — um novo a cada login dispara alertas de
/// "novo dispositivo" e verificação por e-mail.
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

/// Segundo fator enviado junto com o login.
#[derive(Debug, Clone)]
pub struct SecondFactor {
    pub provider: TwoFactorProvider,
    pub token: String,
    /// Pede ao servidor um token para pular o 2FA neste dispositivo nas próximas vezes.
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
    TwoFactorRequired(Vec<TwoFactorProvider>),
    #[error("o servidor pediu verificação de novo dispositivo (código enviado por e-mail)")]
    NewDeviceVerificationRequired,
    #[error("sessão expirada ou token inválido")]
    Unauthorized,
    #[error("servidor respondeu {status}: {body}")]
    Status { status: u16, body: String },
}

/// Resultado de um login bem-sucedido.
#[derive(Debug, Clone, Deserialize)]
pub struct Tokens {
    pub access_token: String,
    /// Validade do access token, em segundos.
    #[serde(default)]
    pub expires_in: u64,
    pub refresh_token: String,
    /// Presente quando o login usou `SecondFactor { remember: true, .. }`.
    #[serde(rename = "twoFactorToken", default)]
    pub two_factor_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RefreshedTokens {
    pub access_token: String,
    #[serde(default)]
    pub expires_in: u64,
    /// Alguns servidores giram o refresh token; se vier, substitua o antigo.
    #[serde(default)]
    pub refresh_token: Option<String>,
}

pub struct Client {
    http: reqwest::Client,
    urls: ServerUrls,
    device: DeviceInfo,
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
            .user_agent(concat!("cofre/", env!("CARGO_PKG_VERSION")))
            .default_headers(headers)
            .build()?;
        Ok(Self { http, urls, device })
    }

    pub fn device(&self) -> &DeviceInfo {
        &self.device
    }

    /// `POST /identity/accounts/prelogin` — descobre a KDF da conta.
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

    /// `POST /identity/connect/token` com `grant_type=password`.
    ///
    /// `server_hash` vem de [`MasterKey::server_hash`](crate::crypto::MasterKey::server_hash).
    /// Se o servidor responder [`ApiError::TwoFactorRequired`], chame de novo com `second_factor`;
    /// se responder [`ApiError::NewDeviceVerificationRequired`], com `new_device_otp`.
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
            ("client_id", CLIENT_ID.into()),
            ("deviceType", DEVICE_TYPE.to_string()),
            ("deviceIdentifier", self.device.identifier.clone()),
            ("deviceName", self.device.name.clone()),
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

    /// Pede ao servidor para enviar o código de 2FA por e-mail.
    pub async fn send_email_login_code(
        &self,
        email: &str,
        server_hash: &str,
    ) -> Result<(), ApiError> {
        let res = self
            .http
            .post(format!("{}/two-factor/send-email-login", self.urls.api))
            .json(&serde_json::json!({
                "email": email,
                "masterPasswordHash": server_hash,
                "deviceIdentifier": self.device.identifier,
            }))
            .send()
            .await?;
        ensure_success(res).await.map(|_| ())
    }

    /// Troca o refresh token por um access token novo.
    pub async fn refresh(&self, refresh_token: &str) -> Result<RefreshedTokens, ApiError> {
        let res = self
            .http
            .post(format!("{}/connect/token", self.urls.identity))
            .form(&[
                ("grant_type", "refresh_token"),
                ("client_id", CLIENT_ID),
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

    /// `GET /api/sync` — cofre inteiro, ainda cifrado.
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
    }

    let Ok(parsed) = parse_normalized::<LoginErrorBody>(body.as_bytes()) else {
        return ApiError::Status {
            status,
            body: body.to_owned(),
        };
    };

    if let Some(providers) = parsed.two_factor_providers {
        // O servidor manda os códigos como números ou como strings ("0").
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
        return ApiError::TwoFactorRequired(providers);
    }

    let message = parsed
        .error_model
        .and_then(|m| m.message)
        .or(parsed.error_description)
        .unwrap_or_else(|| parsed.error.clone());

    if message.to_lowercase().contains("device verification") {
        return ApiError::NewDeviceVerificationRequired;
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
            }
        );
    }

    #[test]
    fn two_factor_as_strings_pascal_case() {
        let body = r#"{"error":"invalid_grant","error_description":"Two factor required.",
                       "TwoFactorProviders":["0","1"],"TwoFactorProviders2":{"0":null,"1":{"Email":"e***@x.com"}}}"#;
        match classify_login_error(400, body) {
            ApiError::TwoFactorRequired(p) => {
                assert_eq!(
                    p,
                    [TwoFactorProvider::Authenticator, TwoFactorProvider::Email]
                )
            }
            other => panic!("esperava 2FA, veio {other:?}"),
        }
    }

    #[test]
    fn two_factor_as_numbers() {
        let body = r#"{"error":"invalid_grant","twoFactorProviders":[0,7]}"#;
        assert!(matches!(
            classify_login_error(400, body),
            ApiError::TwoFactorRequired(p) if p == [TwoFactorProvider::Authenticator, TwoFactorProvider::WebAuthn]
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
    }
}
