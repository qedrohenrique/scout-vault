//! Interface desktop do Scout Vault.
//!
//! - [`app`]        — raiz da janela, troca de telas e bloqueio automático
//! - [`login`]      — servidor, e-mail, senha, 2FA, verificação de dispositivo
//! - [`unlock`]     — desbloqueio offline com a senha mestra
//! - [`vault_view`] — filtros, lista, detalhe, copiar/revelar
//! - [`item_list`]  — dados da lista: filtro e busca
//! - [`flow`]       — fluxos de login/sync sem UI
//! - [`settings`]   — preferências não sensíveis em disco
//! - [`settings_view`] — tela de configurações (aparência)
//! - [`appearance`] — temas claro/escuro, cor de destaque, texto e cantos

pub mod app;
pub mod appearance;
mod assets;
mod attachments;
pub mod autotype;
pub mod autotype_view;
pub mod backdrop;
pub mod browser;
pub mod clipboard;
pub mod flow;
pub mod generator_view;
mod io;
pub mod item_editor;
pub mod item_list;
pub mod login;
pub mod motion;
pub mod saved_session;
pub mod secrets;
pub mod settings;
pub mod settings_view;
pub mod sso;
pub mod toast;
pub mod tray;
mod ui;
pub mod unlock;
pub mod vault_view;
pub mod webauthn;

use gpui_kit::*;

/// Abre a janela principal.
pub fn run() {
    // Já está rodando (talvez escondido na bandeja)? Só pede para aparecer.
    if tray::wake_existing_instance() {
        return;
    }
    gpui_kit::application()
        .with_assets(assets::AppAssets)
        .run(|cx| {
            gpui_kit::init(cx);
            // Textos internos dos componentes (busca, menus) em português.
            gpui_kit::component::set_locale("pt-BR");
            // Senhas copiadas ficam fora do histórico (Win+V) e da nuvem.
            clipboard::use_native_clipboard();
            // Sobras de anexos abertos numa execução anterior (ex.: o app caiu).
            attachments::cleanup_temp();
            attachments::enable_system_open();
            browser::enable_system();
            // Atalho global de preenchimento em outros apps (Ctrl+Alt+A).
            autotype::enable_system_hotkey();
            // Fechar a janela deixa o app na bandeja do Windows.
            tray::enable_system();
            let options = WindowOptions {
                titlebar: Some(TitlebarOptions {
                    title: Some("Scout Vault".into()),
                    ..Default::default()
                }),
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1100.), px(720.)),
                    cx,
                ))),
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| {
                    app::ScoutVaultApp::new(settings::SettingsStore::default_location(), window, cx)
                })
            })
            .expect("falha ao abrir a janela");
        });
}
