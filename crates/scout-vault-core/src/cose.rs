//! Criptografia "v2" do Bitwarden: mensagens COSE_Encrypt0 (RFC 9052) em CBOR.
//!
//! Implementado a partir dos padrões (RFC 9052/9053, C2SP XAES-256-GCM) e da
//! descrição pública do formato; **só decifra** — o Scout Vault ainda não escreve.
//!
//! - `EncString` tipo 7: `7.<base64 padrão do COSE_Encrypt0>`
//!   (`[protected bstr, unprotected map, ciphertext ‖ tag]`).
//! - Cabeçalho protegido: `alg` (1), `content type` (3), `kid` (4).
//!   Cabeçalho desprotegido: `iv` (5).
//! - AAD = `Enc_structure` = `["Encrypt0", protected, h'']`.
//! - Texto com content type `application/x.bitwarden.utf8-padded` vem com
//!   padding no estilo PKCS#7 (o último byte diz quantos bytes remover).
//! - Chaves v2 são `COSE_Key` serializadas e depois "acolchoadas" até ≥ 65 bytes
//!   (mesmo padding), o que as distingue das chaves legadas de 64 bytes.

use chacha20poly1305::aead;
use ciborium::value::Value;

use crate::crypto::CryptoError;

/// XChaCha20-Poly1305 (uso privado do Bitwarden).
pub const ALG_XCHACHA20_POLY1305: i64 = -70000;
/// XAES-256-GCM (uso privado do Bitwarden; c2sp.org/XAES-256-GCM).
pub const ALG_XAES_256_GCM: i64 = -70010;
/// AES-256-CBC + HMAC-SHA256 como AEAD, com chave legada de 64 bytes.
pub const ALG_AES_256_CBC_HMAC_AEAD: i64 = -70011;
/// AES-256-GCM (IANA `A256GCM`).
pub const ALG_A256GCM: i64 = 3;

const CONTENT_TYPE_PADDED_UTF8: &str = "application/x.bitwarden.utf8-padded";

/// Uma mensagem COSE_Encrypt0 já separada em partes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Encrypt0 {
    protected: Vec<u8>,
    pub alg: i64,
    padded_text: bool,
    pub iv: Vec<u8>,
    pub ciphertext: Vec<u8>,
}

fn int(v: &Value) -> Option<i64> {
    match v {
        Value::Integer(i) => i64::try_from(*i).ok(),
        _ => None,
    }
}

fn map_get(map: &[(Value, Value)], label: i64) -> Option<&Value> {
    map.iter()
        .find(|(k, _)| int(k) == Some(label))
        .map(|(_, v)| v)
}

fn decode(bytes: &[u8]) -> Result<Value, CryptoError> {
    ciborium::de::from_reader(bytes).map_err(|_| CryptoError::Cose("CBOR inválido"))
}

impl Encrypt0 {
    pub fn parse(bytes: &[u8]) -> Result<Self, CryptoError> {
        let Value::Array(parts) = decode(bytes)? else {
            return Err(CryptoError::Cose("COSE_Encrypt0 deve ser um array"));
        };
        // Aceita a forma com tag (16) e sem tag.
        let parts = match parts.as_slice() {
            [Value::Bytes(_), Value::Map(_), Value::Bytes(_)] => parts,
            _ => return Err(CryptoError::Cose("COSE_Encrypt0 com formato inesperado")),
        };
        let (Value::Bytes(protected), Value::Map(unprotected), Value::Bytes(ciphertext)) =
            (&parts[0], &parts[1], &parts[2])
        else {
            unreachable!()
        };

        let header = match decode(protected)? {
            Value::Map(m) => m,
            _ => return Err(CryptoError::Cose("cabeçalho protegido não é um mapa")),
        };
        let alg = map_get(&header, 1)
            .and_then(int)
            .ok_or(CryptoError::Cose("sem algoritmo no cabeçalho"))?;
        let padded_text = matches!(
            map_get(&header, 3),
            Some(Value::Text(t)) if t == CONTENT_TYPE_PADDED_UTF8
        );
        let iv = match map_get(unprotected, 5) {
            Some(Value::Bytes(b)) => b.clone(),
            _ => return Err(CryptoError::Cose("sem IV")),
        };
        Ok(Self {
            protected: protected.clone(),
            alg,
            padded_text,
            iv,
            ciphertext: ciphertext.clone(),
        })
    }

