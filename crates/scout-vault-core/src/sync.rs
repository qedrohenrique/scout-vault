//! Resposta de `GET /api/sync` como chega do servidor: **cifrada**.
//!
//! Esses tipos são seguros para cache em disco (fase 3), pois todo campo
//! sensível continua sendo um `EncString` em texto. As chaves do JSON já
//! chegam normalizadas para camelCase (ver `api::parse_normalized`).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncResponse {
    pub profile: Profile,
    #[serde(default)]
    pub folders: Vec<FolderRaw>,
    #[serde(default)]
    pub ciphers: Vec<CipherRaw>,
    /// Coleções das organizações (nome cifrado com a chave da organização).
    #[serde(default)]
    pub collections: Vec<CollectionRaw>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionRaw {
    pub id: String,
    pub organization_id: String,
    pub name: String,
    /// A pessoa só pode ler (não pode pôr itens nela).
    #[serde(default)]
    pub read_only: bool,
}

impl SyncResponse {
    /// Lê o JSON do sync (por exemplo, de um cache em disco), aceitando PascalCase ou camelCase.
    pub fn from_json(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        crate::json::from_slice_normalized(bytes)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountKeys {
    #[serde(default)]
    pub public_key_encryption_key_pair: Option<KeyPair>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyPair {
    #[serde(default)]
    pub wrapped_private_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: String,
    #[serde(default)]
    pub email: Option<String>,
    /// User key cifrada com a master key esticada.
    #[serde(default)]
    pub key: Option<String>,
    /// Chave privada RSA (PKCS#8) cifrada com a user key.
    #[serde(default)]
    pub private_key: Option<String>,
    /// Formato novo das chaves da conta (contas v2). Só usado como reserva
    /// quando `private_key` não vem.
    #[serde(default)]
    pub account_keys: Option<AccountKeys>,
    #[serde(default)]
    pub organizations: Vec<OrganizationRaw>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrganizationRaw {
    pub id: String,
    /// Nome em texto claro.
    #[serde(default)]
    pub name: Option<String>,
    /// Chave da organização cifrada com a chave pública RSA do usuário (`4.…`).
    #[serde(default)]
    pub key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderRaw {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CipherRaw {
    pub id: String,
    #[serde(default)]
    pub organization_id: Option<String>,
    #[serde(default)]
    pub folder_id: Option<String>,
    /// 1 = login, 2 = nota segura, 3 = cartão, 4 = identidade, 5 = chave SSH.
    #[serde(rename = "type")]
    pub kind: u8,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    /// Chave própria do item, cifrada com a user key ou a chave da organização.
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub favorite: bool,
    /// 1 = pedir senha mestra antes de mostrar.
    #[serde(default)]
    pub reprompt: Option<u8>,
    #[serde(default)]
    pub deleted_date: Option<String>,
    #[serde(default)]
    pub revision_date: Option<String>,
    #[serde(default)]
    pub login: Option<LoginRaw>,
    #[serde(default)]
    pub card: Option<CardRaw>,
    #[serde(default)]
    pub identity: Option<IdentityRaw>,
    #[serde(default)]
    pub ssh_key: Option<SshKeyRaw>,
    #[serde(default)]
    pub fields: Option<Vec<FieldRaw>>,
    /// Campos que o Scout Vault não usa (passkeys, histórico de senhas, anexos…),
    /// guardados para voltarem intactos ao servidor quando o item é salvo.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginRaw {
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub totp: Option<String>,
    #[serde(default)]
    pub uris: Option<Vec<UriRaw>>,
    /// Campos que o Scout Vault não usa (passkeys, histórico de senhas, anexos…),
    /// guardados para voltarem intactos ao servidor quando o item é salvo.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UriRaw {
    #[serde(default)]
    pub uri: Option<String>,
    /// Regra de correspondência (domínio, host, exata...). Texto claro.
    #[serde(default, rename = "match")]
    pub match_type: Option<u8>,
    /// Campos que o Scout Vault não usa (passkeys, histórico de senhas, anexos…),
    /// guardados para voltarem intactos ao servidor quando o item é salvo.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CardRaw {
    #[serde(default)]
    pub cardholder_name: Option<String>,
    #[serde(default)]
    pub brand: Option<String>,
    #[serde(default)]
    pub number: Option<String>,
    #[serde(default)]
    pub exp_month: Option<String>,
    #[serde(default)]
    pub exp_year: Option<String>,
    #[serde(default)]
    pub code: Option<String>,
    /// Campos que o Scout Vault não usa (passkeys, histórico de senhas, anexos…),
    /// guardados para voltarem intactos ao servidor quando o item é salvo.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct IdentityRaw {
    pub title: Option<String>,
    pub first_name: Option<String>,
    pub middle_name: Option<String>,
    pub last_name: Option<String>,
    pub address1: Option<String>,
    pub address2: Option<String>,
    pub address3: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub postal_code: Option<String>,
    pub country: Option<String>,
    pub company: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub ssn: Option<String>,
    pub username: Option<String>,
    pub passport_number: Option<String>,
    pub license_number: Option<String>,
    /// Campos que o Scout Vault não usa (passkeys, histórico de senhas, anexos…),
    /// guardados para voltarem intactos ao servidor quando o item é salvo.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshKeyRaw {
    #[serde(default)]
    pub private_key: Option<String>,
    #[serde(default)]
    pub public_key: Option<String>,
    /// O servidor oficial chama de `keyFingerprint`; servidores antigos, de `fingerprint`.
    #[serde(default, alias = "fingerprint")]
    pub key_fingerprint: Option<String>,
    /// Campos que o Scout Vault não usa (passkeys, histórico de senhas, anexos…),
    /// guardados para voltarem intactos ao servidor quando o item é salvo.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldRaw {
    /// 0 = texto, 1 = oculto, 2 = booleano, 3 = vinculado.
    #[serde(default, rename = "type")]
    pub kind: Option<u8>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub value: Option<String>,
    /// Campos que o Scout Vault não usa (passkeys, histórico de senhas, anexos…),
    /// guardados para voltarem intactos ao servidor quando o item é salvo.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}
