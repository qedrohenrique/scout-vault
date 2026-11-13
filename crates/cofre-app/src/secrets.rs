//! Segredos pequenos que precisam sobreviver entre aberturas do app — hoje só o
//! token de "lembrar este dispositivo" do 2FA. Ficam no **Gerenciador de
//! Credenciais do Windows** (protegido pela conta do usuário), nunca no
//! `settings.json`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

const SERVICE: &str = "Cofre";

/// Onde os segredos moram: no cofre do sistema operacional, ou na memória (testes).
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
                .ok(),
            Self::Memory(m) => m.lock().ok()?.get(key).cloned(),
        }
    }

    /// Falhas ao gravar não impedem o login — só custam pedir o 2FA de novo.
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

/// Chave do token de "lembrar dispositivo": um por servidor e conta.
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
