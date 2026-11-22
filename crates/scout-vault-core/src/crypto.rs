//! Bitwarden protocol cryptography.
//!
//! Key flow:
//! ```text
//! password + email ──KDF──▶ MasterKey ──PBKDF2(1)──▶ authentication hash (sent to the server)
//!                             │
//!                         HKDF-Expand ("enc", "mac")
//!                             ▼
//!                    stretched key ──decrypts──▶ UserKey (64 B: enc ‖ mac, or v2 COSE_Key)
//!                                                  │
//!                         ┌────────────────────────┼─────────────────────┐
//!                         ▼                        ▼                     ▼
//!                   vault fields           RSA private key          item keys
//!                                                  │
//!                                       RSA-OAEP-SHA1 ▼
//!                                         organization keys
//! ```

use std::fmt;

use aes::cipher::block_padding::Pkcs7;
use aes::cipher::{BlockModeDecrypt, BlockModeEncrypt, KeyIvInit};
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::enc_string::EncString;
use crate::kdf::Kdf;
use crate::secret::Secret;

type HmacSha256 = Hmac<Sha256>;
type Aes256CbcEnc = cbc::Encryptor<aes::Aes256>;
type Aes256CbcDec = cbc::Decryptor<aes::Aes256>;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CryptoError {
    #[error("MAC inválido: chave errada ou dado corrompido")]
    InvalidMac,
    #[error("falha ao decifrar (padding inválido)")]
    Padding,
    #[error("tipo de EncString incompatível com a chave usada")]
    WrongEncType,
    #[error("chave simétrica deve ter 64 bytes, recebido {0}")]
    KeyLength(usize),
    #[error("parâmetros de KDF inválidos: {0}")]
    Kdf(String),
    #[error("chave privada RSA inválida")]
    RsaKey,
    #[error("falha ao decifrar com RSA-OAEP")]
    Rsa,
    #[error("texto decifrado não é UTF-8 válido")]
    Utf8,
    #[error("gerador de números aleatórios do sistema falhou")]
    Rng,
    #[error("mensagem COSE inválida: {0}")]
    Cose(&'static str),
}

/// Key derived from the master password. Never leaves the machine.
pub struct MasterKey(Zeroizing<[u8; 32]>);

impl MasterKey {
    /// Derives the master key. **Slow on purpose** (hundreds of ms to seconds):
    /// call it off the UI thread.
    pub fn derive(password: &str, email: &str, kdf: Kdf) -> Result<Self, CryptoError> {
        Self::derive_with_salt(password, &email.trim().to_lowercase(), kdf)
    }

    /// Like [`Self::derive`], with any salt taken as is (password-protected exports use a
    /// random one, case included).
    pub fn derive_with_salt(password: &str, salt: &str, kdf: Kdf) -> Result<Self, CryptoError> {
        // Also covers a KDF read from a saved session, not just the prelogin one.
        kdf.validate()
            .map_err(|e| CryptoError::Kdf(e.to_string()))?;
        let email = salt;
        let mut out = Zeroizing::new([0u8; 32]);

        match kdf {
            Kdf::Pbkdf2Sha256 { iterations } => {
                if iterations == 0 {
                    return Err(CryptoError::Kdf("PBKDF2 com 0 iterações".into()));
                }
                pbkdf2::pbkdf2_hmac::<Sha256>(
                    password.as_bytes(),
                    email.as_bytes(),
                    iterations,
                    out.as_mut_slice(),
                );
            }
            Kdf::Argon2id {
                iterations,
                memory_mib,
                parallelism,
            } => {
                let salt = Sha256::digest(email.as_bytes());
                let memory_kib = memory_mib
                    .checked_mul(1024)
                    .ok_or_else(|| CryptoError::Kdf("memória do Argon2 grande demais".into()))?;
                let params = argon2::Params::new(memory_kib, iterations, parallelism, Some(32))
                    .map_err(|e| CryptoError::Kdf(e.to_string()))?;
                argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params)
                    .hash_password_into(password.as_bytes(), &salt, out.as_mut_slice())
                    .map_err(|e| CryptoError::Kdf(e.to_string()))?;
            }
        }

        Ok(Self(out))
    }

    #[cfg(test)]
    pub(crate) fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(Zeroizing::new(bytes))
    }

    /// Hash sent to the server at login: PBKDF2-SHA256(master key, salt = password, 1 iteration).
    pub fn server_hash(&self, password: &str) -> String {
        let hash: Zeroizing<[u8; 32]> = Zeroizing::new(pbkdf2::pbkdf2_hmac_array::<Sha256, 32>(
            self.0.as_slice(),
            password.as_bytes(),
            1,
        ));
        STANDARD.encode(hash.as_slice())
    }

    /// Stretches the master key into an encryption key + MAC key via HKDF-Expand-SHA256.
    pub fn stretch(&self) -> SymmetricKey {
        let hk = hkdf::Hkdf::<Sha256>::from_prk(self.0.as_slice())
            .expect("PRK de 32 bytes tem o tamanho mínimo do SHA-256");
        let mut enc = [0u8; 32];
        let mut mac = [0u8; 32];
        hk.expand(b"enc", &mut enc)
            .expect("32 bytes está dentro do limite do HKDF");
        hk.expand(b"mac", &mut mac)
            .expect("32 bytes está dentro do limite do HKDF");
        let key = SymmetricKey::AesCbcHmac { enc, mac };
        enc.zeroize();
        mac.zeroize();
        key
    }
}

