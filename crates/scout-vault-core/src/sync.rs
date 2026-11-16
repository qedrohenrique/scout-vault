//! Response of `GET /api/sync` as it comes from the server: **encrypted**.
//!
//! These types are safe to cache on disk, since every sensitive field is
//! still an `EncString` in text form. The JSON keys arrive already
//! normalized to camelCase (see `api::parse_normalized`).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncResponse {
    pub profile: Profile,
    #[serde(default)]
    pub folders: Vec<FolderRaw>,
    #[serde(default)]
    pub ciphers: Vec<CipherRaw>,
    /// The organizations' collections (name encrypted with the organization key).
    #[serde(default)]
    pub collections: Vec<CollectionRaw>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionRaw {
    pub id: String,
    pub organization_id: String,
    pub name: String,
    /// The person can only read (can't put items in it).
    #[serde(default)]
    pub read_only: bool,
}

impl SyncResponse {
    /// Reads the sync JSON (for example, from a disk cache), accepting PascalCase or camelCase.
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
    /// User key encrypted with the stretched master key.
    #[serde(default)]
    pub key: Option<String>,
    /// RSA private key (PKCS#8) encrypted with the user key.
    #[serde(default)]
    pub private_key: Option<String>,
    /// New format of the account keys (v2 accounts). Only used as a fallback
    /// when `private_key` is missing.
    #[serde(default)]
    pub account_keys: Option<AccountKeys>,
    #[serde(default)]
    pub organizations: Vec<OrganizationRaw>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrganizationRaw {
    pub id: String,
    /// Name in plain text.
    #[serde(default)]
    pub name: Option<String>,
    /// Organization key encrypted with the user's RSA public key (`4.…`).
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
    /// 1 = login, 2 = secure note, 3 = card, 4 = identity, 5 = SSH key.
    #[serde(rename = "type")]
    pub kind: u8,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    /// The item's own key, encrypted with the user key or the organization key.
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub favorite: bool,
    /// 1 = ask for the master password before showing.
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
    /// Fields Scout Vault doesn't use (passkeys, password history, attachments…),
    /// kept so they go back to the server untouched when the item is saved.
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
    /// Fields Scout Vault doesn't use (passkeys, password history, attachments…),
    /// kept so they go back to the server untouched when the item is saved.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UriRaw {
    #[serde(default)]
    pub uri: Option<String>,
    /// Match rule (domain, host, exact...). Plain text.
    #[serde(default, rename = "match")]
    pub match_type: Option<u8>,
    /// Fields Scout Vault doesn't use (passkeys, password history, attachments…),
    /// kept so they go back to the server untouched when the item is saved.
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
    /// Fields Scout Vault doesn't use (passkeys, password history, attachments…),
    /// kept so they go back to the server untouched when the item is saved.
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
    /// Fields Scout Vault doesn't use (passkeys, password history, attachments…),
    /// kept so they go back to the server untouched when the item is saved.
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
    /// The official server calls it `keyFingerprint`; old servers, `fingerprint`.
    #[serde(default, alias = "fingerprint")]
    pub key_fingerprint: Option<String>,
    /// Fields Scout Vault doesn't use (passkeys, password history, attachments…),
    /// kept so they go back to the server untouched when the item is saved.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldRaw {
    /// 0 = text, 1 = hidden, 2 = boolean, 3 = linked.
    #[serde(default, rename = "type")]
    pub kind: Option<u8>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub value: Option<String>,
    /// Fields Scout Vault doesn't use (passkeys, password history, attachments…),
    /// kept so they go back to the server untouched when the item is saved.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}