    /// AAD de RFC 9052 §5.3: `["Encrypt0", protected, external_aad = h'']`.
    fn aad(&self) -> Vec<u8> {
        let structure = Value::Array(vec![
            Value::Text("Encrypt0".into()),
            Value::Bytes(self.protected.clone()),
            Value::Bytes(Vec::new()),
        ]);
        let mut out = Vec::new();
        ciborium::ser::into_writer(&structure, &mut out).expect("serializar em Vec não falha");
        out
    }

    /// Decifra com uma chave COSE de 32 bytes (XChaCha20, XAES ou AES-GCM) ou
    /// com uma chave legada de 64 bytes (`enc ‖ mac`, só para o AEAD CBC-HMAC).
    pub fn decrypt(&self, key: &[u8]) -> Result<Vec<u8>, CryptoError> {
        let aad = self.aad();
        let mut plain = match self.alg {
            ALG_XCHACHA20_POLY1305 => aead_decrypt::<chacha20poly1305::XChaCha20Poly1305>(
                key,
                &self.iv,
                &self.ciphertext,
                &aad,
            )?,
            ALG_XAES_256_GCM => {
                aead_decrypt::<xaes_256_gcm::Xaes256Gcm>(key, &self.iv, &self.ciphertext, &aad)?
            }
            ALG_A256GCM => {
                aead_decrypt::<aes_gcm::Aes256Gcm>(key, &self.iv, &self.ciphertext, &aad)?
            }
            ALG_AES_256_CBC_HMAC_AEAD => {
                cbc_hmac_aead_decrypt(key, &self.iv, &self.ciphertext, &aad)?
            }
            _ => return Err(CryptoError::Cose("algoritmo COSE não suportado")),
        };
        if self.padded_text {
            let len = unpad(&plain)?.len();
            plain.truncate(len);
        }
        Ok(plain)
    }
}

fn aead_decrypt<A>(
    key: &[u8],
    iv: &[u8],
    ciphertext: &[u8],
    aad: &[u8],
) -> Result<Vec<u8>, CryptoError>
where
    A: aead::Aead + aead::KeyInit,
{
    let cipher = A::new_from_slice(key).map_err(|_| CryptoError::KeyLength(key.len()))?;
    let nonce =
        aead::Nonce::<A>::try_from(iv).map_err(|_| CryptoError::Cose("IV de tamanho errado"))?;
    cipher
        .decrypt(
            &nonce,
            aead::Payload {
                msg: ciphertext,
                aad,
            },
        )
        .map_err(|_| CryptoError::InvalidMac)
}

/// AES-256-CBC + HMAC-SHA256 como AEAD: a tag (32 bytes, no fim) é
/// HMAC(mac, "bitwarden.aes256-cbc-hmac-sha256-aead.v1" ‖ iv ‖ len(ct) ‖ ct ‖ len(aad) ‖ aad),
/// com tamanhos em u64 big-endian.
fn cbc_hmac_aead_decrypt(
    key: &[u8],
    iv: &[u8],
    data: &[u8],
    aad: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    use aes::cipher::block_padding::Pkcs7;
    use aes::cipher::{BlockModeDecrypt, KeyIvInit};
    use hmac::{Hmac, KeyInit, Mac};

    if key.len() != 64 {
        return Err(CryptoError::KeyLength(key.len()));
    }
    let iv: [u8; 16] = iv
        .try_into()
        .map_err(|_| CryptoError::Cose("IV de tamanho errado"))?;
    if data.len() < 32 {
        return Err(CryptoError::InvalidMac);
    }
    let (ct, tag) = data.split_at(data.len() - 32);
    let mut mac =
        Hmac::<sha2::Sha256>::new_from_slice(&key[32..]).expect("HMAC aceita qualquer chave");
    mac.update(b"bitwarden.aes256-cbc-hmac-sha256-aead.v1");
    mac.update(&iv);
    mac.update(&(ct.len() as u64).to_be_bytes());
    mac.update(ct);
    mac.update(&(aad.len() as u64).to_be_bytes());
    mac.update(aad);
    mac.verify_slice(tag).map_err(|_| CryptoError::InvalidMac)?;

    let enc: [u8; 32] = key[..32].try_into().expect("32 bytes");
    let mut buf = ct.to_vec();
    let len = cbc::Decryptor::<aes::Aes256>::new(&enc.into(), &iv.into())
        .decrypt_padded::<Pkcs7>(&mut buf)
        .map_err(|_| CryptoError::Padding)?
        .len();
    buf.truncate(len);
    Ok(buf)
}

