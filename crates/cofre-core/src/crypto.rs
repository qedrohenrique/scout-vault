//! Criptografia do protocolo Bitwarden.
//!
//! Fluxo de chaves:
//! ```text
//! senha + e-mail ──KDF──▶ MasterKey ──PBKDF2(1)──▶ hash de autenticação (vai ao servidor)
//!                             │
//!                         HKDF-Expand ("enc", "mac")
//!                             ▼
//!                    chave esticada ──decifra──▶ UserKey (64 B: enc ‖ mac)
//!                                                  │
//!                         ┌────────────────────────┼─────────────────────┐
//!                         ▼                        ▼                     ▼
//!                  campos do cofre        chave privada RSA       chaves de item
//!                                                  │
//!                                       RSA-OAEP-SHA1 ▼
//!                                         chaves de organização
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
}

/// Chave derivada da senha mestra. Nunca sai da máquina.
pub struct MasterKey(Zeroizing<[u8; 32]>);

impl MasterKey {
    /// Deriva a master key. **Lento de propósito** (centenas de ms a segundos):
    /// chame fora da thread de UI.
    pub fn derive(password: &str, email: &str, kdf: Kdf) -> Result<Self, CryptoError> {
        let email = email.trim().to_lowercase();
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

    /// Hash enviado ao servidor no login: PBKDF2-SHA256(master key, salt = senha, 1 iteração).
    pub fn server_hash(&self, password: &str) -> String {
        let hash: Zeroizing<[u8; 32]> = Zeroizing::new(pbkdf2::pbkdf2_hmac_array::<Sha256, 32>(
            self.0.as_slice(),
            password.as_bytes(),
            1,
        ));
        STANDARD.encode(hash.as_slice())
    }

    /// Estica a master key em uma chave de cifra + chave de MAC via HKDF-Expand-SHA256.
    pub fn stretch(&self) -> SymmetricKey {
        let hk = hkdf::Hkdf::<Sha256>::from_prk(self.0.as_slice())
            .expect("PRK de 32 bytes tem o tamanho mínimo do SHA-256");
        let mut key = SymmetricKey {
            enc: [0; 32],
            mac: [0; 32],
        };
        hk.expand(b"enc", &mut key.enc)
            .expect("32 bytes está dentro do limite do HKDF");
        hk.expand(b"mac", &mut key.mac)
            .expect("32 bytes está dentro do limite do HKDF");
        key
    }
}

impl fmt::Debug for MasterKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MasterKey(***)")
    }
}

/// Chave AES-256-CBC + HMAC-SHA256 (user key, chave de organização ou de item).
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct SymmetricKey {
    enc: [u8; 32],
    mac: [u8; 32],
}

impl SymmetricKey {
    /// Interpreta 64 bytes decifrados como `enc ‖ mac`.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CryptoError> {
        if bytes.len() != 64 {
            return Err(CryptoError::KeyLength(bytes.len()));
        }
        let mut key = SymmetricKey {
            enc: [0; 32],
            mac: [0; 32],
        };
        key.enc.copy_from_slice(&bytes[..32]);
        key.mac.copy_from_slice(&bytes[32..]);
        Ok(key)
    }

    /// Gera uma chave nova a partir do RNG do sistema.
    pub fn generate() -> Result<Self, CryptoError> {
        let mut bytes = Zeroizing::new([0u8; 64]);
        getrandom::fill(bytes.as_mut_slice()).map_err(|_| CryptoError::Rng)?;
        Self::from_bytes(bytes.as_slice())
    }

    pub fn decrypt(&self, enc: &EncString) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
        let EncString::AesCbc256HmacSha256 { iv, data, mac } = enc else {
            return Err(CryptoError::WrongEncType);
        };

        // Verifica o MAC antes de tocar no texto cifrado (encrypt-then-MAC).
        let mut hmac = HmacSha256::new_from_slice(&self.mac).expect("HMAC aceita qualquer chave");
        hmac.update(iv);
        hmac.update(data);
        hmac.verify_slice(mac)
            .map_err(|_| CryptoError::InvalidMac)?;

        let mut buf = Zeroizing::new(data.clone());
        let len = Aes256CbcDec::new(&self.enc.into(), &(*iv).into())
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

    pub fn encrypt(&self, plaintext: &[u8]) -> Result<EncString, CryptoError> {
        let mut iv = [0u8; 16];
        getrandom::fill(&mut iv).map_err(|_| CryptoError::Rng)?;

        let mut buf = vec![0u8; plaintext.len() + 16];
        buf[..plaintext.len()].copy_from_slice(plaintext);
        let data = Aes256CbcEnc::new(&self.enc.into(), &iv.into())
            .encrypt_padded::<Pkcs7>(&mut buf, plaintext.len())
            .expect("buffer tem espaço para um bloco de padding")
            .to_vec();

        let mut hmac = HmacSha256::new_from_slice(&self.mac).expect("HMAC aceita qualquer chave");
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

/// Chave privada RSA do usuário, usada só para abrir as chaves de organização.
pub struct RsaPrivateKey(rsa::PrivateKey);

impl RsaPrivateKey {
    /// `der` é o PKCS#8 decifrado do campo `privateKey` do perfil.
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
}
