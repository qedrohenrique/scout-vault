//! Right-click menu of the tray icon, drawn by the app itself.
//!
//! The native Windows menu doesn't accept the app's colors or icons; this one is a
//! small frameless window, opened at the click point, with Scout Vault's theme and
//! accent color. It closes when something is chosen, with `Esc` or when clicking outside.
//! Each choice becomes a [`Command`] on the same channel as the tray.

use gpui_kit::assets::IconName;
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;
use tokio::sync::mpsc::UnboundedSender;

use crate::tray::{Command, MenuState, PinnedEntry};
use crate::vault_view::CopyShortcut;
use crate::{appearance, motion, ui};

/// Card width.
pub const WIDTH: f32 = 292.;
/// Transparent margin around the card, for the shadow.
pub const MARGIN: f32 = 0.;
/// Card corner radius.
/// Same as the corners Windows 11 cuts on the window (`DWMWCP_ROUND`).
const CARD_RADIUS: f32 = 8.;
const HEADER: f32 = 60.;
const LABEL: f32 = 26.;
const PINNED: f32 = 40.;
const HINT: f32 = 36.;
const ACTION: f32 = 34.;
const SEP: f32 = 9.;
const PAD: f32 = 6.;
/// More than this in quick access scrolls inside the menu (the app pins up to 10).
const MAX_PINNED_VISIBLE: usize = 6;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tone {
    Normal,
    Accent,
    Danger,
}

struct Action {
    id: &'static str,
    icon: IconName,
    label: &'static str,
    hint: Option<String>,
    command: Command,
    tone: Tone,
}

