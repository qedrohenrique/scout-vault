//! In-app updates: looks for a newer release on GitHub (on start and twice a day,
//! if the setting is on, or when asked), and updates by downloading the installer,
//! checking its SHA-256 and running it silently. The installer closes this app,
//! replaces it and opens it again (`/RELAUNCH=1`).
//!
//! When the app didn't come from the installer (the zip), "Update" opens the
//! release page instead: there is nothing installed to replace.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use gpui_kit::{App, Global};
use scout_vault_core::update::{self, Release, Version};

/// Downloads only happen in the real app (the UI tests stay offline).
static NETWORK: AtomicBool = AtomicBool::new(false);

/// In the real app: may ask GitHub.
pub fn enable_network() {
    NETWORK.store(true, Ordering::Relaxed);
}

/// Is this the real app (allowed to ask GitHub)?
pub fn network_enabled() -> bool {
    NETWORK.load(Ordering::Relaxed)
}

/// How often the automatic check runs (when the app stays open).
pub const CHECK_EVERY: std::time::Duration = std::time::Duration::from_secs(12 * 60 * 60);

/// Where an update stands.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum State {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available(Box<Release>),
    Downloading(Version),
    /// The installer is running: the app will close and come back.
    Installing(Version),
    Failed(String),
}

#[derive(Default)]
struct Updater {
    state: State,
}

impl Global for Updater {}

fn updater(cx: &mut App) -> &mut Updater {
    if !cx.has_global::<Updater>() {
        cx.set_global(Updater::default());
    }
    cx.global_mut::<Updater>()
}

pub fn current() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).expect("the crate version is X.Y.Z")
}

pub fn state(cx: &App) -> State {
    cx.try_global::<Updater>()
        .map(|u| u.state.clone())
        .unwrap_or_default()
}

/// A newer version is waiting.
pub fn available(cx: &App) -> Option<Version> {
    match state(cx) {
        State::Available(r) => Some(r.version),
        _ => None,
    }
}

/// Sets the state (tests and the flow below).
pub fn set_state(state: State, cx: &mut App) {
    updater(cx).state = state;
    cx.refresh_windows();
}

/// Asks GitHub for a newer version. `quiet`: the automatic check (no message
/// when there's nothing new or the network fails).
pub fn check(quiet: bool, cx: &mut App) {
    if matches!(
        state(cx),
        State::Checking | State::Downloading(_) | State::Installing(_)
    ) {
        return;
    }
    if !NETWORK.load(Ordering::Relaxed) {
        if !quiet {
            set_state(State::UpToDate, cx);
        }
        return;
    }
    set_state(State::Checking, cx);
    cx.spawn(async move |cx| {
        let result = crate::io::run(async move { update::check(&current()).await }).await;
        cx.update(|cx| {
            let next = match result {
                Ok(Some(release)) => {
                    if quiet {
                        crate::toast::info(t!("update.available_toast", v = release.version), cx);
                    }
                    State::Available(Box::new(release))
                }
                Ok(None) => State::UpToDate,
                Err(_) if quiet => State::Idle,
                Err(e) => State::Failed(t!("update.check_failed", e = e)),
            };
            set_state(next, cx);
        });
    })
    .detach();
}

/// Updates to the version found: installer (installed app) or the release page (zip).
pub fn install(cx: &mut App) {
    let State::Available(release) = state(cx) else {
        return;
    };
    if !is_installed() {
        if let Err(e) = crate::sso::open_browser(&release.page) {
            crate::toast::error(e, cx);
        }
        return;
    }
    let version = release.version.clone();
    set_state(State::Downloading(version.clone()), cx);
    cx.spawn(async move |cx| {
        let release = (*release).clone();
        let result = crate::io::run(async move {
            let (name, bytes) = update::download_installer(&release).await?;
            let path = save_installer(&name, &bytes)
                .map_err(|e| update::UpdateError::Network(e.to_string()))?;
            Ok::<_, update::UpdateError>(path)
        })
        .await;
        let started = cx.update(|cx| match result.map(|path| run_installer(&path)) {
            Ok(Ok(())) => {
                set_state(State::Installing(version), cx);
                true
            }
            Ok(Err(e)) => {
                set_state(State::Failed(t!("update.run_failed", e = e)), cx);
                false
            }
            Err(update::UpdateError::Checksum) => {
                set_state(State::Failed(t!("update.checksum_failed").into()), cx);
                false
            }
            Err(e) => {
                set_state(State::Failed(t!("update.download_failed", e = e)), cx);
                false
            }
        });
        // The app closes itself (the installer's `--quit` only reaches it when the tray
        // icon is on), after a moment to show "Installing…".
        if started {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(1500))
                .await;
            cx.update(|cx| cx.quit());
        }
    })
    .detach();
}

/// The installer goes to a folder of its own in the temp directory.
fn save_installer(name: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
    let dir = std::env::temp_dir().join("scout-vault-update");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(name);
    std::fs::write(&path, bytes)?;
    Ok(path)
}

/// Runs the installer silently; it closes this app (`--quit`) and opens it again.
fn run_installer(path: &Path) -> Result<(), String> {
    std::process::Command::new(path)
        .args(["/SILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/RELAUNCH=1"])
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Did this executable come from the installer? (It is in the folder the installer
/// registered in "Installed apps".)
pub fn is_installed() -> bool {
    let Some(location) = platform::install_location() else {
        return false;
    };
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let normalize = |p: &Path| {
        p.to_string_lossy()
            .trim_end_matches(['\\', '/'])
            .to_lowercase()
    };
    exe.parent()
        .is_some_and(|dir| normalize(dir) == normalize(Path::new(&location)))
}

#[cfg(windows)]
mod platform {
    /// The installer's AppId (see `installer/scout-vault.iss`).
    const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\{1DDFCFAE-DDB3-451C-B403-F457C609A23A}_is1";

    pub fn install_location() -> Option<String> {
        crate::autostart::read_registry_string(UNINSTALL_KEY, "InstallLocation")
    }
}

#[cfg(not(windows))]
mod platform {
    pub fn install_location() -> Option<String> {
        None
    }
}
