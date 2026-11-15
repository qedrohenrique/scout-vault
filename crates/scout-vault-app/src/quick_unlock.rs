//! "Pedir a senha mestra ao abrir: a cada N dias".
//!
//! Depois que a pessoa digita a senha mestra, a **user key** fica guardada no
//! Gerenciador de Credenciais do Windows (protegido pelo usuário do Windows)
//! com uma validade. Ao abrir o app dentro do prazo, o cofre abre direto.
//!
//! Junto vai a user key **cifrada** como o servidor a tinha (`profile.key`).
//! Se a senha mestra for trocada (em outro aparelho, no cofre web…), esse
//! valor muda no próximo sync e a chave guardada é descartada: o app volta a
//! pedir a senha — agora a nova.
//!
//! A validade conta a partir da última vez que a senha foi digitada; abrir o
//! app sem senha não estende o prazo.

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use scout_vault_core::crypto::SymmetricKey;
use scout_vault_core::vault::Keys;
use zeroize::Zeroizing;

use crate::flow::{now_secs, Session};
use crate::settings::SettingsStore;

/// Separador dos campos guardados (`EncString` usa `|`, então não serve).
const SEP: char = '\n';

fn key_name(session: &Session) -> String {
    format!(
        "desbloqueio|{}|{}",
        session.client.urls().identity.trim_end_matches('/'),
        session.email.trim().to_lowercase()
    )
}

/// Guarda a chave por `days` dias (se a opção estiver ligada).
pub fn remember(store: &SettingsStore, session: &Session, keys: &Keys) {
    let days = store.load().unlock_memory_days;
    if days == 0 {
        forget(store, session);
        return;
    }
    let Some(protected) = session.sync.profile.key.as_deref() else {
        return;
    };
    let expires = now_secs() + u64::from(days) * 86_400;
    let bytes = keys.user_key().to_bytes();
    let value = Zeroizing::new(format!(
        "{expires}{SEP}{protected}{SEP}{}",
        STANDARD.encode(&*bytes)
    ));
    store.secrets().set(&key_name(session), &value);
}

/// Abre as chaves sem senha, se houver uma chave guardada dentro do prazo e
/// ainda válida para este perfil. Qualquer problema descarta a chave.
pub fn open(store: &SettingsStore, session: &Session) -> Option<Keys> {
    if store.load().unlock_memory_days == 0 {
        forget(store, session);
        return None;
    }
    let value = Zeroizing::new(store.secrets().get(&key_name(session))?);
    let mut parts = value.splitn(3, SEP);
    let (expires, protected, key) = (parts.next()?, parts.next()?, parts.next()?);
    let valid = expires.parse::<u64>().is_ok_and(|e| e > now_secs())
        && session.sync.profile.key.as_deref() == Some(protected);
    let keys = valid
        .then(|| STANDARD.decode(key).ok())
        .flatten()
        .map(Zeroizing::new)
        .and_then(|bytes| SymmetricKey::from_bytes(&bytes).ok())
        .and_then(|user| Keys::from_user_key(user, &session.sync.profile).ok());
    if keys.is_none() {
        forget(store, session);
    }
    keys
}

/// A senha mestra mudou desde que a chave foi guardada? (então ela é descartada)
pub fn password_changed(store: &SettingsStore, session: &Session) -> bool {
    let Some(value) = store.secrets().get(&key_name(session)).map(Zeroizing::new) else {
        return false;
    };
    let protected = value.split(SEP).nth(1).unwrap_or_default();
    let changed = session.sync.profile.key.as_deref() != Some(protected);
    if changed {
        forget(store, session);
    }
    changed
}

pub fn forget(store: &SettingsStore, session: &Session) {
    store.secrets().delete(&key_name(session));
}
