//! Preferências não sensíveis, salvas em JSON: servidor, último e-mail e a
//! identidade deste dispositivo. **Nada secreto vai para este arquivo.**

use std::path::PathBuf;

use cofre_core::api::{DeviceInfo, ServerUrls};
use serde::{Deserialize, Serialize};

use crate::appearance::Appearance;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ServerChoice {
    #[default]
    BitwardenUs,
    BitwardenEu,
    SelfHosted,
}

impl ServerChoice {
    pub const ALL: [ServerChoice; 3] = [Self::BitwardenUs, Self::BitwardenEu, Self::SelfHosted];

    pub fn label(self) -> &'static str {
        match self {
            Self::BitwardenUs => "bitwarden.com",
            Self::BitwardenEu => "bitwarden.eu",
            Self::SelfHosted => "Servidor próprio",
        }
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|c| *c == self).unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub server: ServerChoice,
    #[serde(default)]
    pub self_hosted_url: String,
    #[serde(default)]
    pub last_email: String,
    /// Mesmo identificador em todos os logins: evita alertas de "novo dispositivo".
    pub device: DeviceInfo,
    /// Tema, cor de destaque e afins.
    #[serde(default)]
    pub appearance: Appearance,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            server: ServerChoice::default(),
            self_hosted_url: String::new(),
            last_email: String::new(),
            device: DeviceInfo::new(&device_name()),
            appearance: Appearance::default(),
        }
    }
}

impl Settings {
    pub fn server_urls(&self) -> Result<ServerUrls, String> {
        match self.server {
            ServerChoice::BitwardenUs => Ok(ServerUrls::bitwarden_us()),
            ServerChoice::BitwardenEu => Ok(ServerUrls::bitwarden_eu()),
            ServerChoice::SelfHosted => {
                let url = self.self_hosted_url.trim();
                if !(url.starts_with("https://") || url.starts_with("http://")) {
                    return Err("Informe o endereço do servidor, começando com https://".into());
                }
                Ok(ServerUrls::self_hosted(url))
            }
        }
    }
}

/// Onde as preferências moram. `None` = não persistir (testes).
#[derive(Debug, Clone, Default)]
pub struct SettingsStore {
    path: Option<PathBuf>,
}

impl SettingsStore {
    /// `%APPDATA%\Cofre\settings.json` no Windows; `~/.config/cofre/settings.json` nos demais.
    pub fn default_location() -> Self {
        let dir = std::env::var_os("APPDATA")
            .map(|d| PathBuf::from(d).join("Cofre"))
            .or_else(|| {
                std::env::var_os("XDG_CONFIG_HOME")
                    .map(PathBuf::from)
                    .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
                    .map(|d| d.join("cofre"))
            });
        Self {
            path: dir.map(|d| d.join("settings.json")),
        }
    }

    pub fn in_memory() -> Self {
        Self { path: None }
    }

    pub fn at(path: PathBuf) -> Self {
        Self { path: Some(path) }
    }

    pub fn load(&self) -> Settings {
        let Some(path) = &self.path else {
            return Settings::default();
        };
        let loaded = std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok());
        match loaded {
            Some(s) => s,
            None => {
                // Primeira execução: cria a identidade do dispositivo e já grava.
                let s = Settings::default();
                self.save(&s);
                s
            }
        }
    }

    /// Falhas ao salvar não impedem o uso — só custam um alerta de novo dispositivo depois.
    pub fn save(&self, settings: &Settings) {
        let Some(path) = &self.path else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_vec_pretty(settings) {
            let _ = std::fs::write(path, json);
        }
    }
}

fn device_name() -> String {
    let host = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_default();
    if host.is_empty() {
        "Cofre".into()
    } else {
        format!("Cofre ({host})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_identity_survives_restart() {
        let dir = std::env::temp_dir().join(format!("cofre-test-{}", std::process::id()));
        let store = SettingsStore::at(dir.join("settings.json"));
        let first = store.load();
        let second = store.load();
        assert_eq!(first.device.identifier, second.device.identifier);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn self_hosted_requires_scheme() {
        let mut s = Settings {
            server: ServerChoice::SelfHosted,
            self_hosted_url: "vault.exemplo.com".into(),
            ..Settings::default()
        };
        assert!(s.server_urls().is_err());
        s.self_hosted_url = "https://vault.exemplo.com/".into();
        assert_eq!(
            s.server_urls().unwrap().api,
            "https://vault.exemplo.com/api"
        );
    }
}
