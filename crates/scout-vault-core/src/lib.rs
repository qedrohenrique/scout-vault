//! Núcleo do Scout Vault: tudo que fala com o servidor compatível com Bitwarden
//! e tudo que cifra/decifra. Não depende de UI, para ser testável isoladamente.
//!
//! - [`api`]        — cliente HTTP: prelogin, login (2FA, novo dispositivo), refresh, sync
//! - [`crypto`]     — master key (PBKDF2/Argon2id), stretch (HKDF), AES-CBC + HMAC, RSA-OAEP
//! - [`edit`]       — escrita: item editado → corpo cifrado para o servidor
//! - [`enc_string`] — formato `EncString` (`2.iv|ct|mac`, `4.data`)
//! - [`kdf`]        — parâmetros de derivação de chave
//! - [`sync`]       — resposta do sync, ainda cifrada (segura para cache)
//! - [`totp`]       — códigos do autenticador (RFC 6238, Steam)
//! - [`vault`]      — cofre decifrado: itens, pastas, organizações, busca

pub mod api;
pub mod attachment;
pub mod cose;
pub mod crypto;
pub mod edit;
pub mod enc_string;
pub mod generator;
mod json;
pub mod kdf;
pub mod secret;
pub mod sync;
pub mod totp;
pub mod vault;

pub use enc_string::EncString;
pub use kdf::Kdf;
pub use secret::Secret;
