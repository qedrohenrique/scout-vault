//! Preferências não sensíveis, salvas em JSON: servidor, último e-mail e a
//! identidade deste dispositivo. **Nada secreto vai para este arquivo.**

use std::path::PathBuf;

use cofre_core::api::{DeviceInfo, ServerUrls};
use cofre_core::generator::PasswordOptions;
use serde::{Deserialize, Serialize};

use crate::appearance::Appearance;
use crate::secrets::SecretStore;

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
    /// Identificador da organização no último login por SSO.
    #[serde(default)]
    pub last_sso_identifier: String,
    /// Mesmo identificador em todos os logins: evita alertas de "novo dispositivo".
    pub device: DeviceInfo,
    /// Tema, cor de destaque e afins.
    #[serde(default)]
    pub appearance: Appearance,
    /// Ao abrir o app, ir direto para o desbloqueio (sessão e cache cifrado guardados).
    #[serde(default = "yes")]
    pub remember_session: bool,
    /// Extensão do navegador pode pedir logins ao app (desligado por padrão).
    #[serde(default)]
    pub browser_fill: bool,
    /// Opções do gerador de senhas (a senha gerada nunca é salva).
    #[serde(default)]
    pub generator: PasswordOptions,
    /// Preenchimento em outros apps pelo atalho global.
    #[serde(default)]
    pub autotype: AutoTypeSettings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AutoTypeSettings {
    pub enabled: bool,
    /// Aperta Enter depois da senha.
    pub press_enter: bool,
}

impl Default for AutoTypeSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            press_enter: true,
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            server: ServerChoice::default(),
            self_hosted_url: String::new(),
            last_email: String::new(),
            last_sso_identifier: String::new(),
            device: DeviceInfo::new(&device_name()),
            appearance: Appearance::default(),
            generator: PasswordOptions::default(),
            remember_session: true,
            browser_fill: false,
            autotype: AutoTypeSettings::default(),
        }
    }
}

fn yes() -> bool {
    true
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
#[derive(Clone, Default)]
pub struct SettingsStore {
    path: Option<PathBuf>,
    /// Segredos que acompanham este armazenamento: o cofre do sistema quando as
    /// preferências vão para o disco; a memória nos testes.
    secrets: SecretStore,
    /// Sessão lembrada quando não há arquivo (testes).
    session_mem: std::sync::Arc<std::sync::Mutex<Option<Vec<u8>>>>,
}

impl std::fmt::Debug for SettingsStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SettingsStore")
            .field("path", &self.path)
            .finish()
    }
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
            secrets: SecretStore::Os,
            session_mem: Default::default(),
        }
    }

    pub fn in_memory() -> Self {
        Self::default()
    }

    /// Preferências num arquivo à parte; os segredos ficam na memória (testes).
    pub fn at(path: PathBuf) -> Self {
        Self {
            path: Some(path),
            secrets: SecretStore::default(),
            session_mem: Default::default(),
        }
    }

    pub fn secrets(&self) -> &SecretStore {
        &self.secrets
    }

    /// Lê o arquivo da sessão lembrada (ou a cópia em memória, nos testes).
    pub(crate) fn read_session_blob(&self) -> Option<Vec<u8>> {
        match &self.path {
            Some(p) => std::fs::read(p.with_file_name("sessao.json")).ok(),
            None => self.session_mem.lock().ok()?.clone(),
        }
    }

    /// Pasta das preferências (`%APPDATA%\Cofre`), se houver.
    pub fn dir(&self) -> Option<PathBuf> {
        self.path
            .as_ref()
            .and_then(|p| p.parent().map(PathBuf::from))
    }

    /// Para testes: há uma sessão guardada (mesmo que incompleta)?
    pub fn read_session_blob_for_tests(&self) -> Option<Vec<u8>> {
        self.read_session_blob()
    }

    pub(crate) fn write_session_blob(&self, bytes: Option<&[u8]>) {
        match &self.path {
            Some(p) => {
                let file = p.with_file_name("sessao.json");
                match bytes {
                    Some(b) => {
                        if let Some(dir) = file.parent() {
                            let _ = std::fs::create_dir_all(dir);
                        }
                        // Grava num temporário e troca: nunca deixa um arquivo pela metade.
                        let tmp = file.with_extension("json.tmp");
                        if std::fs::write(&tmp, b).is_ok() {
                            let _ = std::fs::rename(&tmp, &file);
                        }
                    }
                    None => {
                        let _ = std::fs::remove_file(file);
                    }
                }
            }
            None => {
                if let Ok(mut m) = self.session_mem.lock() {
                    *m = bytes.map(<[u8]>::to_vec);
                }
            }
        }
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
