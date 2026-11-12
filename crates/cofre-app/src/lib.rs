//! Interface desktop do Cofre.
//!
//! - [`app`]        — raiz da janela, troca de telas e bloqueio automático
//! - [`login`]      — servidor, e-mail, senha, 2FA, verificação de dispositivo
//! - [`unlock`]     — desbloqueio offline com a senha mestra
//! - [`vault_view`] — filtros, lista, detalhe, copiar/revelar
//! - [`item_list`]  — dados da lista: filtro e busca
//! - [`flow`]       — fluxos de login/sync sem UI
//! - [`settings`]   — preferências não sensíveis em disco

pub mod app;
pub mod clipboard;
pub mod flow;
mod io;
pub mod item_list;
pub mod login;
pub mod settings;
pub mod unlock;
pub mod vault_view;

use gpui_kit::*;

/// Abre a janela principal.
pub fn run() {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(|cx| {
            gpui_kit::init(cx);
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
