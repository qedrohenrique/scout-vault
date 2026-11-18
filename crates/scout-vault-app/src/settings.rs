//! Non-sensitive preferences, saved as JSON: server, last email and this
//! device's identity. **Nothing secret goes into this file.**

use std::path::PathBuf;

use scout_vault_core::api::{DeviceInfo, ServerUrls};
use scout_vault_core::generator::{
    GeneratorKind, PassphraseOptions, PasswordOptions, UsernameOptions,
};
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
            Self::SelfHosted => t!("settings.server_self_hosted"),
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
    /// Organization identifier from the last SSO login.
    #[serde(default)]
    pub last_sso_identifier: String,
    /// Same identifier on every login: avoids "new device" alerts.
    pub device: DeviceInfo,
    /// Theme, accent color and the like.
    #[serde(default)]
    pub appearance: Appearance,
    /// Interface language by code (`pt-BR`, `en`); empty = the system's.
    #[serde(default)]
    pub language: String,
    /// Open the app without asking for the master password for this many days after typing it
    /// (0 = always ask). The key is kept in Credential Manager.
    #[serde(default)]
    pub unlock_memory_days: u32,
    /// Ask for a full login (email, password, 2FA) every this many days (0 = never).
    #[serde(default = "default_relogin_days")]
    pub relogin_days: u32,
    /// Lock after this many minutes without use (0 = never).
    #[serde(default = "default_auto_lock_minutes")]
    pub auto_lock_minutes: u32,
    /// Icon and color chosen per item (on this computer only).
    #[serde(default)]
    pub item_icons: std::collections::HashMap<String, crate::item_icons::CustomIcon>,
    /// Closing the window keeps the app running in the Windows tray.
    #[serde(default = "yes")]
    pub close_to_tray: bool,
    /// Seconds until the clipboard is cleared after copying (0 = don't clear).
    #[serde(default = "default_clipboard_clear_secs")]
    pub clipboard_clear_secs: u32,
    /// Opacity (%) of the floating windows (HUD, tray menu); behind them, Windows blurs.
    #[serde(default = "default_hud_opacity")]
    pub hud_opacity: u32,
    /// Lock when the window is minimized or closed to the tray.
    #[serde(default)]
    pub lock_on_minimize: bool,
    /// Lock the vault when Windows locks (`Win+L`) or sleeps.
    #[serde(default = "yes")]
    pub lock_with_windows: bool,
    /// Unlock with Windows Hello (the key is encrypted by Hello).
    #[serde(default)]
    pub windows_hello: bool,
    /// Sync on its own when the vault changes on another device.
    #[serde(default = "yes")]
    pub background_sync: bool,
    /// Global quick search (`Ctrl+Shift+Space`).
    #[serde(default = "yes")]
    pub quick_search: bool,
    /// The "still in the tray" notice has already been shown once.
    #[serde(default)]
    pub tray_hint_shown: bool,
    /// Items pinned to the sidebar's "Quick access" (ids only; nothing secret).
    #[serde(default)]
    pub pinned_items: Vec<String>,
    /// On app start, go straight to unlock (session and encrypted cache stored).
    #[serde(default = "yes")]
    pub remember_session: bool,
    /// Website icons in the list (downloaded from the server's icon service).
    #[serde(default = "yes")]
    pub site_icons: bool,
    /// SSH agent on Windows' OpenSSH pipe (off by default).
    #[serde(default)]
    pub ssh_agent: bool,
    /// The SSH agent asks for Windows Hello before each signature (when available).
    #[serde(default = "yes")]
    pub ssh_agent_hello: bool,
    /// The browser extension may ask the app for logins (off by default).
    #[serde(default)]
    pub browser_fill: bool,
    /// Password generator options (the generated password is never saved).
    #[serde(default)]
    pub generator: PasswordOptions,
    /// What the generator screen makes (password, passphrase or username).
    #[serde(default)]
    pub generator_kind: GeneratorKind,
    #[serde(default)]
    pub passphrase: PassphraseOptions,
    #[serde(default)]
    pub username_generator: UsernameOptions,
    /// Filling in other apps via the global shortcut.
    #[serde(default)]
    pub autotype: AutoTypeSettings,
    /// Global shortcuts (auto-type and quick search).
    #[serde(default)]
    pub hotkeys: crate::shortcut::Hotkeys,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AutoTypeSettings {
    pub enabled: bool,
    /// Presses Enter after the password.
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
            language: String::new(),
            generator: PasswordOptions::default(),
            generator_kind: GeneratorKind::default(),
            passphrase: PassphraseOptions::default(),
            username_generator: UsernameOptions::default(),
            remember_session: true,
            site_icons: true,
            ssh_agent: false,
            ssh_agent_hello: true,
            pinned_items: Vec::new(),
            close_to_tray: true,
            item_icons: Default::default(),
            lock_with_windows: true,
            clipboard_clear_secs: default_clipboard_clear_secs(),
            hud_opacity: default_hud_opacity(),
            lock_on_minimize: false,
            windows_hello: false,
            background_sync: true,
            quick_search: true,
            unlock_memory_days: 0,
            relogin_days: default_relogin_days(),
            auto_lock_minutes: default_auto_lock_minutes(),
            tray_hint_shown: false,
            browser_fill: false,
            autotype: AutoTypeSettings::default(),
            hotkeys: Default::default(),
        }
    }
}

