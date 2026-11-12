//! Tela de configurações. Por enquanto só aparência; novas seções entram como
//! mais um `group` em [`SettingsView::render`].

use gpui_kit::assets::IconName;
use gpui_kit::component::{button::*, *};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::appearance::{self, Accent, Appearance, Corners, TextSize, ThemeChoice};
use crate::settings::SettingsStore;
use crate::ui::{self, Segment};

pub enum SettingsEvent {
    Close,
}

pub struct SettingsView {
    store: SettingsStore,
    appearance: Appearance,
}

impl EventEmitter<SettingsEvent> for SettingsView {}

impl SettingsView {
    pub fn new(store: SettingsStore) -> Self {
        let appearance = store.load().appearance;
        Self { store, appearance }
    }

    pub fn appearance(&self) -> Appearance {
        self.appearance
    }

    /// Aplica na hora e grava.
    pub fn set_appearance(&mut self, next: Appearance, window: &Window, cx: &mut Context<Self>) {
        if next == self.appearance {
            return;
        }
        self.appearance = next;
        let mut settings = self.store.load();
        settings.appearance = next;
        self.store.save(&settings);
        appearance::apply(&next, Some(window), cx);
        cx.notify();
    }

    fn update(
        &mut self,
        edit: impl FnOnce(&mut Appearance),
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let mut next = self.appearance;
        edit(&mut next);
        self.set_appearance(next, window, cx);
    }

    fn row(
        title: &'static str,
        description: &'static str,
        control: impl IntoElement,
        cx: &App,
    ) -> Div {
        h_flex()
            .gap_6()
            .px_4()
            .py_3p5()
            .justify_between()
            .child(
                v_flex()
                    .min_w_0()
                    .gap_0p5()
                    .child(div().text_sm().font_medium().child(title))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(description),
                    ),
            )
            .child(div().flex_shrink_0().child(control))
    }

    fn group(title: &'static str, rows: Vec<Div>, cx: &App) -> Div {
        let count = rows.len();
        let body = rows.into_iter().enumerate().map(|(ix, row)| {
            row.when(ix + 1 < count, |r| {
                r.border_b_1().border_color(cx.theme().border)
            })
        });
        v_flex()
            .gap_2()
            .child(ui::section_label(title, cx).px_0())
            .child(
                v_flex()
                    .rounded(cx.theme().radius_lg)
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().popover)
                    .children(body),
            )
    }

    fn render_accents(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dark = cx.theme().is_dark();
        let current = self.appearance.accent;
        let _ = window;
        h_flex().gap_2().children(Accent::ALL.map(|accent| {
            let selected = accent == current;
            div()
                .id(SharedString::from(format!("accent-{}", accent.id())))
                .test_support()
                .role(Role::Button)
                .aria_label(SharedString::from(accent.label()))
                .tooltip(move |window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(accent.label()).build(window, cx)
                })
                .size(px(26.))
                .p(px(3.))
                .rounded_full()
                .border_2()
                .border_color(if selected {
                    accent.swatch(dark)
                } else {
                    transparent_black()
                })
                .cursor_pointer()
                .child(
                    div()
                        .size_full()
                        .rounded_full()
                        .bg(accent.swatch(dark))
                        .flex()
                        .items_center()
                        .justify_center(),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.update(|a| a.accent = accent, window, cx)
                }))
        }))
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let a = self.appearance;
        let this = cx.entity().downgrade();

        let theme_control = {
            let this = this.clone();
            ui::segmented(
                "settings-theme",
                ThemeChoice::ALL
                    .iter()
                    .map(|t| Segment {
                        id: format!("theme-{}", t.id()).into(),
                        label: t.label().into(),
                        icon: Some(match t {
                            ThemeChoice::System => IconName::Monitor,
                            ThemeChoice::Light => IconName::Sun,
                            ThemeChoice::Dark => IconName::Moon,
                        }),
                        selected: *t == a.theme,
                    })
                    .collect(),
                move |ix, window, cx| {
                    this.update(cx, |v, cx| {
                        v.update(|a| a.theme = ThemeChoice::ALL[ix], window, cx)
                    })
                    .ok();
                },
                cx,
            )
        };
        let text_control = {
            let this = this.clone();
            ui::segmented(
                "settings-text-size",
                TextSize::ALL
                    .iter()
                    .map(|t| Segment {
                        id: format!("text-{}", t.id()).into(),
                        label: t.label().into(),
                        icon: None,
                        selected: *t == a.text_size,
                    })
                    .collect(),
                move |ix, window, cx| {
                    this.update(cx, |v, cx| {
                        v.update(|a| a.text_size = TextSize::ALL[ix], window, cx)
                    })
                    .ok();
                },
                cx,
            )
        };
        let corners_control = {
            let this = this.clone();
            ui::segmented(
                "settings-corners",
                Corners::ALL
                    .iter()
                    .map(|c| Segment {
                        id: format!("corners-{}", c.id()).into(),
                        label: c.label().into(),
                        icon: None,
                        selected: *c == a.corners,
                    })
                    .collect(),
                move |ix, window, cx| {
                    this.update(cx, |v, cx| {
                        v.update(|a| a.corners = Corners::ALL[ix], window, cx)
                    })
                    .ok();
                },
                cx,
            )
        };

        let appearance_group = Self::group(
            "APARÊNCIA",
            vec![
                Self::row(
                    "Tema",
                    "Claro, escuro ou igual ao do sistema.",
                    theme_control,
                    cx,
                ),
                Self::row(
                    "Cor de destaque",
                    "Usada na seleção, no foco e nos botões principais.",
                    self.render_accents(window, cx),
                    cx,
                ),
                Self::row(
                    "Tamanho do texto",
                    "Vale para todas as telas.",
                    text_control,
                    cx,
                ),
                Self::row(
                    "Cantos",
                    "Arredondamento de botões e cartões.",
                    corners_control,
                    cx,
                ),
            ],
            cx,
        );

        let header = h_flex()
            .justify_between()
            .child(
                v_flex()
                    .gap_1()
                    .child(div().text_2xl().font_semibold().child("Configurações"))
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("As mudanças valem na hora e ficam salvas neste computador."),
                    ),
            )
            .child(
                Button::new("settings-close")
                    .ghost()
                    .small()
                    .icon(IconName::Close)
                    .tooltip("Fechar (Esc)")
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::Close))),
            );

        div()
            .id("settings")
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
                    .child(appearance_group),
            )
    }
}
