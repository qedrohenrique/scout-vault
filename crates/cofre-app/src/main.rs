// Esconde o console no Windows em builds de release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use gpui_kit::component::button::*;
use gpui_kit::component::*;
use gpui_kit::*;

/// Tela provisória. Na fase 2 vira: Login → Desbloqueio → Cofre.
struct CofreApp;

impl Render for CofreApp {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .v_flex()
            .gap_2()
            .size_full()
            .items_center()
            .justify_center()
            .child("Cofre — cliente nativo para servidores compatíveis com Bitwarden")
            .child(
                Button::new("start")
                    .primary()
                    .label("Em construção")
                    .on_click(|_, _, _| println!("ver docs/ROADMAP.md")),
            )
    }
}

fn main() {
    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);

        gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| CofreApp))
            .expect("falha ao abrir a janela");
    });
}