impl fmt::Debug for MasterKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MasterKey(***)")
    }
}

/// Symmetric key (user key, organization key or item key).
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub enum SymmetricKey {
    /// Legacy key: AES-256-CBC + HMAC-SHA256 (`enc ‖ mac`, 64 bytes).
    AesCbcHmac { enc: [u8; 32], mac: [u8; 32] },
    /// v2 key (COSE): 32 bytes for XChaCha20-Poly1305, XAES-256-GCM or AES-256-GCM.
    Cose {
        #[zeroize(skip)]
        alg: i64,
        key: [u8; 32],
        #[zeroize(skip)]
        kid: Option<Vec<u8>>,
    },
}

impl SymmetricKey {
    /// Interprets the bytes of a decrypted key: 64 bytes are `enc ‖ mac`;
    /// more than 64 are a padded `COSE_Key` (v2 accounts).
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CryptoError> {
        match bytes.len() {
            64 => {
                let mut enc = [0u8; 32];
                let mut mac = [0u8; 32];
                enc.copy_from_slice(&bytes[..32]);
                mac.copy_from_slice(&bytes[32..]);
                let key = SymmetricKey::AesCbcHmac { enc, mac };
                enc.zeroize();
                mac.zeroize();
                Ok(key)
            }
            n if n > 64 => {
                let parsed = crate::cose::parse_cose_key(crate::cose::unpad(bytes)?)?;
                let mut key = [0u8; 32];
                key.copy_from_slice(&parsed.key);
                let out = SymmetricKey::Cose {
                    alg: parsed.alg,
                    key,
                    kid: parsed.kid.clone(),
                };
                key.zeroize();
                Ok(out)
            }
            n => Err(CryptoError::KeyLength(n)),
        }
    }

    /// Raw key bytes, as the AEAD expects them.
    /// Key bytes in the format [`SymmetricKey::from_bytes`] reads back
    /// (64 bytes for legacy keys; padded `COSE_Key` for v2). For storing in the
    /// system credential store — use with care.
    pub fn to_bytes(&self) -> Zeroizing<Vec<u8>> {
        match self {
            SymmetricKey::AesCbcHmac { .. } => self.raw(),
            SymmetricKey::Cose { alg, key, kid } => Zeroizing::new(crate::cose::pad_key(
                &crate::cose::encode_cose_key_with_kid(*alg, key, kid.as_deref()),
            )),
        }
    }

    pub(crate) fn raw(&self) -> Zeroizing<Vec<u8>> {
        match self {
            SymmetricKey::AesCbcHmac { enc, mac } => {
                let mut v = Zeroizing::new(Vec::with_capacity(64));
                v.extend_from_slice(enc);
                v.extend_from_slice(mac);
                v
            }
            SymmetricKey::Cose { key, .. } => Zeroizing::new(key.to_vec()),
        }
    }

    /// Generates a new key from the system RNG.
    pub fn generate() -> Result<Self, CryptoError> {
        let mut bytes = Zeroizing::new([0u8; 64]);
        getrandom::fill(bytes.as_mut_slice()).map_err(|_| CryptoError::Rng)?;
        Self::from_bytes(bytes.as_slice())
    }

    pub fn decrypt(&self, enc: &EncString) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
        if let EncString::Cose { data } = enc {
            let msg = crate::cose::Encrypt0::parse(data)?;
            return msg.decrypt(&self.raw()).map(Zeroizing::new);
        }
        let EncString::AesCbc256HmacSha256 { iv, data, mac } = enc else {
            return Err(CryptoError::WrongEncType);
        };
        let SymmetricKey::AesCbcHmac {
            enc: enc_key,
            mac: mac_key,
        } = self
        else {
            return Err(CryptoError::WrongEncType);
        };

