//! Desbloquear com **Windows Hello** (rosto, digital ou PIN).
//!
//! Como funciona, sem guardar a chave aberta em lugar nenhum:
//!
//! 1. Ao ligar a opção (com o cofre aberto), o Windows Hello cria um par de
//!    chaves RSA que **nunca sai do TPM/Windows** e assina um desafio aleatório.
//! 2. A assinatura (RSA PKCS#1 v1.5: sempre a mesma para o mesmo desafio e a
//!    mesma chave) vira, por SHA-512, uma chave que cifra a **user key**.
//! 3. No Gerenciador de Credenciais ficam só o desafio, a user key **cifrada**
//!    e a `profile.key` do servidor (para perceber troca de senha).
//!
//! Para abrir, o Windows Hello assina o desafio de novo (pedindo rosto/PIN) e
//! a user key é decifrada. Sem o Hello do usuário, o que está guardado não abre.
//! Trocar a senha mestra (em qualquer aparelho) descarta a configuração.

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

/// Assina um desafio com a chave do Windows Hello do usuário (pede rosto/PIN).
pub type Signer = Arc<dyn Fn(&[u8]) -> Result<Vec<u8>, HelloError> + Send + Sync>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HelloError {
    /// A pessoa cancelou ou preferiu a senha.
    Canceled,
    /// Não há Windows Hello configurado neste computador.
    Unavailable,
    Failed(String),
}

impl HelloError {
    pub fn message(&self) -> String {
        match self {
            Self::Canceled => "Windows Hello cancelado. Use a senha mestra.".into(),
            Self::Unavailable => {
                "O Windows Hello não está configurado neste computador (Configurações do Windows › Contas › Opções de entrada).".into()
            }
            Self::Failed(e) => format!("O Windows Hello não abriu o cofre: {e}"),
        }
    }
}

struct Provider {
    signer: Signer,
    supported: Arc<AtomicBool>,
}

impl Global for Provider {}

/// Usa `signer` como Windows Hello (os testes passam um falso).
pub fn install(cx: &mut App, signer: Signer) {
    cx.set_global(Provider {
        signer,
        supported: Arc::new(AtomicBool::new(true)),
    });
}

/// O Windows Hello de verdade (só no app; a checagem de suporte roda à parte).
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

/// O Windows Hello existe e está configurado?
pub fn available(cx: &App) -> bool {
    cx.try_global::<Provider>()
        .is_some_and(|p| p.supported.load(Ordering::Relaxed))
}

pub fn signer(cx: &App) -> Option<Signer> {
    cx.try_global::<Provider>()
        .filter(|p| p.supported.load(Ordering::Relaxed))
        .map(|p| p.signer.clone())
}

/// Assinador determinístico para testes (nada de Windows Hello de verdade).
pub fn fake_signer(seed: &'static [u8]) -> Signer {
    Arc::new(move |data| {
        let mut h = Sha256::new();
        h.update(seed);
        h.update(data);
        Ok(h.finalize().to_vec())
    })
}

/// Assinador que sempre "cancela" (testes).
pub fn canceling_signer() -> Signer {
    Arc::new(|_| Err(HelloError::Canceled))
}

// ---------------------------------------------------------------------------
// O que fica guardado

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

/// Desafio, user key cifrada e a `profile.key` de quando foi ligado.
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

/// O que abre com o Windows Hello para esta sessão (se ainda vale).
pub fn sealed(store: &SettingsStore, session: &Session) -> Option<Sealed> {
    read(store, session).filter(|s| session.sync.profile.key.as_deref() == Some(&s.protected))
}

pub fn is_enrolled(store: &SettingsStore, session: &Session) -> bool {
    sealed(store, session).is_some()
}

/// Um desafio novo (32 bytes aleatórios).
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

/// Guarda a user key cifrada com a assinatura do desafio.
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

/// Abre as chaves com a assinatura do desafio guardado.
pub fn unseal(sealed: &Sealed, signature: &[u8], session: &Session) -> Result<Keys, HelloError> {
    let bytes = wrapping_key(signature)?
        .decrypt(&sealed.wrapped)
        .map_err(|_| {
            HelloError::Failed("a chave do Windows Hello mudou; ligue a opção de novo".into())
        })?;
    let user = SymmetricKey::from_bytes(&bytes).map_err(|e| HelloError::Failed(e.to_string()))?;
    Keys::from_user_key(user, &session.sync.profile)
        .map_err(|_| HelloError::Failed("a senha mestra mudou; ligue a opção de novo".into()))
}

pub fn forget(store: &SettingsStore, session: &Session) {
    store.secrets().delete(&session_key(session));
}

/// A senha mestra mudou desde que o Windows Hello foi ligado? (então descarta)
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

    /// Nome da chave do Windows Hello deste app.
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
                HelloError::Failed("o dispositivo de segurança está bloqueado".into())
            }
            other => HelloError::Failed(format!("código {}", other.0)),
        }
    }

    fn failed(e: windows::core::Error) -> HelloError {
        HelloError::Failed(e.message().to_string())
    }

    /// A janela do Windows Hello às vezes abre atrás do app: traz para a frente.
    fn focus_prompt_soon() {
        std::thread::spawn(|| {
            for _ in 0..40 {
                std::thread::sleep(std::time::Duration::from_millis(75));
                // SAFETY: só procura a janela do diálogo do sistema e a ativa.
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
        // Primeira vez: cria a chave (o Windows pede rosto/PIN).
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
