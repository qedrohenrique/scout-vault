//! Scout Vault core: everything that talks to the Bitwarden-compatible server
//! and everything that encrypts/decrypts. It doesn't depend on the UI, so it can be tested on its own.
//!
//! - [`api`]        — HTTP client: prelogin, login (2FA, new device), refresh, sync
//! - [`crypto`]     — master key (PBKDF2/Argon2id), stretch (HKDF), AES-CBC + HMAC, RSA-OAEP
//! - [`import`]     — import from Bitwarden, browsers and other CSVs
//! - [`export`]     — export to Bitwarden JSON (plain or password-protected) and CSV
//! - [`icons`]      — website icons from the server's icon service
//! - [`edit`]       — writing: edited item → encrypted body for the server
//! - [`enc_string`] — `EncString` format (`2.iv|ct|mac`, `4.data`)
//! - [`kdf`]        — key derivation parameters
//! - [`ssh`]        — SSH keys: generate, import, public key and fingerprint
//! - [`ssh_agent`]  — SSH agent protocol: list the vault's keys and sign
//! - [`sync`]       — sync response, still encrypted (safe to cache)
//! - [`totp`]       — authenticator codes (RFC 6238, Steam)
//! - [`update`]     — newer releases on GitHub, installer download checked by SHA-256
//! - [`vault`]      — decrypted vault: items, folders, organizations, search

pub mod api;
pub mod attachment;
pub mod cose;
pub mod crypto;
pub mod edit;
pub mod enc_string;
pub mod export;
pub mod generator;
pub mod health;
pub mod icons;
pub mod import;
mod json;
pub mod kdf;
pub mod secret;
pub mod ssh;
pub mod ssh_agent;
pub mod sync;
pub mod totp;
pub mod update;
pub mod vault;

pub use enc_string::EncString;
pub use kdf::Kdf;
pub use secret::Secret;