enum Row {
    Label(&'static str),
    Pinned(Vec<PinnedEntry>),
    Hint(&'static str),
    Action(Action),
    Separator,
}

impl Row {
    fn height(&self) -> f32 {
        match self {
            Row::Label(_) => LABEL,
            Row::Pinned(items) => PINNED * items.len().min(MAX_PINNED_VISIBLE) as f32,
            Row::Hint(_) => HINT,
            Row::Action(_) => ACTION,
            Row::Separator => SEP,
        }
    }
}

/// `clear`: clipboard clearing time, for the generator hint.
fn rows(state: &MenuState, clear: Option<std::time::Duration>) -> Vec<Row> {
    let generate = Row::Action(Action {
        id: "tray-menu-generate",
        icon: IconName::WandSparkles,
        label: t!("tray.generate"),
        hint: clear.map(|d| t!("tray.clears_in", n = d.as_secs())),
        command: Command::GeneratePassword,
        tone: Tone::Accent,
    });
    let quit = Row::Action(Action {
        id: "tray-menu-quit",
        icon: IconName::Power,
        label: t!("tray.quit"),
        hint: None,
        command: Command::Quit,
        tone: Tone::Danger,
    });
    if !state.unlocked {
        return vec![
            Row::Action(Action {
                id: "tray-menu-unlock",
                icon: IconName::LockOpen,
                label: t!("tray.unlock"),
                hint: None,
                command: Command::Show,
                tone: Tone::Accent,
            }),
            generate,
            Row::Separator,
            quit,
        ];
    }
    let mut out = vec![Row::Label(t!("tray.quick_access"))];
    if state.pinned.is_empty() {
        out.push(Row::Hint(t!("tray.pin_hint")));
    } else {
        out.push(Row::Pinned(state.pinned.clone()));
    }
    out.extend([
        Row::Separator,
        Row::Action(Action {
            id: "tray-menu-open",
            icon: IconName::AppWindow,
            label: t!("tray.open"),
            hint: None,
            command: Command::Show,
            tone: Tone::Normal,
        }),
        generate,
        Row::Action(Action {
            id: "tray-menu-sync",
            icon: IconName::RefreshCw,
            label: t!("tray.sync_now"),
            hint: None,
            command: Command::Sync,
            tone: Tone::Normal,
        }),
        Row::Separator,
        Row::Action(Action {
            id: "tray-menu-lock",
            icon: IconName::Lock,
            label: t!("tray.lock"),
            hint: None,
            command: Command::Lock,
            tone: Tone::Normal,
        }),
        quit,
    ]);
    out
}

/// Size of the menu window (card + shadow margin) for this state.
pub fn window_size(state: &MenuState) -> gpui_kit::Size<Pixels> {
    let body: f32 = rows(state, None).iter().map(Row::height).sum();
    // +2: top and bottom border of the card.
    let height = HEADER + PAD * 2. + body + 2. + MARGIN * 2.;
    size(px(WIDTH + MARGIN * 2.), px(height))
}

pub struct TrayMenu {
    state: MenuState,
    tx: UnboundedSender<Command>,
    focus: FocusHandle,
    /// Has gained focus at some point (only then does losing focus close the menu).
    activated: bool,
    _activation: Subscription,
}

impl TrayMenu {
    pub fn new(
        state: MenuState,
        tx: UnboundedSender<Command>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        // Clicking outside takes focus away from the menu: it closes, like a real menu.
        let activation = cx.observe_window_activation(window, |this, window, _| {
            if window.is_window_active() {
                this.activated = true;
            } else if this.activated {
                window.remove_window();
            }
        });
        // If Windows doesn't let the menu take focus, it doesn't get stuck on screen.
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(10))
                .await;
            this.update_in(cx, |this, window, _| {
                if !this.activated {
                    window.remove_window();
                }
            })
            .ok();
        })
        .detach();
        Self {
            state,
            tx,
            focus,
            activated: false,
            _activation: activation,
        }
    }

    pub fn state(&self) -> &MenuState {
        &self.state
    }

    fn choose(&mut self, command: Command, window: &mut Window) {
        let _ = self.tx.send(command);
        window.remove_window();
    }

    fn header(&self, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let accent = appearance::accent(cx);
        let (status, dot) = if self.state.unlocked {
            let n = self.state.pinned.len();
            let pins = match n {
                0 => t!("tray.none_pinned").to_owned(),
                1 => t!("tray.one_pinned").to_owned(),
                n => t!("tray.n_pinned", n = n),
            };
            (t!("tray.vault_open", pins = pins), theme.success)
        } else {
            (t!("app.vault_locked").to_owned(), theme.warning)
        };
        let unlocked = self.state.unlocked;
        h_flex()
            .h(px(HEADER))
            .flex_none()
            .px_3()
            .gap_3()
            .border_b_1()
            .border_color(theme.border)
            // The card clips as a rectangle, not at the rounded corner: the
            // gradient needs the same corners (card radius minus the border).
            .rounded_t(px(CARD_RADIUS - 1.))
            .bg(linear_gradient(
                90.,
                linear_color_stop(accent.opacity(0.18), 0.),
                linear_color_stop(accent.opacity(0.02), 1.),
            ))
            .child(ui::brand_mark(px(34.), cx))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_0p5()
                    .child(div().text_sm().font_semibold().child("Scout Vault"))
                    .child(
                        h_flex()
                            .gap_1p5()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child({
                                let d = div().size(px(7.)).rounded_full().bg(dot);
                                // Unlocked: the dot "breathes" slowly.
                                if unlocked {
                                    d.with_animation(
                                        "tray-menu-dot",
                                        Animation::new(std::time::Duration::from_millis(1600))
                                            .repeat()
                                            .with_easing(pulsating_between(0.45, 1.)),
                                        |d, t| d.opacity(t),
                                    )
                                    .into_any_element()
                                } else {
                                    d.into_any_element()
                                }
                            })
                            .child(div().truncate().child(status)),
                    ),
            )
            .child(
                Icon::new(if unlocked {
                    IconName::LockOpen
                } else {
                    IconName::Lock
                })
                .small()
                .text_color(if unlocked { accent } else { theme.warning }),
            )
    }

    fn action_row(&self, action: Action, cx: &mut Context<Self>) -> impl IntoElement {
        let Action {
            id,
            icon,
            label,
            hint,
            command,
            tone,
        } = action;
        let theme = cx.theme();
        let accent = appearance::accent(cx);
        let (icon_color, hover_bg, hover_fg) = match tone {
            Tone::Normal => (
                theme.muted_foreground,
                accent.opacity(0.12),
                theme.foreground,
            ),
            Tone::Accent => (accent, accent.opacity(0.14), accent),
            Tone::Danger => (
                theme.muted_foreground,
                theme.danger.opacity(0.12),
                theme.danger,
            ),
        };
        let muted = theme.muted_foreground;
        h_flex()
            .id(id)
            .test_support()
            .role(Role::MenuItem)
            .aria_label(SharedString::from(label))
            .h(px(ACTION))
            .flex_none()
            .px_2p5()
            .gap_2p5()
            .rounded(px(8.))
            .text_sm()
            .cursor_pointer()
            .hover(move |s| s.bg(hover_bg).text_color(hover_fg))
            .active(|s| s.opacity(0.75))
            .child(
                div()
                    .size(px(22.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(icon_color)
                    .child(Icon::new(icon).small()),
            )
            .child(div().flex_1().min_w_0().truncate().child(label))
            .when_some(hint, |el, hint| {
                el.child(div().text_xs().text_color(muted).child(hint))
            })
            .on_click(cx.listener(move |this, _, window, _| this.choose(command.clone(), window)))
    }

    fn pinned_rows(&self, items: &[PinnedEntry], cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let accent = appearance::accent(cx);
        let (muted, hover) = (theme.muted_foreground, accent.opacity(0.12));
        let mut list = v_flex()
            .id("tray-menu-pinned")
            .h(px(PINNED * items.len().min(MAX_PINNED_VISIBLE) as f32))
            .flex_none()
            .overflow_y_scroll();
        for (order, item) in items.iter().enumerate() {
            // Clicking the row copies the password (or the username); the buttons, each field.
            let primary = if item.has_password {
                Some(CopyShortcut::Password)
            } else if item.has_username {
                Some(CopyShortcut::Username)
            } else if item.has_totp {
                Some(CopyShortcut::TotpCode)
            } else {
                None
            };
            let mut buttons = h_flex().gap_0p5();
            for (has, field, icon, label) in [
                (
                    item.has_username,
                    "user",
                    IconName::User,
                    t!("tray.copy_username"),
                ),
                (
                    item.has_password,
                    "pass",
                    IconName::KeyRound,
                    t!("tray.copy_password"),
                ),
                (item.has_totp, "code", IconName::Timer, t!("tray.copy_code")),
            ] {
                if !has {
                    continue;
                }
                let what = match field {
                    "user" => CopyShortcut::Username,
                    "pass" => CopyShortcut::Password,
                    _ => CopyShortcut::TotpCode,
                };
                let item_id = item.id.clone();
                buttons = buttons.child(
                    div()
                        .id(SharedString::from(format!("tray-pin-{}-{field}", item.id)))
                        .test_support()
                        .role(Role::Button)
                        .aria_label(SharedString::from(label))
                        .size(px(26.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(7.))
                        .text_color(muted)
                        .cursor_pointer()
                        .hover(move |s| s.bg(accent.opacity(0.18)).text_color(accent))
                        .active(|s| s.opacity(0.7))
                        .child(Icon::new(icon).xsmall())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.choose(
                                Command::Copy {
                                    item_id: item_id.clone(),
                                    what,
                                },
                                window,
                            )
                        })),
                );
            }
            let item_id = item.id.clone();
            let row = h_flex()
                .id(SharedString::from(format!("tray-pin-{}", item.id)))
                .test_support()
                .role(Role::MenuItem)
                .aria_label(SharedString::from(item.name.clone()))
                .h(px(PINNED))
                .flex_none()
                .w_full()
                .px_2()
                .gap_2p5()
                .rounded(px(8.))
                .text_sm()
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .active(|s| s.opacity(0.8))
                .child(ui::icon_chip(
                    item.icon.unwrap_or(IconName::Globe.into()),
                    px(26.),
                    true,
                    cx,
                ))
                .child(div().flex_1().min_w_0().truncate().child(item.name.clone()))
                .child(buttons)
                .when_some(primary, |el, what| {
                    el.on_click(cx.listener(move |this, _, window, _| {
                        this.choose(
                            Command::Copy {
                                item_id: item_id.clone(),
                                what,
                            },
                            window,
                        )
                    }))
                });
            list = list.child(motion::cascade(
                div().w_full().child(row),
                motion::key("tray-pin-entra", &item.id),
                order,
            ));
        }
        list
    }
}

