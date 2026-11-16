//! Small secrets that need to survive between app launches — today only the
//! 2FA "remember this device" token. They live in **Windows Credential
//! Manager** (protected by the user's account), never in
//! `settings.json`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

const SERVICE: &str = "Scout Vault";
/// The app's name before it became Scout Vault: old secrets migrate when read.
const OLD_SERVICE: &str = "Cofre";

/// Where secrets live: in the operating system's credential store, or in memory (tests).
#[derive(Clone)]
pub enum SecretStore {
    Os,
    Memory(Arc<Mutex<HashMap<String, String>>>),
}

impl Default for SecretStore {
    fn default() -> Self {
        Self::Memory(Arc::default())
    }
}

impl SecretStore {
    pub fn get(&self, key: &str) -> Option<String> {
        match self {
            Self::Os => keyring::Entry::new(SERVICE, key)
                .and_then(|e| e.get_password())
                .ok()
                .or_else(|| {
                    let old = keyring::Entry::new(OLD_SERVICE, key).ok()?;
                    let value = old.get_password().ok()?;
                    self.set(key, &value);
                    let _ = old.delete_credential();
                    Some(value)
                }),
            Self::Memory(m) => m.lock().ok()?.get(key).cloned(),
        }
    }

    /// Write failures don't block login — they only cost asking for 2FA again.
    pub fn set(&self, key: &str, value: &str) {
        match self {
            Self::Os => {
                let _ = keyring::Entry::new(SERVICE, key).and_then(|e| e.set_password(value));
            }
            Self::Memory(m) => {
                if let Ok(mut m) = m.lock() {
                    m.insert(key.to_owned(), value.to_owned());
                }
            }
        }
    }

    pub fn delete(&self, key: &str) {
        match self {
            Self::Os => {
                let _ = keyring::Entry::new(SERVICE, key).and_then(|e| e.delete_credential());
            }
            Self::Memory(m) => {
                if let Ok(mut m) = m.lock() {
                    m.remove(key);
                }
            }
        }
    }
}

/// Key of the "remember device" token: one per server and account.
pub fn remember_key(identity_url: &str, email: &str) -> String {
    format!(
        "2fa-lembrar|{}|{}",
        identity_url.trim_end_matches('/'),
        email.trim().to_lowercase()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_round_trip() {
        let s = SecretStore::default();
        let k = remember_key("https://identity.bitwarden.com/", " Eu@Exemplo.com ");
        assert_eq!(
            k,
            "2fa-lembrar|https://identity.bitwarden.com|eu@exemplo.com"
        );
        assert_eq!(s.get(&k), None);
        s.set(&k, "tok");
        assert_eq!(s.get(&k).as_deref(), Some("tok"));
        s.delete(&k);
        assert_eq!(s.get(&k), None);
    }
}
