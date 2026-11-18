//! SSH keys: generate one, or import an existing private key and work out its
//! public key and fingerprint.
//!
//! The vault keeps the private key as an unencrypted OpenSSH key (`-----BEGIN
//! OPENSSH PRIVATE KEY-----`), the way the Bitwarden apps do: the vault already
//! encrypts it. Imported keys with a passphrase are decrypted once, on import.
//!
//! Accepted on import: OpenSSH (Ed25519, RSA, ECDSA P-256/P-384, with or without a
//! passphrase), PuTTY `.ppk` and RSA keys in PEM (PKCS#1 / PKCS#8).

use ssh_key::getrandom::SysRng;
use ssh_key::private::{KeypairData, RsaKeypair};
use ssh_key::rand_core::UnwrapErr;
use ssh_key::{Algorithm, HashAlg, LineEnding, PrivateKey};
use zeroize::Zeroizing;

/// A key ready for the vault.
pub struct KeyMaterial {
    /// OpenSSH private key, unencrypted, with `\n` line endings.
    pub private_key: Zeroizing<String>,
    /// `ssh-ed25519 AAAA…` (the line for `authorized_keys`).
    pub public_key: String,
    /// `SHA256:…`
    pub fingerprint: String,
    /// The key type, as OpenSSH names it (`ssh-ed25519`, `ssh-rsa`…).
    pub algorithm: String,
}

/// Why a private key can't be imported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ImportError {
    #[error("the key is protected by a passphrase")]
    NeedsPassphrase,
    #[error("wrong passphrase")]
    WrongPassphrase,
    #[error("unsupported key type")]
    Unsupported,
    #[error("not a private key")]
    Invalid,
}

impl KeyMaterial {
    fn from_key(key: &PrivateKey) -> Result<Self, ImportError> {
        // Comments are not kept: the item's name already says what the key is.
        let mut key = key.clone();
        key.set_comment("");
        let private_key = key
            .to_openssh(LineEnding::LF)
            .map_err(|_| ImportError::Unsupported)?;
        let public_key = key
            .public_key()
            .to_openssh()
            .map_err(|_| ImportError::Unsupported)?;
        Ok(Self {
            private_key: Zeroizing::new(private_key.to_string()),
            public_key: public_key.trim().to_owned(),
            fingerprint: key.fingerprint(HashAlg::Sha256).to_string(),
            algorithm: key.algorithm().as_str().to_owned(),
        })
    }
}

/// A new Ed25519 key (what the Bitwarden apps generate too).
pub fn generate() -> KeyMaterial {
    let key = PrivateKey::random(&mut UnwrapErr(SysRng), Algorithm::Ed25519)
        .expect("Ed25519 keys can always be generated");
    KeyMaterial::from_key(&key).expect("a fresh Ed25519 key can be encoded")
}

/// Reads a private key pasted or loaded from a file. `passphrase` is only needed
/// for protected keys ([`ImportError::NeedsPassphrase`] says so).
pub fn import(text: &str, passphrase: Option<&str>) -> Result<KeyMaterial, ImportError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(ImportError::Invalid);
    }
    let key = if text.starts_with("PuTTY-User-Key-File-") {
        import_ppk(text, passphrase)?
    } else if text.contains("-----BEGIN OPENSSH PRIVATE KEY-----") {
        import_openssh(text, passphrase)?
    } else if text.contains("-----BEGIN RSA PRIVATE KEY-----")
        || text.contains("-----BEGIN PRIVATE KEY-----")
    {
        import_rsa_pem(text)?
    } else if text.contains("-----BEGIN ENCRYPTED PRIVATE KEY-----")
        || text.contains("Proc-Type: 4,ENCRYPTED")
    {
        // Old-style encrypted PEM: convert it with `ssh-keygen -p` first.
        return Err(ImportError::Unsupported);
    } else {
        return Err(ImportError::Invalid);
    };
    if matches!(key.key_data(), KeypairData::Encrypted(_)) {
        return Err(ImportError::NeedsPassphrase);
    }
    KeyMaterial::from_key(&key)
}

fn import_openssh(text: &str, passphrase: Option<&str>) -> Result<PrivateKey, ImportError> {
    let key = PrivateKey::from_openssh(text).map_err(|_| ImportError::Invalid)?;
    if !key.is_encrypted() {
        return Ok(key);
    }
    let passphrase = passphrase
        .filter(|p| !p.is_empty())
        .ok_or(ImportError::NeedsPassphrase)?;
    key.decrypt(passphrase).map_err(|e| match e {
        ssh_key::Error::Crypto => ImportError::WrongPassphrase,
        _ => ImportError::Unsupported,
    })
}