        // Check the MAC before touching the ciphertext (encrypt-then-MAC).
        let mut hmac = HmacSha256::new_from_slice(mac_key).expect("HMAC aceita qualquer chave");
        hmac.update(iv);
        hmac.update(data);
        hmac.verify_slice(mac)
            .map_err(|_| CryptoError::InvalidMac)?;

        let mut buf = Zeroizing::new(data.clone());
        let len = Aes256CbcDec::new(&(*enc_key).into(), &(*iv).into())
            .decrypt_padded::<Pkcs7>(&mut buf)
            .map_err(|_| CryptoError::Padding)?
            .len();
        buf.truncate(len);
        Ok(buf)
    }

    pub fn decrypt_str(&self, enc: &EncString) -> Result<Secret, CryptoError> {
        let bytes = self.decrypt(enc)?;
        let text = std::str::from_utf8(&bytes).map_err(|_| CryptoError::Utf8)?;
        Ok(Secret::new(text.to_owned()))
    }

    /// Encrypts a text (item fields). With v2 keys, uses the official clients'
    /// padded text format.
    pub fn encrypt_str(&self, text: &str) -> Result<EncString, CryptoError> {
        match self {
            SymmetricKey::Cose { alg, key, kid } => Ok(EncString::Cose {
                data: crate::cose::encrypt(*alg, key, kid.as_deref(), text.as_bytes(), true)?,
            }),
            _ => self.encrypt(text.as_bytes()),
        }
    }

    pub fn encrypt(&self, plaintext: &[u8]) -> Result<EncString, CryptoError> {
        if let SymmetricKey::Cose { alg, key, kid } = self {
            return Ok(EncString::Cose {
                data: crate::cose::encrypt(*alg, key, kid.as_deref(), plaintext, false)?,
            });
        }
        let SymmetricKey::AesCbcHmac {
            enc: enc_key,
            mac: mac_key,
        } = self
        else {
            return Err(CryptoError::WrongEncType);
        };
        let mut iv = [0u8; 16];
        getrandom::fill(&mut iv).map_err(|_| CryptoError::Rng)?;

        let mut buf = vec![0u8; plaintext.len() + 16];
        buf[..plaintext.len()].copy_from_slice(plaintext);
        let data = Aes256CbcEnc::new(&(*enc_key).into(), &iv.into())
            .encrypt_padded::<Pkcs7>(&mut buf, plaintext.len())
            .expect("buffer tem espaço para um bloco de padding")
            .to_vec();

        let mut hmac = HmacSha256::new_from_slice(mac_key).expect("HMAC aceita qualquer chave");
        hmac.update(&iv);
        hmac.update(&data);
        let mac: [u8; 32] = hmac.finalize().into_bytes().into();

        Ok(EncString::AesCbc256HmacSha256 { iv, data, mac })
    }
}

impl fmt::Debug for SymmetricKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SymmetricKey(***)")
    }
}

/// The user's RSA private key, used only to open organization keys.
pub struct RsaPrivateKey(rsa::PrivateKey);

impl RsaPrivateKey {
    /// `der` is the decrypted PKCS#8 from the profile's `privateKey` field.
    pub fn from_pkcs8_der(der: &[u8]) -> Result<Self, CryptoError> {
        use rsa::pkcs8::DecodePrivateKey;
        rsa::PrivateKey::from_pkcs8_der(der)
            .map(Self)
            .map_err(|_| CryptoError::RsaKey)
    }

    pub fn decrypt(&self, enc: &EncString) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
        let EncString::Rsa2048OaepSha1 { data } = enc else {
            return Err(CryptoError::WrongEncType);
        };
        self.0
            .oaep_decrypt::<sha1::Sha1>(data)
            .map(Zeroizing::new)
            .map_err(|_| CryptoError::Rsa)
    }
}

