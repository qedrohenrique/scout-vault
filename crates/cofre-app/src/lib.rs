//! Interface desktop do Cofre.
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
pub mod clipboard;
pub mod flow;
mod io;
pub mod item_list;
pub mod login;
pub mod settings;
pub mod settings_view;
mod ui;
pub mod unlock;
pub mod vault_view;

use gpui_kit::*;

/// Abre a janela principal.
pub fn run() {
    gpui_kit::application()
        .with_assets(assets::AppAssets)
        .run(|cx| {
            gpui_kit::init(cx);
            // Textos internos dos componentes (busca, menus) em português.
            gpui_kit::component::set_locale("pt-BR");
            let options = WindowOptions {
                titlebar: Some(TitlebarOptions {
                    title: Some("Cofre".into()),
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
                    app::CofreApp::new(settings::SettingsStore::default_location(), window, cx)
                })
            })
            .expect("falha ao abrir a janela");
        });
}
