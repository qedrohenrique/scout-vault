//! Remembered session: open the app straight to unlock, without email or 2FA.
//!
//! What is stored and where:
//! - `%APPDATA%\Scout Vault\sessao.json`: server, email, KDF and the **encrypted sync**
//!   exactly as the server sent it. Without the master password nothing in it
//!   can be read — it is the same thing the server stores.
//! - **Windows Credential Manager**: the refresh token (that is what
//!   allows syncing without logging in). The access token is not stored.
//!
//! "Sign out", an expired session or turning the option off deletes both.

use std::sync::Arc;

use scout_vault_core::api::{Client, ServerUrls};
use scout_vault_core::sync::SyncResponse;
use scout_vault_core::Kdf;
use serde::{Deserialize, Serialize};

use crate::flow::Session;
use crate::settings::SettingsStore;

const VERSION: u8 = 1;

#[derive(Serialize, Deserialize)]
struct Saved {
    version: u8,
    urls: ServerUrls,
    sso: bool,
    email: String,
    kdf: Kdf,
    sync: SyncResponse,
    /// Last full login (0 = unknown: sessions saved before this option existed).
    #[serde(default)]
    logged_in_at: u64,
}

fn token_key(urls: &ServerUrls, email: &str) -> String {
    format!(
        "sessao|{}|{}",
        urls.identity.trim_end_matches('/'),
        email.trim().to_lowercase()
    )
}

/// Stores the session (if the option is on).
pub fn save(store: &SettingsStore, session: &Session) {
    if !store.load().remember_session || session.refresh_token.is_empty() {
        return;
    }
    let urls = session.client.urls().clone();
    let saved = Saved {
        version: VERSION,
        sso: session.client.is_sso(),
        email: session.email.clone(),
        kdf: session.kdf,
        sync: (*session.sync).clone(),
        urls: urls.clone(),
        logged_in_at: session.logged_in_at,
    };
    let Ok(bytes) = serde_json::to_vec(&saved) else {
        return;
    };
    store
        .secrets()
        .set(&token_key(&urls, &session.email), &session.refresh_token);
    store.write_session_blob(Some(&bytes));
}

/// Stored session, ready for offline unlock. `None` if there is none,
/// if the option is off or if the refresh token is missing.
pub fn load(store: &SettingsStore) -> Option<Session> {
    let settings = store.load();
    if !settings.remember_session {
        return None;
    }
    let saved: Saved = serde_json::from_slice(&store.read_session_blob()?).ok()?;
    if saved.version != VERSION {
        return None;
    }
    // "Ask for a full login every N days": past the deadline, forget the session.
    let logged_in_at = if saved.logged_in_at == 0 {
        crate::flow::now_secs()
    } else {
        saved.logged_in_at
    };
    if settings.relogin_days > 0
        && crate::flow::now_secs().saturating_sub(logged_in_at)
            > u64::from(settings.relogin_days) * 86_400
    {
        clear(store);
        return None;
    }
    let refresh_token = store.secrets().get(&token_key(&saved.urls, &saved.email))?;
    let client = if saved.sso {
        Client::for_sso(saved.urls, settings.device)
    } else {
        Client::new(saved.urls, settings.device)
    }
    .ok()?;
    Some(Session {
        client: Arc::new(client),
        email: saved.email,
        kdf: saved.kdf,
        // Expired either way: the first sync renews it with the refresh token.
        access_token: String::new(),
        refresh_token,
        sync: Arc::new(saved.sync),
        logged_in_at,
    })
}

/// Forgets the session: deletes the file and the refresh token.
pub fn clear(store: &SettingsStore) {
    if let Some(saved) = store
        .read_session_blob()
        .and_then(|b| serde_json::from_slice::<Saved>(&b).ok())
    {
        store
            .secrets()
            .delete(&token_key(&saved.urls, &saved.email));
        store
            .secrets()
            .delete(&quick_key(&saved.urls, &saved.email));
        store
            .secrets()
            .delete(&crate::hello::key_name(&saved.urls.identity, &saved.email));
    }
    store.write_session_blob(None);
}

/// Same name that [`crate::quick_unlock`] uses for the stored key.
fn quick_key(urls: &ServerUrls, email: &str) -> String {
    format!(
        "desbloqueio|{}|{}",
        urls.identity.trim_end_matches('/'),
        email.trim().to_lowercase()
    )
}

/// Deletes only the remembered session's "open without password" key (the option was turned off).
pub fn forget_quick_unlock(store: &SettingsStore) {
    if let Some(saved) = store
        .read_session_blob()
        .and_then(|b| serde_json::from_slice::<Saved>(&b).ok())
    {
        store
            .secrets()
            .delete(&quick_key(&saved.urls, &saved.email));
    }
}