/// Remove o padding no estilo PKCS#7 usado em chaves e textos v2.
pub fn unpad(bytes: &[u8]) -> Result<&[u8], CryptoError> {
    let n = *bytes.last().ok_or(CryptoError::Padding)? as usize;
    if n == 0 || n > bytes.len() {
        return Err(CryptoError::Padding);
    }
    Ok(&bytes[..bytes.len() - n])
}

/// Chave simétrica v2 extraída de uma `COSE_Key`.
pub struct CoseKey {
    pub alg: i64,
    pub key: Vec<u8>,
    /// Identificador da chave (label 2). Vai no cabeçalho do que ciframos:
    /// os clientes oficiais conferem que bate com a chave.
    pub kid: Option<Vec<u8>>,
}

/// Lê uma `COSE_Key` simétrica (kty = 4): `alg` (3) e `k` (-1).
pub fn parse_cose_key(bytes: &[u8]) -> Result<CoseKey, CryptoError> {
    let Value::Map(map) = decode(bytes)? else {
        return Err(CryptoError::Cose("COSE_Key deve ser um mapa"));
    };
    if map_get(&map, 1).and_then(int) != Some(4) {
        return Err(CryptoError::Cose("COSE_Key não é simétrica"));
    }
    let alg = map_get(&map, 3)
        .and_then(int)
        .ok_or(CryptoError::Cose("COSE_Key sem algoritmo"))?;
    let key = match map_get(&map, -1) {
        Some(Value::Bytes(k)) if k.len() == 32 => k.clone(),
        _ => return Err(CryptoError::Cose("COSE_Key sem chave de 32 bytes")),
    };
    let kid = match map_get(&map, 2) {
        Some(Value::Bytes(k)) => Some(k.clone()),
        _ => None,
    };
    Ok(CoseKey { alg, key, kid })
}

/// Cifra em `COSE_Encrypt0` com XChaCha20-Poly1305 e IV aleatório.
/// `padded_text` marca o conteúdo como texto acolchoado (o formato dos campos
/// de texto do Bitwarden), escondendo o tamanho exato.
pub fn encrypt(
    alg: i64,
    key: &[u8],
    kid: Option<&[u8]>,
    plaintext: &[u8],
    padded_text: bool,
) -> Result<Vec<u8>, CryptoError> {
    use aead::{Aead, KeyInit, Payload};
    use ciborium::value::Integer;
    if alg != ALG_XCHACHA20_POLY1305 {
        return Err(CryptoError::Cose(
            "cifrar só é suportado com XChaCha20-Poly1305",
        ));
    }
    let mut header = vec![(
        Value::Integer(Integer::from(1)),
        Value::Integer(Integer::from(ALG_XCHACHA20_POLY1305)),
    )];
    if padded_text {
        header.push((
            Value::Integer(Integer::from(3)),
            Value::Text(CONTENT_TYPE_PADDED_UTF8.into()),
        ));
    }
    if let Some(kid) = kid {
        header.push((Value::Integer(Integer::from(4)), Value::Bytes(kid.to_vec())));
    }
    let mut plain = zeroize::Zeroizing::new(plaintext.to_vec());
    if padded_text {
        let len = plain.len();
        let n = (32 * (1 + len / 32)).saturating_sub(len).max(1);
        plain.resize(len + n, n as u8);
    }
    let mut protected = Vec::new();
    ciborium::ser::into_writer(&Value::Map(header), &mut protected)
        .map_err(|_| CryptoError::Cose("falha ao montar o cabeçalho"))?;
    let mut iv = [0u8; 24];
    getrandom::fill(&mut iv).map_err(|_| CryptoError::Rng)?;
    let msg = Encrypt0 {
        protected: protected.clone(),
        alg,
        padded_text,
        iv: iv.to_vec(),
        ciphertext: Vec::new(),
    };
    let cipher = chacha20poly1305::XChaCha20Poly1305::new_from_slice(key)
        .map_err(|_| CryptoError::Cose("chave inválida"))?;
    let nonce = aead::Nonce::<chacha20poly1305::XChaCha20Poly1305>::try_from(&iv[..])
        .map_err(|_| CryptoError::Cose("IV inválido"))?;
    let ct = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: &plain,
                aad: &msg.aad(),
            },
        )
        .map_err(|_| CryptoError::Cose("falha ao cifrar"))?;
    let array = Value::Array(vec![
        Value::Bytes(protected),
        Value::Map(vec![(
            Value::Integer(Integer::from(5)),
            Value::Bytes(iv.to_vec()),
        )]),
        Value::Bytes(ct),
    ]);
    let mut out = Vec::new();
    ciborium::ser::into_writer(&array, &mut out)
        .map_err(|_| CryptoError::Cose("falha ao serializar"))?;
    Ok(out)
}

