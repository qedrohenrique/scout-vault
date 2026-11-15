//! Cliente HTTP para servidores compatíveis com Bitwarden (oficial e Vaultwarden).
//!
//! Só transporta dados **cifrados**: nada aqui decifra. Fluxo típico:
//!
//! ```no_run
//! # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
//! use scout_vault_core::api::{Client, DeviceInfo, ServerUrls};
//! use scout_vault_core::crypto::MasterKey;
//! use scout_vault_core::vault::{Keys, Vault};
//!
//! let (email, password) = ("eu@exemplo.com", "senha mestra");
//! let client = Client::new(ServerUrls::self_hosted("http://localhost:8000"), DeviceInfo::new("Scout Vault"))?;
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
/// Cliente usado no login por SSO: o servidor só aceita redirecionar para
/// `http://localhost:8065..8070` nos clientes `cli`/`desktop`, e o `desktop`
/// prefere `bitwarden://`, que pertence ao app oficial.
pub const SSO_CLIENT_ID: &str = "cli";
/// Versão anunciada ao servidor. O Vaultwarden usa isso para decidir, por exemplo,
/// se envia chaves SSH no sync. O servidor oficial **recusa versões antigas**
/// ("Please update your app to continue using Bitwarden"): em nov/2026 o
/// `2024.12.0` já era recusado e o `2025.1.0` em diante aceito. Manter perto
/// da versão atual do app desktop oficial.
const CLIENT_VERSION: &str = "2026.10.0";

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
    /// Cofre web (usado para o login por SSO).
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

    /// Servidor próprio (Vaultwarden ou Bitwarden self-hosted): `<base>/api` e `<base>/identity`.
    pub fn self_hosted(base: &str) -> Self {
        let base = base.trim().trim_end_matches('/');
        Self {
            api: format!("{base}/api"),
            identity: format!("{base}/identity"),
            vault: base.to_owned(),
        }
    }
}

/// Código devolvido pelo navegador no fim do login por SSO.
#[derive(Debug, Clone)]
pub struct SsoCode {
    pub code: String,
    /// Verificador PKCE (o `code_challenge` foi o SHA-256 dele).
    pub verifier: String,
    pub redirect_uri: String,
}

/// Como a conta abre o cofre depois do login (`UserDecryptionOptions`).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct UserDecryptionOptions {
    #[serde(rename = "hasMasterPassword", default)]
    pub has_master_password: Option<bool>,
    #[serde(rename = "keyConnectorOption", default)]
    pub key_connector: Option<Value>,
    #[serde(rename = "trustedDeviceOption", default)]
    pub trusted_device: Option<Value>,
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

/// O que o servidor devolve quando pede o segundo fator.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TwoFactorChallenge {
    pub providers: Vec<TwoFactorProvider>,
    /// Token de sessão que o servidor oficial exige para reenviar o código por
    /// e-mail (`ssoEmail2faSessionToken`). O Vaultwarden não manda.
    pub email_session_token: Option<String>,
    /// Dados de cada método (`TwoFactorProviders2`), pelo código do método:
    /// opções da chave de segurança (7), `AuthUrl` do Duo (2/6), e-mail mascarado (1)…
    pub provider_data: Vec<(TwoFactorProvider, Value)>,
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
    TwoFactorRequired(TwoFactorChallenge),
    #[error("código do segundo fator inválido: {0}")]
    InvalidTwoFactorCode(String),
    #[error("o servidor pediu verificação de novo dispositivo (código enviado por e-mail)")]
    NewDeviceVerificationRequired,
    #[error("código de verificação de novo dispositivo inválido")]
    InvalidNewDeviceCode,
    #[error("o servidor exige um captcha, que só os apps oficiais resolvem")]
    CaptchaRequired,
    /// Muitas tentativas seguidas (HTTP 429); o servidor libera depois de um tempo.
    #[error("muitas tentativas; espere um pouco")]
    RateLimited,
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
    /// User key cifrada com a master key. O sync também a traz em `profile.key`;
    /// esta serve de reserva caso o sync deixe de trazê-la.
    #[serde(default)]
    pub key: Option<String>,
    #[serde(rename = "userDecryptionOptions", default)]
    pub user_decryption_options: Option<UserDecryptionOptions>,
}

impl Tokens {
    /// E-mail da conta, lido do JWT do access token (sem verificar a assinatura:
    /// só serve para a KDF; o servidor é quem valida o token).
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
    /// Alguns servidores giram o refresh token; se vier, substitua o antigo.
    #[serde(default)]
    pub refresh_token: Option<String>,
}

/// Resposta do registro de um anexo novo.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentUpload {
    pub attachment_id: String,
    #[serde(default)]
    pub url: Option<String>,
    /// 0 = direto no servidor (Vaultwarden, self-hosted); 1 = Azure (nuvem).
    #[serde(default)]
    pub file_upload_type: u8,
}

pub struct Client {
    http: reqwest::Client,
    urls: ServerUrls,
    device: DeviceInfo,
    /// `client_id` do OAuth: o mesmo do login precisa ser usado no refresh.
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

