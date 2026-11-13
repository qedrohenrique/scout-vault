//! Gerador de senhas (`Ctrl+G`), aberto por cima da tela atual como as configurações.
//! As opções ficam salvas em `settings.json`; a senha gerada, só em memória.

use cofre_core::generator::{self, PasswordOptions};
use gpui_kit::assets::IconName;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::{button::*, *};
use gpui_kit::prelude::*;
use gpui_kit::*;
use zeroize::Zeroizing;

use crate::settings::SettingsStore;
use crate::ui::{self, Segment};
use crate::{appearance, clipboard};

pub const LENGTHS: [usize; 6] = [12, 16, 20, 24, 32, 64];

pub enum GeneratorEvent {
    Close,
}

pub struct GeneratorView {
    store: SettingsStore,
    options: PasswordOptions,
    password: Zeroizing<String>,
}

impl EventEmitter<GeneratorEvent> for GeneratorView {}

impl GeneratorView {
    pub fn new(store: SettingsStore) -> Self {
        let options = store.load().generator;
        Self {
            store,
            password: generator::password(&options),
            options,
        }
    }

    pub fn password(&self) -> &str {
        &self.password
    }

    pub fn options(&self) -> PasswordOptions {
        self.options
    }

    pub fn regenerate(&mut self, cx: &mut Context<Self>) {
        self.password = generator::password(&self.options);
        cx.notify();
    }

    /// Troca as opções, gera de novo e grava.
    pub fn set_options(&mut self, next: PasswordOptions, cx: &mut Context<Self>) {
        self.options = next;
        let mut settings = self.store.load();
        settings.generator = next;
        self.store.save(&settings);
        self.regenerate(cx);
    }

    fn edit(&mut self, f: impl FnOnce(&mut PasswordOptions), cx: &mut Context<Self>) {
        let mut next = self.options;
        f(&mut next);
        self.set_options(next, cx);
    }

    pub fn copy(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        clipboard::copy_and_clear_later(&self.password, cx);
        window.push_notification(
            SharedString::from(format!(
                "Senha copiada. A área de transferência será limpa em {} s.",
                clipboard::CLEAR_AFTER.as_secs()
            )),
            cx,
        );
    }

    fn check(
        &self,
        id: &'static str,
        label: &'static str,
        value: bool,
        set: fn(&mut PasswordOptions, bool),
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        Checkbox::new(id)
            .label(label)
            .checked(value)
            .on_change(cx.listener(move |this, v: &bool, _, cx| {
                let v = *v;
                this.edit(|o| set(o, v), cx)
            }))
    }
}

impl Render for GeneratorView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let o = self.options;
        let bits = o.entropy_bits();
        let this = cx.entity().downgrade();

        let header = h_flex()
            .justify_between()
            .child(
                v_flex()
                    .gap_1()
                    .child(div().text_2xl().font_semibold().child("Gerador de senhas"))
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("Aleatoriedade do sistema; nada é enviado ao servidor."),
                    ),
            )
            .child(
                Button::new("generator-close")
                    .ghost()
                    .small()
                    .icon(IconName::Close)
                    .tooltip("Fechar (Esc)")
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(GeneratorEvent::Close))),
            );

        let output = v_flex()
            .gap_3()
            .p_5()
            .rounded(cx.theme().radius_lg)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .child(
                div()
                    .id("generator-output")
                    .test_support()
                    .aria_label(SharedString::from(self.password.to_string()))
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_xl()
                    .whitespace_normal()
                    .child(self.password.to_string()),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .id("generator-strength")
                            .test_support()
                            .aria_label(SharedString::from(generator::strength_label(bits)))
                            .flex_1()
                            .text_xs()
                            .text_color(appearance::accent(cx))
                            .child(format!(
                                "{} · ~{} bits",
                                generator::strength_label(bits),
                                bits.round() as u32
                            )),
                    )
                    .child(
                        Button::new("generator-refresh")
                            .outline()
                            .small()
                            .icon(IconName::RefreshCw)
                            .label("Gerar outra")
                            .on_click(cx.listener(|this, _, _, cx| this.regenerate(cx))),
                    )
                    .child(
                        Button::new("generator-copy")
                            .primary()
                            .small()
                            .icon(IconName::Copy)
                            .label("Copiar")
                            .on_click(cx.listener(|this, _, window, cx| this.copy(window, cx))),
                    ),
            );

        let length = ui::segmented(
            "generator-length",
            LENGTHS
                .iter()
                .map(|n| Segment {
                    id: format!("length-{n}").into(),
                    label: n.to_string().into(),
                    icon: None,
                    selected: *n == o.length,
                })
                .collect(),
            move |ix, _, cx| {
                this.update(cx, |v, cx| v.edit(|o| o.length = LENGTHS[ix], cx))
                    .ok();
            },
            cx,
        );

        let options = v_flex()
            .gap_2()
            .child(ui::section_label("OPÇÕES", cx).px_0())
            .child(
                v_flex()
                    .gap_4()
                    .p_4()
                    .rounded(cx.theme().radius_lg)
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().popover)
                    .child(
                        h_flex()
                            .justify_between()
                            .gap_4()
                            .child(div().text_sm().font_medium().child("Tamanho"))
                            .child(length),
                    )
                    .child(
                        h_flex()
                            .flex_wrap()
                            .gap_x_6()
                            .gap_y_3()
                            .child(self.check(
                                "gen-upper",
                                "A–Z",
                                o.uppercase,
                                |o, v| o.uppercase = v,
                                cx,
                            ))
                            .child(self.check(
                                "gen-lower",
                                "a–z",
                                o.lowercase,
                                |o, v| o.lowercase = v,
                                cx,
                            ))
                            .child(self.check(
                                "gen-digits",
                                "0–9",
                                o.digits,
                                |o, v| o.digits = v,
                                cx,
                            ))
                            .child(self.check(
                                "gen-symbols",
                                "!@#$%^&*",
                                o.symbols,
                                |o, v| o.symbols = v,
                                cx,
                            ))
                            .child(self.check(
                                "gen-ambiguous",
                                "Evitar caracteres parecidos (I l 1 O 0)",
                                o.avoid_ambiguous,
                                |o, v| o.avoid_ambiguous = v,
                                cx,
                            )),
                    ),
            );

        div()
            .id("generator")
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .overflow_y_scroll()
            .child(
                v_flex()
                    .w_full()
                    .max_w(px(680.))
                    .mx_auto()
                    .px_8()
                    .py_10()
                    .gap_8()
                    .child(header)
                    .child(output)
                    .child(options),
            )
    }
}
