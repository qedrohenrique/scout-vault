//! Núcleo do Cofre: tudo que fala com o servidor compatível com Bitwarden
//! e tudo que cifra/decifra. Não depende de UI, para ser testável isoladamente.
//!
//! Módulos planejados (ver `docs/ROADMAP.md`):
//! - `kdf`        — parâmetros de derivação de chave (PBKDF2 / Argon2id)
//! - `enc_string` — formato `EncString` (`2.iv|ct|mac`, `4.data`)
//! - `crypto`     — master key, stretch (HKDF), AES-CBC + HMAC, RSA-OAEP   (fase 1)
//! - `api`        — prelogin, login, refresh, sync                         (fase 1)
//! - `vault`      — modelo decifrado: itens, pastas, organizações         (fase 1)

pub mod enc_string;
pub mod kdf;

pub use enc_string::EncString;
pub use kdf::Kdf;