    /// Cliente para o login por SSO (e para renovar os tokens dele).
    pub fn for_sso(urls: ServerUrls, device: DeviceInfo) -> Result<Self, ApiError> {
        let mut client = Self::new(urls, device)?;
        client.client_id = SSO_CLIENT_ID;
        Ok(client)
    }

    pub fn device(&self) -> &DeviceInfo {
        &self.device
    }

    /// Este cliente usa o `client_id` do login por SSO.
    pub fn is_sso(&self) -> bool {
        self.client_id == SSO_CLIENT_ID
    }

    pub fn urls(&self) -> &ServerUrls {
        &self.urls
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

    /// `POST /identity/connect/token` com `grant_type=authorization_code` (fim do SSO).
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

    /// Pede ao servidor para enviar o código de 2FA por e-mail.
    ///
    /// O servidor oficial autentica o pedido pelo `session_token` que veio em
    /// [`TwoFactorChallenge::email_session_token`]; o Vaultwarden, pelo hash da
    /// senha. Mandamos os dois.
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

    /// Troca o refresh token por um access token novo.
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

    /// `POST /api/ciphers` — cria um item pessoal. Devolve o item como ficou no servidor.
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

    /// `POST /api/ciphers/create` — cria um item **numa organização**, já nas coleções.
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

    /// Troca as coleções de um item de organização. Servidores novos usam
    /// `collections_v2`; os antigos, `collections`.
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

    /// `PUT /api/ciphers/{id}` — grava a edição de um item.
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

    /// `PUT /api/ciphers/{id}/partial` — muda só a pasta (e o favorito) do item,
    /// sem recifrar nada. Vale também para itens de organização (a pasta é pessoal).
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

    /// `POST /api/folders` — cria uma pasta. `name` já vem cifrado com a user key.
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

    /// `PUT /api/folders/{id}` — renomeia (nome cifrado).
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

    /// `DELETE /api/folders/{id}` — apaga a pasta; os itens dela ficam sem pasta.
    pub async fn delete_folder(&self, access_token: &str, id: &str) -> Result<(), ApiError> {
        let res = self
            .http
            .delete(format!("{}/folders/{id}", self.urls.api))
            .bearer_auth(access_token)
            .send()
            .await?;
        ensure_success(res).await.map(|_| ())
    }

    /// `PUT /api/ciphers/{id}/delete` — manda para a lixeira (dá para restaurar).
    pub async fn trash_cipher(&self, access_token: &str, id: &str) -> Result<(), ApiError> {
        let res = self
            .http
            .put(format!("{}/ciphers/{id}/delete", self.urls.api))
            .bearer_auth(access_token)
            .send()
            .await?;
        ensure_success(res).await.map(|_| ())
    }

    /// `PUT /api/ciphers/{id}/restore` — tira da lixeira.
    pub async fn restore_cipher(&self, access_token: &str, id: &str) -> Result<(), ApiError> {
        let res = self
            .http
            .put(format!("{}/ciphers/{id}/restore", self.urls.api))
            .bearer_auth(access_token)
            .send()
            .await?;
        ensure_success(res).await.map(|_| ())
    }

    /// `DELETE /api/ciphers/{id}` — apaga **de vez** (só itens já na lixeira).
    pub async fn delete_cipher(&self, access_token: &str, id: &str) -> Result<(), ApiError> {
        let res = self
            .http
            .delete(format!("{}/ciphers/{id}", self.urls.api))
            .bearer_auth(access_token)
            .send()
            .await?;
        ensure_success(res).await.map(|_| ())
    }

    /// Endereço de download de um anexo (`GET /api/ciphers/{id}/attachment/{att}`).
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

    /// Endereço relativo ao servidor vira absoluto.
    fn absolute(&self, url: &str) -> String {
        if url.starts_with("http://") || url.starts_with("https://") {
            return url.to_owned();
        }
        let base = self.urls.api.trim_end_matches('/').trim_end_matches("/api");
        format!("{base}/{}", url.trim_start_matches('/'))
    }

    /// Baixa o arquivo cifrado (o endereço já vem assinado; sem token).
    pub async fn download(&self, url: &str) -> Result<Vec<u8>, ApiError> {
        let res = self.http.get(url).send().await?;
        let res = ensure_success(res).await?;
        Ok(res.bytes().await?.to_vec())
    }

    /// Registra um anexo novo e devolve onde enviar o arquivo.
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

    /// Envia o arquivo cifrado para o anexo registrado.
    pub async fn upload_attachment(
        &self,
        access_token: &str,
        cipher_id: &str,
        upload: &AttachmentUpload,
        file_name: &str,
        data: Vec<u8>,
    ) -> Result<(), ApiError> {
        let res = if upload.file_upload_type == 1 {
            // Bitwarden na nuvem: direto no armazenamento (Azure), com URL assinada.
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
    // Mensagens do servidor oficial (DeviceValidator/BaseRequestValidator) e do Vaultwarden.
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
                // TwoFactorProviders2 vem com chaves numéricas em texto.
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
        // JWT de exemplo: payload {"email":"eu@exemplo.com"} (assinatura irrelevante aqui).
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