impl Focusable for TrayMenu {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for TrayMenu {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (border, popover, fg, muted) = (
            theme.border,
            theme.popover,
            theme.popover_foreground,
            theme.muted_foreground,
        );
        let mut body = v_flex().py(px(PAD)).px_1p5();
        let mut order = 0;
        for row in rows(&self.state, crate::clipboard::clear_after(cx)) {
            let el: AnyElement = match row {
                Row::Label(text) => div()
                    .h(px(LABEL))
                    .flex_none()
                    .px_2p5()
                    .pt_2()
                    .text_xs()
                    .font_semibold()
                    .text_color(muted)
                    .child(text)
                    .into_any_element(),
                Row::Hint(text) => h_flex()
                    .h(px(HINT))
                    .flex_none()
                    .px_2p5()
                    .gap_2()
                    .text_xs()
                    .text_color(muted)
                    .child(Icon::new(IconName::Pin).xsmall())
                    .child(text)
                    .into_any_element(),
                Row::Pinned(items) => self.pinned_rows(&items, cx).into_any_element(),
                Row::Separator => div()
                    .h(px(SEP))
                    .flex_none()
                    .flex()
                    .items_center()
                    .px_2()
                    .child(div().h_px().w_full().bg(border))
                    .into_any_element(),
                Row::Action(action) => {
                    order += 1;
                    let id = action.id;
                    motion::cascade(
                        div().w_full().child(self.action_row(action, cx)),
                        motion::key("tray-acao-entra", id),
                        order,
                    )
                    .into_any_element()
                }
            };
            body = body.child(el);
        }
        let card = v_flex()
            .size_full()
            .rounded(px(CARD_RADIUS))
            .border_1()
            .border_color(border)
            .bg(popover.opacity(crate::quick_search::opacity(cx)))
            .text_color(fg)
            .overflow_hidden()
            .child(self.header(cx))
            .child(body);
        div()
            .id("tray-menu")
            .size_full()
            .p(px(MARGIN))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|_, e: &KeyDownEvent, window, _| {
                if e.keystroke.key == "escape" {
                    window.remove_window();
                }
            }))
            .child(card)
    }
}