fn default_relogin_days() -> u32 {
    30
}

fn default_hud_opacity() -> u32 {
    crate::quick_search::DEFAULT_OPACITY
}

fn default_clipboard_clear_secs() -> u32 {
    crate::clipboard::DEFAULT_CLEAR_SECS
}

fn default_auto_lock_minutes() -> u32 {
    5
}

fn yes() -> bool {
    true
}

/// The app used to be called "Cofre": moves the old folder (settings and remembered
/// session) to the new name, only once.
fn migrate_old_dir(new: &std::path::Path, old_name: &str) {
    let Some(old) = new.parent().map(|p| p.join(old_name)) else {
        return;
    };
    if !new.exists() && old.is_dir() {
        let _ = std::fs::rename(&old, new);
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
                    return Err(t!("settings.server_url_required").into());
                }
                // Without TLS, anyone on the path can swap the server's responses (the KDF,
                // for example) and see the login hash: http:// only on this computer.
                if let Some(rest) = url.strip_prefix("http://") {
                    if !is_loopback_authority(rest) {
                        return Err("Use https:// para servidores fora deste computador \
                                    (http:// só vale para localhost)."
                            .into());
                    }
                }
                Ok(ServerUrls::self_hosted(url))
            }
        }
    }
}

/// Is `host[:port]/…` this computer (localhost, 127.0.0.0/8 or ::1)?
fn is_loopback_authority(rest: &str) -> bool {
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let authority = authority.rsplit('@').next().unwrap_or_default();
    let host = match authority.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or_default(),
        None => authority.split(':').next().unwrap_or_default(),
    };
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// Where the preferences live. `None` = don't persist (tests).
#[derive(Clone, Default)]
pub struct SettingsStore {
    path: Option<PathBuf>,
    /// Secrets that go with this storage: the system credential store when the
    /// preferences go to disk; memory in tests.
    secrets: SecretStore,
    /// Remembered session when there is no file (tests).
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
    /// `%APPDATA%\Scout Vault\settings.json` on Windows; `~/.config/scout-vault/settings.json` elsewhere.
    pub fn default_location() -> Self {
        let dir = std::env::var_os("APPDATA")
            .map(|d| {
                let dir = PathBuf::from(d).join("Scout Vault");
                migrate_old_dir(&dir, "Cofre");
                dir
            })
            .or_else(|| {
                std::env::var_os("XDG_CONFIG_HOME")
                    .map(PathBuf::from)
                    .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
                    .map(|d| d.join("scout-vault"))
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

    /// Preferences in a separate file; secrets stay in memory (tests).
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

    /// Reads the remembered session file (or the in-memory copy, in tests).
    pub(crate) fn read_session_blob(&self) -> Option<Vec<u8>> {
        match &self.path {
            Some(p) => std::fs::read(p.with_file_name("sessao.json")).ok(),
            None => self.session_mem.lock().ok()?.clone(),
        }
    }

    /// Preferences folder (`%APPDATA%\Scout Vault`), if any.
    pub fn dir(&self) -> Option<PathBuf> {
        self.path
            .as_ref()
            .and_then(|p| p.parent().map(PathBuf::from))
    }

    /// For tests: is there a stored session (even an incomplete one)?
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
                        // Writes to a temporary file and swaps: never leaves a half-written file.
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
                // First run: creates the device identity and saves right away.
                let s = Settings::default();
                self.save(&s);
                s
            }
        }
    }

    /// Save failures don't prevent use — they only cost a new device alert later.
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
        "Scout Vault".into()
    } else {
        format!("Scout Vault ({host})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_identity_survives_restart() {
        let dir = std::env::temp_dir().join(format!("scout-vault-test-{}", std::process::id()));
        let store = SettingsStore::at(dir.join("settings.json"));
        let first = store.load();
        let second = store.load();
        assert_eq!(first.device.identifier, second.device.identifier);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Preferences saved before the language option still work (system language).
    #[test]
    fn old_settings_without_language_load() {
        let mut json = serde_json::to_value(Settings::default()).unwrap();
        json.as_object_mut().unwrap().remove("language");
        let loaded: Settings = serde_json::from_value(json).unwrap();
        assert_eq!(loaded.language, "");
    }

    #[test]
    fn self_hosted_requires_scheme() {
        let mut s = Settings {
            server: ServerChoice::SelfHosted,
            self_hosted_url: "vault.exemplo.com".into(),
            ..Settings::default()
        };
        assert!(s.server_urls().is_err());
        // http:// only on this computer.
        for (url, ok) in [
            ("http://vault.exemplo.com", false),
            ("http://192.168.0.10:8000", false),
            ("http://localhost.evil.com", false),
            ("http://127.0.0.1@evil.com", false),
            ("http://LOCALHOST:8000", true),
            ("http://localhost:8000/", true),
            ("http://127.0.0.1:9", true),
            ("http://[::1]:8000", true),
        ] {
            s.self_hosted_url = url.into();
            assert_eq!(s.server_urls().is_ok(), ok, "{url}");
        }
        s.self_hosted_url = "https://vault.exemplo.com/".into();
        assert_eq!(
            s.server_urls().unwrap().api,
            "https://vault.exemplo.com/api"
        );
    }
}
