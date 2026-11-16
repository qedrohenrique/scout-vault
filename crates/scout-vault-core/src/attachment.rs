//! Attachments: encrypted files stored with the item.
//!
//! Each attachment has **its own key** (64 bytes, AES-CBC + HMAC), stored
//! encrypted with the item's (or the owner's) key. The file on the server is an
//! "EncArrayBuffer": `[type][iv 16][mac 32][data]` for type 2, or
//! `[7][COSE_Encrypt0]` on v2 accounts. The file name is encrypted too.

use serde::Deserialize;
use zeroize::Zeroizing;

use crate::crypto::{CryptoError, SymmetricKey};
use crate::enc_string::EncString;
use crate::sync::CipherRaw;
use crate::vault::{Keys, VaultError};

/// The official clients' size limit for an attachment.
pub const MAX_SIZE: usize = 500 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentRaw {
    pub id: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub file_name: Option<String>,
    /// Attachment key, encrypted with the item/owner key.
    #[serde(default)]
    pub key: Option<String>,
    /// The server sends it as text ("1024") or as a number.
    #[serde(default)]
    pub size: Option<serde_json::Value>,
    #[serde(default)]
    pub size_name: Option<String>,
}

impl AttachmentRaw {
    pub fn size_bytes(&self) -> Option<u64> {
        match self.size.as_ref()? {
            serde_json::Value::Number(n) => n.as_u64(),
            serde_json::Value::String(s) => s.parse().ok(),
            _ => None,
        }
    }
}

/// The item's attachments, as they came in the sync (still encrypted).
pub fn raw_attachments(cipher: &CipherRaw) -> Vec<AttachmentRaw> {
    cipher
        .extra
        .get("attachments")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

/// The key used for the item's fields: the item's own or the owner's.
pub fn item_key(cipher: &CipherRaw, keys: &Keys) -> Result<SymmetricKey, VaultError> {
    let owner = keys.for_owner(cipher.organization_id.as_deref())?;
    match &cipher.key {
        Some(k) => Ok(SymmetricKey::from_bytes(&owner.decrypt(&k.parse()?)?)?),
        None => Ok(owner.clone()),
    }
}

/// An attachment's key (very old attachments don't have one: they use the item's).
pub fn attachment_key(
    cipher: &CipherRaw,
    attachment: &AttachmentRaw,
    keys: &Keys,
) -> Result<SymmetricKey, VaultError> {
    let item = item_key(cipher, keys)?;
    match &attachment.key {
        Some(k) => Ok(SymmetricKey::from_bytes(&item.decrypt(&k.parse()?)?)?),
        None => Ok(item),
    }
}

/// Decrypts the downloaded file.
pub fn decrypt_file(bytes: &[u8], key: &SymmetricKey) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
    let (&kind, rest) = bytes.split_first().ok_or(CryptoError::WrongEncType)?;
    let enc = match kind {
        2 if rest.len() > 48 => {
            let mut iv = [0u8; 16];
            let mut mac = [0u8; 32];
            iv.copy_from_slice(&rest[..16]);
            mac.copy_from_slice(&rest[16..48]);
            EncString::AesCbc256HmacSha256 {
                iv,
                mac,
                data: rest[48..].to_vec(),
            }
        }
        7 => EncString::Cose {
            data: rest.to_vec(),
        },
        _ => return Err(CryptoError::WrongEncType),
    };
    key.decrypt(&enc)
}

/// Encrypts the file in the format the server stores.
pub fn encrypt_file(plain: &[u8], key: &SymmetricKey) -> Result<Vec<u8>, CryptoError> {
    match key.encrypt(plain)? {
        EncString::AesCbc256HmacSha256 { iv, data, mac } => {
            let mut out = Vec::with_capacity(1 + 16 + 32 + data.len());
            out.push(2);
            out.extend_from_slice(&iv);
            out.extend_from_slice(&mac);
            out.extend_from_slice(&data);
            Ok(out)
        }
        EncString::Cose { data } => {
            let mut out = Vec::with_capacity(1 + data.len());
            out.push(7);
            out.extend_from_slice(&data);
            Ok(out)
        }
        _ => Err(CryptoError::WrongEncType),
    }
}

/// Everything that goes to the server for a new attachment.
pub struct NewAttachment {
    /// Attachment key, encrypted with the item key.
    pub key: String,
    /// File name, encrypted.
    pub file_name: String,
    /// Encrypted file.
    pub data: Vec<u8>,
}

/// Prepares a new attachment: generates the key, encrypts the name and the contents.
pub fn prepare(
    cipher: &CipherRaw,
    keys: &Keys,
    file_name: &str,
    contents: &[u8],
) -> Result<NewAttachment, VaultError> {
    if contents.len() > MAX_SIZE {
        return Err(VaultError::Unsupported(
            "o arquivo passa do limite de 500 MB".into(),
        ));
    }
    let item = item_key(cipher, keys)?;
    let attachment = SymmetricKey::generate()?;
    Ok(NewAttachment {
        key: item.encrypt(&attachment.raw())?.to_string(),
        file_name: item.encrypt_str(file_name)?.to_string(),
        data: encrypt_file(contents, &attachment)?,
    })
}

/// Human-readable size ("1,2 MB").
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["bytes", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} bytes")
    } else {
        format!("{value:.1} {}", UNITS[unit]).replace('.', ",")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_round_trip_and_tamper_detection() {
        let key = SymmetricKey::generate().unwrap();
        let plain = b"conteudo do PDF \x00\x01\x02".repeat(100);
        let enc = encrypt_file(&plain, &key).unwrap();
        assert_eq!(enc[0], 2);
        assert_eq!(&decrypt_file(&enc, &key).unwrap()[..], &plain[..]);
        let mut bad = enc.clone();
        *bad.last_mut().unwrap() ^= 1;
        assert!(
            decrypt_file(&bad, &key).is_err(),
            "MAC tem que pegar adulteração"
        );
        let other = SymmetricKey::generate().unwrap();
        assert!(decrypt_file(&enc, &other).is_err());
    }

    #[test]
    fn sizes_read_nicely() {
        assert_eq!(human_size(900), "900 bytes");
        assert_eq!(human_size(1536), "1,5 KB");
        assert_eq!(human_size(5 * 1024 * 1024), "5,0 MB");
    }
}
