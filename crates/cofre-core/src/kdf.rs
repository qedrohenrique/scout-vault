//! Parâmetros de derivação da master key, vindos de `POST /identity/accounts/prelogin`.
//!
//! A derivação em si (PBKDF2-SHA256 / Argon2id) entra na fase 1, no módulo `crypto`.

use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kdf {
    /// Salt = e-mail em minúsculas.
    Pbkdf2Sha256 { iterations: u32 },
    /// Salt = SHA-256 do e-mail em minúsculas. Memória em MiB.
    Argon2id {
        iterations: u32,
        memory_mib: u32,
        parallelism: u32,
    },
}

/// Resposta crua do prelogin. Aceita camelCase e PascalCase porque
/// servidores diferentes (Bitwarden oficial / Vaultwarden, versões antigas) variam.
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
}

impl TryFrom<PreloginResponse> for Kdf {
    type Error = KdfError;

    fn try_from(r: PreloginResponse) -> Result<Self, Self::Error> {
        match r.kdf {
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
        }
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
    fn argon2id_missing_memory() {
        assert_eq!(
            parse(r#"{"kdf":1,"kdfIterations":3,"kdfParallelism":4}"#),
            Err(KdfError::MissingParam("kdfMemory"))
        );
    }
}
