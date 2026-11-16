//! Unlock with **Windows Hello** (face, fingerprint or PIN).
//!
//! How it works, without storing the open key anywhere:
//!
//! 1. When the option is turned on (with the vault open), Windows Hello creates an RSA
//!    key pair that **never leaves the TPM/Windows** and signs a random challenge.
//! 2. The signature (RSA PKCS#1 v1.5: always the same for the same challenge and the
//!    same key) becomes, via SHA-512, a key that encrypts the **user key**.
//! 3. Credential Manager holds only the challenge, the **encrypted** user key
//!    and the server's `profile.key` (to notice a password change).
//!
//! To open, Windows Hello signs the challenge again (asking for face/PIN) and
//! the user key is decrypted. Without the user's Hello, what is stored doesn't open.
//! Changing the master password (on any device) discards the setup.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use gpui_kit::{App, Global};
use scout_vault_core::crypto::SymmetricKey;
use scout_vault_core::vault::Keys;
use scout_vault_core::EncString;
use sha2::{Digest, Sha256, Sha512};
use zeroize::Zeroizing;

use crate::flow::Session;
use crate::settings::SettingsStore;

/// Signs a challenge with the user's Windows Hello key (asks for face/PIN).
pub type Signer = Arc<dyn Fn(&[u8]) -> Result<Vec<u8>, HelloError> + Send + Sync>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HelloError {
    /// The person canceled or preferred the password.
    Canceled,
    /// There is no Windows Hello set up on this computer.
    Unavailable,
    Failed(String),
}

impl HelloError {
    pub fn message(&self) -> String {
        match self {
            Self::Canceled => t!("hello.canceled").into(),
            Self::Unavailable => t!("settings.windows_hello_unavailable").into(),
            Self::Failed(e) => t!("hello.failed", e = e),
        }
    }
}

struct Provider {
    signer: Signer,
    supported: Arc<AtomicBool>,
}

impl Global for Provider {}

/// Uses `signer` as Windows Hello (tests pass a fake one).
pub fn install(cx: &mut App, signer: Signer) {
    cx.set_global(Provider {
        signer,
        supported: Arc::new(AtomicBool::new(true)),
    });
}

/// The real Windows Hello (app only; the support check runs separately).
pub fn install_system(cx: &mut App) {
    let supported = Arc::new(AtomicBool::new(false));
    let flag = supported.clone();
    std::thread::Builder::new()
        .name("scout-vault-hello-check".into())
        .spawn(move || flag.store(platform::is_supported(), Ordering::Relaxed))
        .ok();
    cx.set_global(Provider {
        signer: Arc::new(platform::sign),
        supported,
    });
}

/// Does Windows Hello exist and is it set up?
pub fn available(cx: &App) -> bool {
    cx.try_global::<Provider>()
        .is_some_and(|p| p.supported.load(Ordering::Relaxed))
}

pub fn signer(cx: &App) -> Option<Signer> {
    cx.try_global::<Provider>()
        .filter(|p| p.supported.load(Ordering::Relaxed))
        .map(|p| p.signer.clone())
}

/// Deterministic signer for tests (no real Windows Hello).
pub fn fake_signer(seed: &'static [u8]) -> Signer {
    Arc::new(move |data| {
        let mut h = Sha256::new();
        h.update(seed);
        h.update(data);
        Ok(h.finalize().to_vec())
    })
}

/// Signer that always "cancels" (tests).
pub fn canceling_signer() -> Signer {
    Arc::new(|_| Err(HelloError::Canceled))
}

// ---------------------------------------------------------------------------
// What is stored

const SEP: char = '\n';

pub fn key_name(identity_url: &str, email: &str) -> String {
    format!(
        "hello|{}|{}",
        identity_url.trim_end_matches('/'),
        email.trim().to_lowercase()
    )
}

fn session_key(session: &Session) -> String {
    key_name(&session.client.urls().identity, &session.email)
}

/// Challenge, encrypted user key and the `profile.key` from when it was turned on.
pub struct Sealed {
    pub challenge: Vec<u8>,
    protected: String,
    wrapped: EncString,
}

fn read(store: &SettingsStore, session: &Session) -> Option<Sealed> {
    let value = Zeroizing::new(store.secrets().get(&session_key(session))?);
    let mut parts = value.splitn(3, SEP);
    let challenge = STANDARD.decode(parts.next()?).ok()?;
    let protected = parts.next()?.to_owned();
    let wrapped = parts.next()?.parse().ok()?;
    Some(Sealed {
        challenge,
        protected,
        wrapped,
    })
}

/// What opens with Windows Hello for this session (if still valid).
pub fn sealed(store: &SettingsStore, session: &Session) -> Option<Sealed> {
    read(store, session).filter(|s| session.sync.profile.key.as_deref() == Some(&s.protected))
}

pub fn is_enrolled(store: &SettingsStore, session: &Session) -> bool {
    sealed(store, session).is_some()
}

/// A new challenge (32 random bytes).
pub fn new_challenge() -> Vec<u8> {
    let mut c = vec![0u8; 32];
    getrandom::fill(&mut c).expect("gerador aleatório do sistema");
    c
}

fn wrapping_key(signature: &[u8]) -> Result<SymmetricKey, HelloError> {
    if signature.len() < 32 {
        return Err(HelloError::Failed("assinatura curta demais".into()));
    }
    let mut h = Sha512::new();
    h.update(b"scout-vault/windows-hello/v1");
    h.update(signature);
    let bytes = Zeroizing::new(h.finalize().to_vec());
    SymmetricKey::from_bytes(&bytes).map_err(|e| HelloError::Failed(e.to_string()))
}

