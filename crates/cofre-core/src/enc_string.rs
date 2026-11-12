//! `EncString`: o formato textual que o Bitwarden usa para todo campo cifrado.
//!
//! - Tipo 2: `2.<iv b64>|<ciphertext b64>|<mac b64>` — AES-256-CBC + HMAC-SHA256
//!   (praticamente todos os campos do cofre e a chave do usuário).
//! - Tipo 4: `4.<data b64>` — RSA-2048-OAEP-SHA1 (chaves de organização).
//!
//! Este módulo só faz parse/serialização; decifrar fica em `crypto` (fase 1).

use std::fmt;
use std::str::FromStr;

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EncString {
    AesCbc256HmacSha256 {
        iv: [u8; 16],
        data: Vec<u8>,
        mac: [u8; 32],
    },
    Rsa2048OaepSha1 {
        data: Vec<u8>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    #[error("EncString sem prefixo de tipo")]
    MissingType,
    #[error("tipo de EncString não suportado: {0}")]
    UnsupportedType(String),
    #[error("número de partes incorreto: esperado {expected}, recebido {got}")]
    WrongParts { expected: usize, got: usize },
    #[error("base64 inválido")]
    Base64,
    #[error("tamanho inválido no campo {0}")]
    Length(&'static str),
}

fn decode(s: &str) -> Result<Vec<u8>, ParseError> {
    STANDARD.decode(s).map_err(|_| ParseError::Base64)
}

impl FromStr for EncString {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (ty, rest) = s.split_once('.').ok_or(ParseError::MissingType)?;
        let parts: Vec<&str> = rest.split('|').collect();

        match ty {
            "2" => {
                if parts.len() != 3 {
                    return Err(ParseError::WrongParts {
                        expected: 3,
                        got: parts.len(),
                    });
                }
                let iv: [u8; 16] = decode(parts[0])?
                    .try_into()
                    .map_err(|_| ParseError::Length("iv"))?;
                let data = decode(parts[1])?;
                let mac: [u8; 32] = decode(parts[2])?
                    .try_into()
                    .map_err(|_| ParseError::Length("mac"))?;
                Ok(EncString::AesCbc256HmacSha256 { iv, data, mac })
            }
            "4" => {
                if parts.len() != 1 {
                    return Err(ParseError::WrongParts {
                        expected: 1,
                        got: parts.len(),
                    });
                }
                Ok(EncString::Rsa2048OaepSha1 {
                    data: decode(parts[0])?,
                })
            }
            other => Err(ParseError::UnsupportedType(other.to_string())),
        }
    }
}

impl fmt::Display for EncString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EncString::AesCbc256HmacSha256 { iv, data, mac } => write!(
                f,
                "2.{}|{}|{}",
                STANDARD.encode(iv),
                STANDARD.encode(data),
                STANDARD.encode(mac)
            ),
            EncString::Rsa2048OaepSha1 { data } => write!(f, "4.{}", STANDARD.encode(data)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_type2() -> String {
        format!(
            "2.{}|{}|{}",
            STANDARD.encode([1u8; 16]),
            STANDARD.encode(b"ciphertext"),
            STANDARD.encode([2u8; 32])
        )
    }

    #[test]
    fn parses_type2() {
        let parsed: EncString = sample_type2().parse().unwrap();
        assert_eq!(
            parsed,
            EncString::AesCbc256HmacSha256 {
                iv: [1; 16],
                data: b"ciphertext".to_vec(),
                mac: [2; 32],
            }
        );
    }

    #[test]
    fn round_trips() {
        let s = sample_type2();
        assert_eq!(s.parse::<EncString>().unwrap().to_string(), s);

        let rsa = format!("4.{}", STANDARD.encode(b"rsa"));
        assert_eq!(rsa.parse::<EncString>().unwrap().to_string(), rsa);
    }

    #[test]
    fn rejects_bad_input() {
        assert_eq!("semtipo".parse::<EncString>(), Err(ParseError::MissingType));
        assert_eq!(
            "9.abc".parse::<EncString>(),
            Err(ParseError::UnsupportedType("9".into()))
        );
        assert_eq!(
            "2.a|b".parse::<EncString>(),
            Err(ParseError::WrongParts {
                expected: 3,
                got: 2
            })
        );
        let short_iv = format!(
            "2.{}|{}|{}",
            STANDARD.encode([0u8; 8]),
            STANDARD.encode(b"x"),
            STANDARD.encode([0u8; 32])
        );
        assert_eq!(short_iv.parse::<EncString>(), Err(ParseError::Length("iv")));
    }
}
