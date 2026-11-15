//! Sessão lembrada: abrir o app direto no desbloqueio, sem e-mail nem 2FA.
//!
//! O que fica guardado e onde:
//! - `%APPDATA%\Scout Vault\sessao.json`: servidor, e-mail, KDF e o **sync cifrado**
//!   exatamente como o servidor mandou. Sem a senha mestra não dá para ler
//!   nada dele — é o mesmo que o servidor guarda.
//! - **Gerenciador de Credenciais do Windows**: o refresh token (é ele que
//!   permite sincronizar sem login). O access token não é guardado.
//!
//! "Sair da conta", sessão expirada ou desligar a opção apagam os dois.

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
    /// Último login completo (0 = desconhecido: sessões salvas antes desta opção).
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

/// Guarda a sessão (se a opção estiver ligada).
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

/// Sessão guardada, pronta para o desbloqueio offline. `None` se não houver,
/// se a opção estiver desligada ou se faltar o refresh token.
pub fn load(store: &SettingsStore) -> Option<Session> {
    let settings = store.load();
    if !settings.remember_session {
        return None;
    }
    let saved: Saved = serde_json::from_slice(&store.read_session_blob()?).ok()?;
    if saved.version != VERSION {
        return None;
    }
    // "Pedir login completo a cada N dias": passou do prazo, esquece a sessão.
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
        // Vencido de qualquer jeito: o primeiro sync renova com o refresh token.
        access_token: String::new(),
        refresh_token,
        sync: Arc::new(saved.sync),
        logged_in_at,
    })
}

/// Esquece a sessão: apaga o arquivo e o refresh token.
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
    }
    store.write_session_blob(None);
}

/// Mesmo nome que [`crate::quick_unlock`] usa para a chave guardada.
fn quick_key(urls: &ServerUrls, email: &str) -> String {
    format!(
        "desbloqueio|{}|{}",
        urls.identity.trim_end_matches('/'),
        email.trim().to_lowercase()
    )
}

/// Apaga só a chave de "abrir sem senha" da sessão lembrada (a opção foi desligada).
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