/// Stores the user key encrypted with the challenge signature.
pub fn seal(
    store: &SettingsStore,
    session: &Session,
    keys: &Keys,
    challenge: &[u8],
    signature: &[u8],
) -> Result<(), HelloError> {
    let Some(protected) = session.sync.profile.key.as_deref() else {
        return Err(HelloError::Failed("perfil sem chave".into()));
    };
    let wrapped = wrapping_key(signature)?
        .encrypt(&keys.user_key().to_bytes())
        .map_err(|e| HelloError::Failed(e.to_string()))?;
    let value = Zeroizing::new(format!(
        "{}{SEP}{protected}{SEP}{wrapped}",
        STANDARD.encode(challenge)
    ));
    store.secrets().set(&session_key(session), &value);
    Ok(())
}

/// Opens the keys with the signature of the stored challenge.
pub fn unseal(sealed: &Sealed, signature: &[u8], session: &Session) -> Result<Keys, HelloError> {
    let bytes = wrapping_key(signature)?
        .decrypt(&sealed.wrapped)
        .map_err(|_| HelloError::Failed(t!("hello.key_changed").into()))?;
    let user = SymmetricKey::from_bytes(&bytes).map_err(|e| HelloError::Failed(e.to_string()))?;
    Keys::from_user_key(user, &session.sync.profile)
        .map_err(|_| HelloError::Failed(t!("hello.password_changed").into()))
}

pub fn forget(store: &SettingsStore, session: &Session) {
    store.secrets().delete(&session_key(session));
}

/// Has the master password changed since Windows Hello was turned on? (then discard)
pub fn password_changed(store: &SettingsStore, session: &Session) -> bool {
    match read(store, session) {
        Some(s) if session.sync.profile.key.as_deref() != Some(&s.protected) => {
            forget(store, session);
            true
        }
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Windows

#[cfg(windows)]
mod platform {
    use windows::core::{w, Array, HSTRING};
    use windows::Security::Credentials::{
        KeyCredential, KeyCredentialCreationOption, KeyCredentialManager, KeyCredentialStatus,
    };
    use windows::Security::Cryptography::CryptographicBuffer;
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, SetForegroundWindow};

    use super::HelloError;

    /// Name of this app's Windows Hello key.
    const KEY_NAME: &str = "ScoutVault";

    pub fn is_supported() -> bool {
        KeyCredentialManager::IsSupportedAsync()
            .and_then(|op| op.join())
            .unwrap_or(false)
    }

    fn status_error(status: KeyCredentialStatus) -> HelloError {
        match status {
            KeyCredentialStatus::UserCanceled | KeyCredentialStatus::UserPrefersPassword => {
                HelloError::Canceled
            }
            KeyCredentialStatus::NotFound => HelloError::Unavailable,
            KeyCredentialStatus::SecurityDeviceLocked => {
                HelloError::Failed(t!("hello.device_locked").into())
            }
            other => HelloError::Failed(format!("código {}", other.0)),
        }
    }

    fn failed(e: windows::core::Error) -> HelloError {
        HelloError::Failed(e.message().to_string())
    }

    /// The Windows Hello window sometimes opens behind the app: bring it to the front.
    fn focus_prompt_soon() {
        std::thread::spawn(|| {
            for _ in 0..40 {
                std::thread::sleep(std::time::Duration::from_millis(75));
                // SAFETY: only looks up the system dialog window and activates it.
                unsafe {
                    if let Ok(hwnd) = FindWindowW(w!("Credential Dialog Xaml Host"), None) {
                        if !hwnd.is_invalid() {
                            let _ = SetForegroundWindow(hwnd);
                            return;
                        }
                    }
                }
            }
        });
    }

    fn credential() -> Result<KeyCredential, HelloError> {
        let name = HSTRING::from(KEY_NAME);
        let opened = KeyCredentialManager::OpenAsync(&name)
            .and_then(|op| op.join())
            .map_err(failed)?;
        if opened.Status().map_err(failed)? == KeyCredentialStatus::Success {
            return opened.Credential().map_err(failed);
        }
        // First time: creates the key (Windows asks for face/PIN).
        focus_prompt_soon();
        let created = KeyCredentialManager::RequestCreateAsync(
            &name,
            KeyCredentialCreationOption::ReplaceExisting,
        )
        .and_then(|op| op.join())
        .map_err(failed)?;
        match created.Status().map_err(failed)? {
            KeyCredentialStatus::Success => created.Credential().map_err(failed),
            other => Err(status_error(other)),
        }
    }

    pub fn sign(data: &[u8]) -> Result<Vec<u8>, HelloError> {
        let credential = credential()?;
        let buffer = CryptographicBuffer::CreateFromByteArray(data).map_err(failed)?;
        focus_prompt_soon();
        let result = credential
            .RequestSignAsync(&buffer)
            .and_then(|op| op.join())
            .map_err(failed)?;
        match result.Status().map_err(failed)? {
            KeyCredentialStatus::Success => {
                let signed = result.Result().map_err(failed)?;
                let mut out = Array::<u8>::new();
                CryptographicBuffer::CopyToByteArray(&signed, &mut out).map_err(failed)?;
                Ok(out.to_vec())
            }
            other => Err(status_error(other)),
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::HelloError;

    pub fn is_supported() -> bool {
        false
    }

    pub fn sign(_: &[u8]) -> Result<Vec<u8>, HelloError> {
        Err(HelloError::Unavailable)
    }
}