fn import_ppk(text: &str, passphrase: Option<&str>) -> Result<PrivateKey, ImportError> {
    let encrypted = text
        .lines()
        .any(|l| l.trim().starts_with("Encryption:") && !l.trim().ends_with("none"));
    let passphrase = passphrase.filter(|p| !p.is_empty());
    if encrypted && passphrase.is_none() {
        return Err(ImportError::NeedsPassphrase);
    }
    PrivateKey::from_ppk(text, passphrase.map(str::to_owned)).map_err(|_| {
        if encrypted {
            ImportError::WrongPassphrase
        } else {
            ImportError::Invalid
        }
    })
}

fn import_rsa_pem(text: &str) -> Result<PrivateKey, ImportError> {
    use rsa::pkcs1::DecodeRsaPrivateKey;
    use rsa::pkcs8::DecodePrivateKey;
    let rsa = rsa::PrivateKey::from_pkcs1_pem(text)
        .or_else(|_| rsa::PrivateKey::from_pkcs8_pem(text))
        .map_err(|_| ImportError::Unsupported)?;
    let keypair = RsaKeypair::try_from(rsa).map_err(|_| ImportError::Unsupported)?;
    PrivateKey::new(KeypairData::from(keypair), "").map_err(|_| ImportError::Unsupported)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_keys_import_back_the_same() {
        let key = generate();
        assert!(key
            .private_key
            .starts_with("-----BEGIN OPENSSH PRIVATE KEY-----"));
        assert!(key.public_key.starts_with("ssh-ed25519 AAAA"));
        assert!(key.fingerprint.starts_with("SHA256:"));
        assert_eq!(key.algorithm, "ssh-ed25519");
        let again = import(&key.private_key, None).unwrap();
        assert_eq!(again.public_key, key.public_key);
        assert_eq!(again.fingerprint, key.fingerprint);
        assert_eq!(*again.private_key, *key.private_key);
        // Two keys are never the same.
        assert_ne!(generate().public_key, key.public_key);
    }

    #[test]
    fn protected_keys_ask_for_the_passphrase() {
        let plain = PrivateKey::random(&mut UnwrapErr(SysRng), Algorithm::Ed25519).unwrap();
        let locked = plain
            .encrypt(&mut UnwrapErr(SysRng), "correct horse")
            .unwrap();
        let text = locked.to_openssh(LineEnding::LF).unwrap();

        assert_eq!(
            import(&text, None).err(),
            Some(ImportError::NeedsPassphrase)
        );
        assert_eq!(
            import(&text, Some("")).err(),
            Some(ImportError::NeedsPassphrase)
        );
        assert_eq!(
            import(&text, Some("wrong")).err(),
            Some(ImportError::WrongPassphrase)
        );
        let key = import(&text, Some("correct horse")).unwrap();
        // Stored without the passphrase: the vault already encrypts it.
        assert!(import(&key.private_key, None).is_ok());
        assert_eq!(
            key.fingerprint,
            plain.fingerprint(HashAlg::Sha256).to_string()
        );
    }

    #[test]
    fn comments_are_dropped() {
        let mut plain = PrivateKey::random(&mut UnwrapErr(SysRng), Algorithm::Ed25519).unwrap();
        plain.set_comment("me@laptop");
        let text = plain.to_openssh(LineEnding::CRLF).unwrap();
        let key = import(&text, None).unwrap();
        assert!(!key.public_key.contains("me@laptop"));
        assert!(!key.private_key.contains('\r'));
    }

    #[test]
    fn garbage_is_refused() {
        assert_eq!(import("", None).err(), Some(ImportError::Invalid));
        assert_eq!(import("hello", None).err(), Some(ImportError::Invalid));
        assert_eq!(
            import(
                "-----BEGIN OPENSSH PRIVATE KEY-----\nAAAA\n-----END OPENSSH PRIVATE KEY-----",
                None
            )
            .err(),
            Some(ImportError::Invalid)
        );
        assert_eq!(
            import(
                "-----BEGIN ENCRYPTED PRIVATE KEY-----\nAAAA\n-----END ENCRYPTED PRIVATE KEY-----",
                None
            )
            .err(),
            Some(ImportError::Unsupported)
        );
    }

    #[test]
    fn rsa_pem_keys_are_converted() {
        use rsa::pkcs1::EncodeRsaPrivateKey;
        use rsa::pkcs8::EncodePrivateKey;
        let rsa = rsa::PrivateKey::new(&mut UnwrapErr(SysRng), 1024).unwrap();
        let pkcs1 = rsa.to_pkcs1_pem(rsa::pkcs8::LineEnding::LF).unwrap();
        let pkcs8 = rsa.to_pkcs8_pem(rsa::pkcs8::LineEnding::LF).unwrap();
        let a = import(&pkcs1, None).unwrap();
        let b = import(&pkcs8, None).unwrap();
        assert!(a.public_key.starts_with("ssh-rsa AAAA"));
        assert_eq!(a.fingerprint, b.fingerprint);
        assert!(a
            .private_key
            .starts_with("-----BEGIN OPENSSH PRIVATE KEY-----"));
    }
}
