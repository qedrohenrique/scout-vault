//! "Ask for the master password on open: every N days".
//!
//! After the person types the master password, the **user key** is stored in
//! Windows Credential Manager (protected by the Windows user)
//! with an expiry. Opening the app within that period opens the vault directly.
//!
//! Along with it goes the **encrypted** user key as the server had it (`profile.key`).
//! If the master password changes (on another device, in the web vault…), that
//! value changes on the next sync and the stored key is discarded: the app asks
//! for the password again — now the new one.
//!
//! The expiry counts from the last time the password was typed; opening the
//! app without a password doesn't extend it.

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use scout_vault_core::crypto::SymmetricKey;
use scout_vault_core::vault::Keys;
use zeroize::Zeroizing;

use crate::flow::{now_secs, Session};
use crate::settings::SettingsStore;

/// Separator of the stored fields (`EncString` uses `|`, so it won't do).
const SEP: char = '\n';

fn key_name(session: &Session) -> String {
    format!(
        "desbloqueio|{}|{}",
        session.client.urls().identity.trim_end_matches('/'),
        session.email.trim().to_lowercase()
    )
}

/// Stores the key for `days` days (if the option is on).
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

/// Opens the keys without a password, if there is a stored key within the period and
/// still valid for this profile. Any problem discards the key.
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

/// Has the master password changed since the key was stored? (then it is discarded)
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