impl fmt::Debug for RsaPrivateKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RsaPrivateKey(***)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_then_decrypt_round_trips() {
        let key = SymmetricKey::generate().unwrap();
        for msg in ["", "a", "exatamente 16 b!", "Olá, coração 🔐 — com acentos"] {
            let enc = key.encrypt(msg.as_bytes()).unwrap();
            assert_eq!(key.decrypt_str(&enc).unwrap().expose(), msg);
        }
    }

    #[test]
    fn tampered_mac_is_rejected() {
        let key = SymmetricKey::generate().unwrap();
        let EncString::AesCbc256HmacSha256 { iv, mut data, mac } = key.encrypt(b"segredo").unwrap()
        else {
            unreachable!()
        };
        data[0] ^= 1;
        let tampered = EncString::AesCbc256HmacSha256 { iv, data, mac };
        assert_eq!(key.decrypt(&tampered).unwrap_err(), CryptoError::InvalidMac);
    }

    #[test]
    fn wrong_key_is_rejected() {
        let a = SymmetricKey::generate().unwrap();
        let b = SymmetricKey::generate().unwrap();
        let enc = a.encrypt(b"segredo").unwrap();
        assert_eq!(b.decrypt(&enc).unwrap_err(), CryptoError::InvalidMac);
    }

    #[test]
    fn key_must_be_64_bytes() {
        assert_eq!(
            SymmetricKey::from_bytes(&[0; 32]).unwrap_err(),
            CryptoError::KeyLength(32)
        );
    }

    #[test]
    fn debug_never_shows_key_material() {
        let key = SymmetricKey::from_bytes(&[7; 64]).unwrap();
        assert_eq!(format!("{key:?}"), "SymmetricKey(***)");
        let mk = MasterKey::from_bytes([7; 32]);
        assert_eq!(format!("{mk:?}"), "MasterKey(***)");
    }

    /// Bitwarden public vector: type 7 EncString with the CBC-HMAC AEAD and a
    /// legacy key (enc = 00..1f, mac = 20..3f).
    #[test]
    fn decrypts_public_type7_cbc_hmac_vector() {
        let key: Vec<u8> = (0u8..64).collect();
        let key = SymmetricKey::from_bytes(&key).unwrap();
        let enc: EncString = "7.g1g/owE6AAERegN4I2FwcGxpY2F0aW9uL3guYml0d2FyZGVuLnV0ZjgtcGFkZGVkBFCsTs/7Lgh6WRgNHdGZULjpoQVQXYJbsJat2eG+9CcclU7mqVhwKiZITE/+y7LqmpX+A+QS7iZ3Ng/0/+zZnW/TFQ//5t5qnJl6p0GV3t804h4rSfwEGtXvI27//Uaup0KEq+XU8kwx5m4VSq8JarD48spPgxlrmpLncTpFgIh+Z6YDPc300PsAbOj+bHT+dSLFH7cRHA=="
            .parse()
            .unwrap();
        assert_eq!(
            key.decrypt_str(&enc).unwrap().expose(),
            "Type 7 AES-256-CBC-HMAC test vector"
        );
    }

    /// End-to-end v2 account: the user key is a padded COSE_Key,
    /// wrapped by the stretched key (type 2); the fields come as type 7.
    #[test]
    fn v2_user_key_opens_type7_fields() {
        let stretched = MasterKey::from_bytes([5; 32]).stretch();
        let raw_user: Vec<u8> = (100u8..132).collect();
        let cose_key = crate::cose::pad_key(&crate::cose::encode_cose_key(
            crate::cose::ALG_XCHACHA20_POLY1305,
            &raw_user,
        ));
        let wrapped = stretched.encrypt(&cose_key).unwrap();

        let user = SymmetricKey::from_bytes(&stretched.decrypt(&wrapped).unwrap()).unwrap();
        assert!(matches!(user, SymmetricKey::Cose { .. }));

        let field = EncString::Cose {
            data: crate::cose::encrypt_xchacha_for_test(&raw_user, "senha-ção".as_bytes(), true),
        };
        // Round trip through the text, as it comes from the server.
        let field: EncString = field.to_string().parse().unwrap();
        assert_eq!(user.decrypt_str(&field).unwrap().expose(), "senha-ção");

        // Storing and reloading the v2 key preserves algorithm and kid.
        let again = SymmetricKey::from_bytes(&user.to_bytes()).unwrap();
        assert_eq!(again.decrypt_str(&field).unwrap().expose(), "senha-ção");
        let legacy = SymmetricKey::generate().unwrap();
        let back = SymmetricKey::from_bytes(&legacy.to_bytes()).unwrap();
        let enc = legacy.encrypt_str("x").unwrap();
        assert_eq!(back.decrypt_str(&enc).unwrap().expose(), "x");

        // Encrypt with the v2 key and decrypt back (random IV every time).
        let a = user.encrypt_str("nova senha ✓").unwrap();
        let b = user.encrypt_str("nova senha ✓").unwrap();
        assert!(matches!(a, EncString::Cose { .. }));
        assert_ne!(a.to_string(), b.to_string());
        let a: EncString = a.to_string().parse().unwrap();
        assert_eq!(user.decrypt_str(&a).unwrap().expose(), "nova senha ✓");

        let other = SymmetricKey::from_bytes(&crate::cose::pad_key(&crate::cose::encode_cose_key(
            crate::cose::ALG_XCHACHA20_POLY1305,
            &[1u8; 32],
        )))
        .unwrap();
        assert_eq!(other.decrypt(&field).unwrap_err(), CryptoError::InvalidMac);
    }
}
