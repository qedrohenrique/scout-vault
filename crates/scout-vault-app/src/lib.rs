//! Scout Vault desktop interface.
//!
//! - [`app`]        — window root, screen switching and automatic lock
//! - [`login`]      — server, email, password, 2FA, device verification
//! - [`unlock`]     — offline unlock with the master password
//! - [`vault_view`] — filters, list, detail, copy/reveal
//! - [`item_list`]  — list data: filtering and search
//! - [`flow`]       — login/sync flows without UI
//! - [`settings`]   — non-sensitive preferences on disk
//! - [`settings_view`] — settings screen (appearance)
//! - [`appearance`] — light/dark themes, accent color, text and corners
//! - [`i18n`]       — interface languages (`t!("key")`, catalogs in `locales/`)

// First: makes the `t!` macro visible in the modules below.
#[macro_use]
pub mod i18n;

pub mod app;
pub mod appearance;
mod assets;
mod attachments;
pub mod autostart;
pub mod autotype;
pub mod backdrop;
pub mod browser;
pub mod clipboard;
pub mod colors;
pub mod dnd;
pub mod export_view;
pub mod flow;
pub mod generator_view;
pub mod health_view;
pub mod hello;
pub mod import_view;
mod io;
pub mod item_editor;
pub mod item_icons;
pub mod item_list;
pub mod login;
pub mod motion;
pub mod profile_photo;
pub mod profile_view;
pub mod quick_search;
pub mod quick_unlock;
pub mod saved_session;
pub mod secrets;
pub mod settings;
pub mod settings_view;
pub mod shortcut;
pub mod site_icons;
pub mod ssh_agent;
pub mod sso;
pub mod theme_fx;
pub mod toast;
pub mod tray;
pub mod tray_menu;
mod ui;
pub mod unlock;
pub mod updater;
pub mod vault_view;
pub mod webauthn;
pub mod windows_events;

use gpui_kit::*;

/// Opens the main window.
pub fn run() {
    let args: Vec<String> = std::env::args().collect();
    // `--quit` (installer, before updating or uninstalling): closes the running app
    // and waits for it to leave, so its files can be replaced.
    if args.iter().any(|a| a == "--quit") {
        tray::quit_existing_instance(std::time::Duration::from_secs(10));
        return;
    }
    // `--tray` (start with Windows): opens hidden in the tray.
    let hidden = args.iter().any(|a| a == autostart::TRAY_ARG);
    if hidden && tray::existing_instance() {
        return;
    }
    // Already running (maybe hidden in the tray)? Just ask it to show up.
    if tray::wake_existing_instance() {
        return;
    }
    autostart::enable_system();
    autostart::refresh();
    gpui_kit::application()
        .with_assets(assets::AppAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            // Language chosen in Settings or, by default, the Windows one
            // (also applies to the components' internal texts).
            let language = settings::SettingsStore::default_location().load().language;
            i18n::set(i18n::Language::from_setting(&language));
            // Copied passwords stay out of the history (Win+V) and the cloud.
            clipboard::use_native_clipboard();
            // Leftovers of attachments opened in a previous run (e.g. the app crashed).
            attachments::cleanup_temp();
            attachments::enable_system_open();
            browser::enable_system();
            // Global shortcut for filling in other apps (Ctrl+Alt+A).
            autotype::enable_system_hotkey();
            // Closing the window leaves the app in the Windows tray.
            tray::enable_system();
            site_icons::enable_network();
            updater::enable_network();
            // Unlock with Windows Hello (if set up in Windows).
            hello::install_system(cx);
            // The app draws its own title bar (`app.rs`), in the screen's colors.
            let base = gpui_kit::component::TitleBar::window_options();
            let options = WindowOptions {
                titlebar: Some(TitlebarOptions {
                    title: Some("Scout Vault".into()),
                    ..gpui_kit::component::TitleBar::title_bar_options()
                }),
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1100.), px(720.)),
                    cx,
                ))),
                ..base
            };
            let handle = gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| {
                    app::ScoutVaultApp::new(settings::SettingsStore::default_location(), window, cx)
                })
            })
            .expect("falha ao abrir a janela");
            if hidden {
                let _ = handle
                    .0
                    .update(cx, |_, window, _| tray::hide_window(window));
            }
        });
}
