//! Master key derivation parameters, from `POST /identity/accounts/prelogin`.
//!
//! The derivation itself (PBKDF2-SHA256 / Argon2id) lives in the `crypto` module.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kdf {
    /// Salt = lowercase email.
    Pbkdf2Sha256 { iterations: u32 },
    /// Salt = SHA-256 of the lowercase email. Memory in MiB.
    Argon2id {
        iterations: u32,
        memory_mib: u32,
        parallelism: u32,
    },
}

/// Raw prelogin response. Accepts camelCase and PascalCase because
/// different servers (official Bitwarden / Vaultwarden, old versions) vary.
#[derive(Debug, Deserialize)]
pub struct PreloginResponse {
    #[serde(alias = "Kdf")]
    pub kdf: u8,
    #[serde(rename = "kdfIterations", alias = "KdfIterations")]
    pub kdf_iterations: u32,
    #[serde(rename = "kdfMemory", alias = "KdfMemory", default)]
    pub kdf_memory: Option<u32>,
    #[serde(rename = "kdfParallelism", alias = "KdfParallelism", default)]
    pub kdf_parallelism: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KdfError {
    #[error("tipo de KDF desconhecido: {0}")]
    UnknownType(u8),
    #[error("Argon2id sem parâmetro obrigatório: {0}")]
    MissingParam(&'static str),
    /// The server asked for a derivation that is too weak. Accepting it would let whoever
    /// controls (or intercepts) the server crack the master password offline from the login hash.
    #[error(
        "o servidor pediu uma derivação de chave fraca demais ({0}); login recusado por segurança"
    )]
    TooWeak(String),
}

/// Accepted minimums, the same as Bitwarden's official clients.
pub const PBKDF2_MIN_ITERATIONS: u32 = 5_000;
pub const ARGON2_MIN_ITERATIONS: u32 = 2;
pub const ARGON2_MIN_MEMORY_MIB: u32 = 16;
pub const ARGON2_MIN_PARALLELISM: u32 = 1;

impl Kdf {
    /// Refuses parameters below the minimums (from the server or from a saved session).
    pub fn validate(&self) -> Result<(), KdfError> {
        match *self {
            Kdf::Pbkdf2Sha256 { iterations } if iterations < PBKDF2_MIN_ITERATIONS => Err(
                KdfError::TooWeak(format!(
                    "PBKDF2 com {iterations} iterações; mínimo {PBKDF2_MIN_ITERATIONS}"
                )),
            ),
            Kdf::Argon2id {
                iterations,
                memory_mib,
                parallelism,
            } if iterations < ARGON2_MIN_ITERATIONS
                || memory_mib < ARGON2_MIN_MEMORY_MIB
                || parallelism < ARGON2_MIN_PARALLELISM =>
            {
                Err(KdfError::TooWeak(format!(
                    "Argon2id t={iterations}, m={memory_mib} MiB, p={parallelism}; mínimo                      t={ARGON2_MIN_ITERATIONS}, m={ARGON2_MIN_MEMORY_MIB} MiB, p={ARGON2_MIN_PARALLELISM}"
                )))
            }
            _ => Ok(()),
        }
    }
}

impl TryFrom<PreloginResponse> for Kdf {
    type Error = KdfError;

    fn try_from(r: PreloginResponse) -> Result<Self, Self::Error> {
        let kdf = match r.kdf {
            0 => Ok(Kdf::Pbkdf2Sha256 {
                iterations: r.kdf_iterations,
            }),
            1 => Ok(Kdf::Argon2id {
                iterations: r.kdf_iterations,
                memory_mib: r.kdf_memory.ok_or(KdfError::MissingParam("kdfMemory"))?,
                parallelism: r
                    .kdf_parallelism
                    .ok_or(KdfError::MissingParam("kdfParallelism"))?,
            }),
            other => Err(KdfError::UnknownType(other)),
        }?;
        kdf.validate()?;
        Ok(kdf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: &str) -> Result<Kdf, KdfError> {
        serde_json::from_str::<PreloginResponse>(json)
            .unwrap()
            .try_into()
    }

    #[test]
    fn pbkdf2() {
        assert_eq!(
            parse(r#"{"kdf":0,"kdfIterations":600000,"kdfMemory":null,"kdfParallelism":null}"#),
            Ok(Kdf::Pbkdf2Sha256 { iterations: 600000 })
        );
    }

    #[test]
    fn argon2id_pascal_case() {
        assert_eq!(
            parse(r#"{"Kdf":1,"KdfIterations":3,"KdfMemory":64,"KdfParallelism":4}"#),
            Ok(Kdf::Argon2id {
                iterations: 3,
                memory_mib: 64,
                parallelism: 4
            })
        );
    }

    #[test]
    fn weak_parameters_from_the_server_are_refused() {
        for json in [
            r#"{"kdf":0,"kdfIterations":1}"#,
            r#"{"kdf":0,"kdfIterations":4999}"#,
            r#"{"kdf":1,"kdfIterations":1,"kdfMemory":64,"kdfParallelism":4}"#,
            r#"{"kdf":1,"kdfIterations":3,"kdfMemory":1,"kdfParallelism":4}"#,
            r#"{"kdf":1,"kdfIterations":3,"kdfMemory":64,"kdfParallelism":0}"#,
        ] {
            assert!(
                matches!(parse(json), Err(KdfError::TooWeak(_))),
                "deveria recusar {json}"
            );
        }
        // Exactly at the minimums: accepted.
        assert!(parse(r#"{"kdf":0,"kdfIterations":5000}"#).is_ok());
        assert!(parse(r#"{"kdf":1,"kdfIterations":2,"kdfMemory":16,"kdfParallelism":1}"#).is_ok());
    }

    #[test]
    fn argon2id_missing_memory() {
        assert_eq!(
            parse(r#"{"kdf":1,"kdfIterations":3,"kdfParallelism":4}"#),
            Err(KdfError::MissingParam("kdfMemory"))
        );
    }
}