/// Acolchoa uma chave serializada até pelo menos 65 bytes (o inverso de [`unpad`]).
pub(crate) fn pad_key(bytes: &[u8]) -> Vec<u8> {
    let n = 65usize.saturating_sub(bytes.len()).max(1);
    let mut out = bytes.to_vec();
    out.resize(bytes.len() + n, n as u8);
    out
}

/// Serializa uma chave simétrica como `COSE_Key` (kty 4), com o `kid` se houver.
pub(crate) fn encode_cose_key_with_kid(alg: i64, key: &[u8], kid: Option<&[u8]>) -> Vec<u8> {
    use ciborium::value::Integer;
    let mut map = vec![(
        Value::Integer(Integer::from(1)),
        Value::Integer(Integer::from(4)),
    )];
    if let Some(kid) = kid {
        map.push((Value::Integer(Integer::from(2)), Value::Bytes(kid.to_vec())));
    }
    map.push((
        Value::Integer(Integer::from(3)),
        Value::Integer(Integer::from(alg)),
    ));
    map.push((
        Value::Integer(Integer::from(-1)),
        Value::Bytes(key.to_vec()),
    ));
    let mut out = Vec::new();
    ciborium::ser::into_writer(&Value::Map(map), &mut out).expect("mapa simples serializa");
    out
}

#[cfg(test)]
pub(crate) fn encode_cose_key(alg: i64, key: &[u8]) -> Vec<u8> {
    encode_cose_key_with_kid(alg, key, Some(&[0; 16]))
}

/// Cifra uma mensagem v2 com XChaCha20-Poly1305 — só para testes (o app não escreve).
#[cfg(test)]
pub(crate) fn encrypt_xchacha_for_test(key: &[u8], plaintext: &[u8], padded_text: bool) -> Vec<u8> {
    use aead::{Aead, KeyInit, Payload};
    use ciborium::value::Integer;
    let mut header = vec![(
        Value::Integer(Integer::from(1)),
        Value::Integer(Integer::from(ALG_XCHACHA20_POLY1305)),
    )];
    let mut plain = plaintext.to_vec();
    if padded_text {
        header.push((
            Value::Integer(Integer::from(3)),
            Value::Text(CONTENT_TYPE_PADDED_UTF8.into()),
        ));
        let target = 32 * (1 + plain.len() / 32);
        let n = target.saturating_sub(plain.len()).max(1);
        plain.resize(plain.len() + n, n as u8);
    }
    let mut protected = Vec::new();
    ciborium::ser::into_writer(&Value::Map(header), &mut protected).unwrap();
    let iv = [7u8; 24];
    let msg = Encrypt0 {
        protected: protected.clone(),
        alg: ALG_XCHACHA20_POLY1305,
        padded_text,
        iv: iv.to_vec(),
        ciphertext: Vec::new(),
    };
    let cipher = chacha20poly1305::XChaCha20Poly1305::new_from_slice(key).unwrap();
    let nonce = aead::Nonce::<chacha20poly1305::XChaCha20Poly1305>::try_from(&iv[..]).unwrap();
    let ct = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: &plain,
                aad: &msg.aad(),
            },
        )
        .unwrap();
    let array = Value::Array(vec![
        Value::Bytes(protected),
        Value::Map(vec![(
            Value::Integer(Integer::from(5)),
            Value::Bytes(iv.to_vec()),
        )]),
        Value::Bytes(ct),
    ]);
    let mut out = Vec::new();
    ciborium::ser::into_writer(&array, &mut out).unwrap();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // Vetor de teste público do Bitwarden (XChaCha20-Poly1305, chave 00..1f,
    // texto "Message test vector").
    const XCHACHA_VECTOR: &[u8] = &[
        131, 88, 28, 163, 1, 58, 0, 1, 17, 111, 3, 24, 42, 4, 80, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10,
        11, 12, 13, 14, 15, 161, 5, 88, 24, 78, 20, 28, 157, 180, 246, 131, 220, 82, 104, 72, 73,
        75, 43, 69, 139, 216, 167, 145, 220, 67, 168, 144, 173, 88, 35, 127, 234, 194, 83, 189,
        172, 65, 29, 156, 73, 98, 87, 231, 87, 129, 15, 235, 127, 125, 97, 211, 51, 212, 211, 2,
        13, 36, 123, 53, 12, 31, 191, 40, 13, 175,
    ];

    fn key_00_1f() -> Vec<u8> {
        (0u8..32).collect()
    }

    #[test]
    fn decrypts_public_xchacha20_vector() {
        let msg = Encrypt0::parse(XCHACHA_VECTOR).unwrap();
        assert_eq!(msg.alg, ALG_XCHACHA20_POLY1305);
        assert_eq!(msg.iv.len(), 24);
        assert_eq!(msg.decrypt(&key_00_1f()).unwrap(), b"Message test vector");
    }

    #[test]
    fn wrong_key_or_tampering_fails() {
        let msg = Encrypt0::parse(XCHACHA_VECTOR).unwrap();
        assert_eq!(msg.decrypt(&[9u8; 32]), Err(CryptoError::InvalidMac));
        let mut bad = XCHACHA_VECTOR.to_vec();
        *bad.last_mut().unwrap() ^= 1;
        let msg = Encrypt0::parse(&bad).unwrap();
        assert_eq!(msg.decrypt(&key_00_1f()), Err(CryptoError::InvalidMac));
    }

    #[test]
    fn xaes_matches_c2sp_vectors() {
        use aead::{Aead, KeyInit, Payload};
        let nonce = b"ABCDEFGHIJKLMNOPQRSTUVWX";
        for (k, aad, expected) in [
            (
                1u8,
                &b""[..],
                "ce546ef63c9cc60765923609b33a9a1974e96e52daf2fcf7075e2271",
            ),
            (
                3u8,
                &b"c2sp.org/XAES-256-GCM"[..],
                "986ec1832593df5443a179437fd083bf3fdb41abd740a21f71eb769d",
            ),
        ] {
            let ct: Vec<u8> = (0..expected.len() / 2)
                .map(|i| u8::from_str_radix(&expected[2 * i..2 * i + 2], 16).unwrap())
                .collect();
            let cipher = xaes_256_gcm::Xaes256Gcm::new_from_slice(&[k; 32]).unwrap();
            let n = aead::Nonce::<xaes_256_gcm::Xaes256Gcm>::try_from(&nonce[..]).unwrap();
            let pt = cipher.decrypt(&n, Payload { msg: &ct, aad }).unwrap();
            assert_eq!(pt, b"XAES-256-GCM");
        }
    }

    #[test]
    fn cose_key_round_trip_with_padding() {
        let padded = pad_key(&encode_cose_key(ALG_XCHACHA20_POLY1305, &key_00_1f()));
        assert!(padded.len() >= 65);
        let key = parse_cose_key(unpad(&padded).unwrap()).unwrap();
        assert_eq!(key.alg, ALG_XCHACHA20_POLY1305);
        assert_eq!(key.key, key_00_1f());
    }
}
