//! Global quick search (`Ctrl+Shift+Space`): a small window on top of everything
//! to find an item and **fill** it into the app that had focus, **copy**
//! username/password/code or **open** it in Scout Vault — without switching windows.
//!
//! The window only receives names and usernames (nothing secret); each action becomes a
//! [`Command`] on the same channel as the tray, and the app is the one that copies or types
//! (with the master password prompt, if the item requires it).

use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{button::*, *};
use gpui_kit::prelude::*;
use gpui_kit::*;
use scout_vault_core::vault::{Item, ItemKind};
use tokio::sync::mpsc::UnboundedSender;

use crate::appearance;
use crate::item_list::kind_icon;
use crate::tray::Command;
use crate::vault_view::{CopyShortcut, VaultView};

/// Global shortcut, to show in the interface.
/// Window size (with the transparent margin for the shadow).
pub const WIDTH: f32 = 580.;
pub const HEIGHT: f32 = 452.;
/// Height of the HUD while it asks for the vault password.
pub const LOCKED_HEIGHT: f32 = 92.;
/// Height of the HUD while it shows the login.
pub const LOGIN_HEIGHT: f32 = 440.;
/// Height of the HUD with an auto-type message.
pub const NOTICE_HEIGHT: f32 = 132.;
pub const MARGIN: f32 = 0.;
/// Default opacity (%) of the floating windows: see-through enough to keep the
/// context behind, solid enough to read comfortably.
pub const DEFAULT_OPACITY: u32 = 92;

/// Opacity of the floating windows' card (0–1), from the settings.
pub struct HudOpacity(pub f32);

impl Global for HudOpacity {}

pub fn set_opacity(percent: u32, cx: &mut App) {
    cx.set_global(HudOpacity(percent.clamp(1, 100) as f32 / 100.));
}

pub fn opacity(cx: &App) -> f32 {
    cx.try_global::<HudOpacity>()
        .map(|o| o.0)
        .unwrap_or(DEFAULT_OPACITY as f32 / 100.)
}
/// The results list grows with its rows up to this; then it scrolls.
const MAX_LIST_HEIGHT: f32 = 380.;
/// Same as the corners Windows 11 cuts on the window (`DWMWCP_ROUND`).
const CARD_RADIUS: f32 = 8.;
/// How many results show up.
pub const MAX_RESULTS: usize = 7;

/// An item as quick search shows it (no secrets).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickItem {
    pub id: String,
    pub name: String,
    /// Username, website or type — the second line.
    pub detail: String,
    pub icon: crate::item_icons::ItemIcon,
    /// Color chosen for the icon (id from `item_icons::COLORS`).
    pub color: Option<String>,
    /// Saved addresses (for search only).
    pub uris: Vec<String>,
    pub favorite: bool,
    pub has_username: bool,
    pub has_password: bool,
    pub has_totp: bool,
    /// Can be typed (login with a username or password).
    pub typeable: bool,
}

impl QuickItem {
    pub fn from_item(item: &Item) -> Self {
        let (has_username, has_password, has_totp) = VaultView::copyable(item);
        let (detail, uris) = match &item.kind {
            ItemKind::Login(l) => (
                l.username
                    .clone()
                    .filter(|u| !u.is_empty())
                    .or_else(|| l.uris.first().cloned())
                    .unwrap_or_default(),
                l.uris.clone(),
            ),
            ItemKind::Card(c) => (
                c.brand.clone().unwrap_or_else(|| t!("kind.card").into()),
                Vec::new(),
            ),
            ItemKind::Identity(_) => (t!("kind.identity").into(), Vec::new()),
            ItemKind::SecureNote => (t!("kind.note").into(), Vec::new()),
            ItemKind::SshKey(_) => (t!("quick.ssh_key").into(), Vec::new()),
            ItemKind::Unknown(_) => (String::new(), Vec::new()),
        };
        Self {
            id: item.id.clone(),
            name: item.name.clone(),
            detail,
            icon: kind_icon(&item.kind).into(),
            color: None,
            uris,
            favorite: item.favorite,
            has_username,
            has_password,
            has_totp,
            typeable: crate::autotype::typeable(item),
        }
    }
}

/// The items that match the search, from most to least likely.
pub fn search(items: &[QuickItem], query: &str) -> Vec<QuickItem> {
    let q = query.trim().to_lowercase();
    let mut scored: Vec<(u8, &QuickItem)> = items
        .iter()
        .filter_map(|i| {
            let name = i.name.to_lowercase();
            let score = if q.is_empty() {
                // No search: favorites first.
                if i.favorite {
                    0
                } else {
                    5
                }
            } else if name.starts_with(&q) {
                0
            } else if name.split_whitespace().any(|w| w.starts_with(&q)) {
                1
            } else if name.contains(&q) {
                2
            } else if i.detail.to_lowercase().contains(&q) {
                3
            } else if i.uris.iter().any(|u| u.to_lowercase().contains(&q)) {
                4
            } else {
                return None;
            };
            Some((score, i))
        })
        .collect();
    scored.sort_by(|(sa, a), (sb, b)| {
        sa.cmp(sb)
            .then(b.favorite.cmp(&a.favorite))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    scored
        .into_iter()
        .take(MAX_RESULTS)
        .map(|(_, i)| i.clone())
        .collect()
}

pub struct QuickSearch {
    items: Vec<QuickItem>,
    results: Vec<QuickItem>,
    selected: usize,
    query: Entity<InputState>,
    /// There is a target app to type into (the shortcut wasn't used in Scout Vault itself).
    can_fill: bool,
    /// Title of the target app.
    target: String,
    /// Opened by the auto-type shortcut (Ctrl+Alt+A): a HUD with only the
    /// search, the logins and a close button; Enter types into the target app.
    autotype: bool,
    /// Ids the target window's title suggests, listed first (auto-type).
    suggested: Vec<String>,
    /// Vault locked: the HUD asks for the vault password first.
    locked: Option<Locked>,
    /// Signed out: the HUD shows the login (email, password, 2FA…).
    login: Option<Entity<crate::login::LoginView>>,
    /// Auto-type stopped or needs a confirmation: message (and "type anyway").
    notice: Option<(SharedString, bool)>,
    tx: UnboundedSender<Command>,
    /// Notice after copying ("Password copied"), before closing.
    flash: Option<SharedString>,
    activated: bool,
    _subscriptions: Vec<Subscription>,
}

/// The HUD's password step.
struct Locked {
    password: Entity<InputState>,
    busy: bool,
    error: Option<SharedString>,
}

impl QuickSearch {
    /// The vault is locked: the HUD asks for the vault password, then the app
    /// opens the list (or types, if the window title picks one login).
    pub fn locked(
        target: Option<String>,
        autotype: bool,
        tx: UnboundedSender<Command>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self::build(Vec::new(), target, autotype, Vec::new(), tx, window, cx);
        let password = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder(t!("hud.password_placeholder"))
        });
        password.update(cx, |p, cx| p.focus(window, cx));
        this.locked = Some(Locked {
            password,
            busy: false,
            error: None,
        });
        this
    }

    /// Signed out: the HUD hosts the login; the app follows its events.
    pub fn signed_out(
        target: Option<String>,
        autotype: bool,
        store: crate::settings::SettingsStore,
        tx: UnboundedSender<Command>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self::build(Vec::new(), target, autotype, Vec::new(), tx, window, cx);
        this.login = Some(cx.new(|cx| crate::login::LoginView::new_compact(store, window, cx)));
        this
    }

    /// A message from auto-type in the HUD. With `confirm`, offers "type anyway".
    pub fn notice(
        message: SharedString,
        confirm: bool,
        tx: UnboundedSender<Command>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self::build(Vec::new(), None, true, Vec::new(), tx, window, cx);
        this.notice = Some((message, confirm));
        this
    }

    pub fn notice_text(&self) -> Option<&SharedString> {
        self.notice.as_ref().map(|(m, _)| m)
    }

    pub fn login_view(&self) -> Option<&Entity<crate::login::LoginView>> {
        self.login.as_ref()
    }

    pub fn is_locked(&self) -> bool {
        self.locked.is_some()
    }

    /// Error shown in the password step (wrong password…).
    pub fn unlock_error(&self) -> Option<&SharedString> {
        self.locked.as_ref().and_then(|l| l.error.as_ref())
    }

    /// Sends the typed password to the app (Enter in the locked HUD).
    pub fn submit_password(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(locked) = &mut self.locked else {
            return;
        };
        if locked.busy {
            return;
        }
        let password = zeroize::Zeroizing::new(locked.password.read(cx).value().to_string());
        if password.is_empty() {
            locked.error = Some(t!("login.password_required").into());
            cx.notify();
            return;
        }
        locked
            .password
            .update(cx, |p, cx| p.set_value("", window, cx));
        locked.busy = true;
        locked.error = None;
        let _ = self
            .tx
            .send(Command::Unlock(crate::tray::HudPassword(password)));
        cx.notify();
    }

    /// Wrong password (or another problem): show it and let the person try again.
    pub fn unlock_failed(
        &mut self,
        message: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(locked) = &mut self.locked {
            locked.busy = false;
            locked.error = Some(message);
            locked.password.update(cx, |p, cx| p.focus(window, cx));
            cx.notify();
        }
    }

    pub fn new(
        items: Vec<QuickItem>,
        target: Option<String>,
        tx: UnboundedSender<Command>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::build(items, target, false, Vec::new(), tx, window, cx)
    }

    /// The auto-type HUD: only typeable logins, the suggested ones first.
    pub fn autotype(
        items: Vec<QuickItem>,
        target: String,
        suggested: Vec<String>,
        tx: UnboundedSender<Command>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let items = items.into_iter().filter(|i| i.typeable).collect();
        Self::build(items, Some(target), true, suggested, tx, window, cx)
    }

    fn build(
        items: Vec<QuickItem>,
        target: Option<String>,
        autotype: bool,
        suggested: Vec<String>,
        tx: UnboundedSender<Command>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let query = cx.new(|cx| InputState::new(window, cx).placeholder(t!("quick.placeholder")));
        let input_sub = cx.subscribe_in(&query, window, |this, _, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Change) {
                this.refresh(cx);
            }
        });
        // Clicking outside closes it, like a menu.
        let activation = cx.observe_window_activation(window, |this, window, _| {
            if window.is_window_active() {
                this.activated = true;
            } else if this.activated && this.login.is_none() {
                window.remove_window();
            }
        });
        query.update(cx, |q, cx| q.focus(window, cx));
        let results = Self::find(&items, &suggested, "");
        Self {
            items,
            results,
            selected: 0,
            query,
            can_fill: target.is_some(),
            target: target.unwrap_or_default(),
            autotype,
            suggested,
            locked: None,
            login: None,
            notice: None,
            tx,
            flash: None,
            activated: false,
            _subscriptions: vec![input_sub, activation],
        }
    }

    /// The search, with the suggested items first while there is no query.
    fn find(items: &[QuickItem], suggested: &[String], query: &str) -> Vec<QuickItem> {
        let mut found = search(items, query);
        if query.trim().is_empty() && !suggested.is_empty() {
            found = items
                .iter()
                .filter(|i| suggested.contains(&i.id))
                .cloned()
                .chain(found.into_iter().filter(|i| !suggested.contains(&i.id)))
                .take(MAX_RESULTS)
                .collect();
        }
        found
    }

    /// Opened by Ctrl+Alt+A.
    pub fn is_autotype(&self) -> bool {
        self.autotype
    }

    pub fn results(&self) -> &[QuickItem] {
        &self.results
    }

    pub fn selected(&self) -> Option<&QuickItem> {
        self.results.get(self.selected)
    }

    pub fn set_query(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.query
            .update(cx, |q, cx| q.set_value(text.to_owned(), window, cx));
        self.refresh(cx);
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let q = self.query.read(cx).value().to_string();
        self.results = Self::find(&self.items, &self.suggested, &q);
        self.selected = 0;
        cx.notify();
    }

    pub fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.results.is_empty() {
            return;
        }
        let n = self.results.len() as isize;
        self.selected = (self.selected as isize + delta).rem_euclid(n) as usize;
        cx.notify();
    }

    /// Enter: types into the target app (logins) or opens it in Scout Vault.
    pub fn activate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self.selected().cloned() else {
            return;
        };
        if self.can_fill && item.typeable {
            // Close first: focus goes back to the target app, where the app will type.
            window.remove_window();
            let _ = self.tx.send(Command::Fill { item_id: item.id });
        } else {
            self.open(window, cx);
        }
    }

    pub fn open(&mut self, window: &mut Window, _: &mut Context<Self>) {
        let Some(item) = self.selected().cloned() else {
            return;
        };
        window.remove_window();
        let _ = self.tx.send(Command::OpenItem { item_id: item.id });
    }

    pub fn copy(&mut self, what: CopyShortcut, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self.selected().cloned() else {
            return;
        };
        let has = match what {
            CopyShortcut::Username => item.has_username,
            CopyShortcut::Password => item.has_password,
            CopyShortcut::TotpCode => item.has_totp,
        };
        if !has {
            self.flash = Some(
                match what {
                    CopyShortcut::Username => t!("quick.no_username"),
                    CopyShortcut::Password => t!("quick.no_password"),
                    CopyShortcut::TotpCode => t!("quick.no_code"),
                }
                .into(),
            );
            cx.notify();
            return;
        }
        let _ = self.tx.send(Command::Copy {
            item_id: item.id,
            what,
        });
        self.flash = Some(match what {
            CopyShortcut::Username => t!("quick.username_copied").into(),
            CopyShortcut::Password => {
                t!("generator.copied", note = crate::clipboard::clear_note(cx)).into()
            }
            CopyShortcut::TotpCode => t!("quick.code_copied").into(),
        });
        cx.notify();
        // Shows the notice for a moment and closes.
        cx.spawn_in(window, async move |_, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(650))
                .await;
            cx.update(|window, _| window.remove_window()).ok();
        })
        .detach();
    }

    fn on_key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &e.keystroke;
        let ctrl = k.modifiers.control;
        if self.notice.is_some() {
            if k.key == "escape" {
                window.remove_window();
                cx.stop_propagation();
            }
            return;
        }
        if self.login.is_some() {
            if k.key == "escape" {
                window.remove_window();
                cx.stop_propagation();
            }
            return;
        }
        if self.locked.is_some() {
            match k.key.as_str() {
                "escape" => window.remove_window(),
                "enter" => self.submit_password(window, cx),
                _ => return,
            }
            cx.stop_propagation();
            return;
        }
        match (ctrl, k.key.as_str()) {
            (false, "escape") => window.remove_window(),
            (false, "down") => self.move_selection(1, cx),
            (false, "up") => self.move_selection(-1, cx),
            (false, "enter") => self.activate(window, cx),
            (true, "p") => self.copy(CopyShortcut::Password, window, cx),
            (true, "u") => self.copy(CopyShortcut::Username, window, cx),
            (true, "t") => self.copy(CopyShortcut::TotpCode, window, cx),
            (true, "o") => self.open(window, cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    fn row(&self, ix: usize, item: &QuickItem, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = ix == self.selected;
        let accent = appearance::accent(cx);
        let theme = cx.theme();
        let (muted, hover) = (theme.muted_foreground, theme.secondary_hover);
        h_flex()
            .id(SharedString::from(format!("quick-item-{}", item.id)))
            .test_support()
            .gap_3()
            .px_3()
            .py_2()
            .mx_1p5()
            .rounded(px(9.))
            .cursor_pointer()
            .when(selected, |el| el.bg(accent.opacity(0.14)))
            .when(!selected, |el| el.hover(|el| el.bg(hover)))
            .child(crate::item_icons::chip_with(
                item.icon,
                item.color
                    .as_deref()
                    .and_then(|c| crate::item_icons::color_by_id(c, cx)),
                px(30.),
                selected,
                cx,
            ))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .text_sm()
                            .font_semibold()
                            .child(item.name.clone()),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_xs()
                            .text_color(muted)
                            .child(item.detail.clone()),
                    ),
            )
            .when(selected && !self.autotype, |el| {
                el.child(div().text_xs().text_color(accent).child(
                    if self.can_fill && item.typeable {
                        t!("quick.enter_fill")
                    } else {
                        t!("quick.enter_open")
                    },
                ))
            })
            .on_click(cx.listener(move |this, _, window, cx| {
                this.selected = ix;
                this.activate(window, cx);
            }))
    }
}

impl Render for QuickSearch {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (border, popover, fg, muted) = (
            theme.border,
            theme.popover,
            theme.popover_foreground,
            theme.muted_foreground,
        );
        let rows: Vec<AnyElement> = self
            .results
            .clone()
            .iter()
            .enumerate()
            .map(|(ix, item)| self.row(ix, item, cx).into_any_element())
            .collect();
        let empty = rows.is_empty();
        let target = if self.can_fill {
            t!("quick.fill_in", target = self.target.trim())
        } else {
            t!("quick.title").to_owned()
        };
        let footer: SharedString = match &self.flash {
            Some(text) => text.clone(),
            None => t!("quick.footer").into(),
        };
        if let Some((message, confirm)) = self.notice.clone() {
            let theme = cx.theme();
            let card = v_flex()
                .w_full()
                .rounded(px(CARD_RADIUS))
                .border_1()
                .border_color(theme.border)
                .bg(theme.popover.opacity(opacity(cx)))
                .text_color(theme.popover_foreground)
                .overflow_hidden()
                .p_4()
                .gap_3()
                .child(
                    h_flex()
                        .gap_3()
                        .items_start()
                        .child(
                            div()
                                .text_color(theme.warning)
                                .child(Icon::new(IconName::TriangleAlert)),
                        )
                        .child(
                            div()
                                .id("quick-search-notice")
                                .test_support()
                                .aria_label(message.clone())
                                .flex_1()
                                .text_sm()
                                .child(message),
                        ),
                )
                .child(
                    h_flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("quick-search-close")
                                .ghost()
                                .label(if confirm {
                                    t!("common.cancel")
                                } else {
                                    t!("common.close")
                                })
                                .on_click(cx.listener(|_, _, window, _| window.remove_window())),
                        )
                        .when(confirm, |el| {
                            el.child(
                                Button::new("quick-search-type-anyway")
                                    .primary()
                                    .label(t!("autotype.type_anyway"))
                                    .on_click(cx.listener(|this, _, window, _| {
                                        // Close first: focus goes back to the target app.
                                        window.remove_window();
                                        let _ = this.tx.send(Command::TypeAnyway);
                                    })),
                            )
                        }),
                );
            return self.fit(card, cx);
        }
        if let Some(login) = self.login.clone() {
            let theme = cx.theme();
            let card = v_flex()
                .w_full()
                .relative()
                .rounded(px(CARD_RADIUS))
                .border_1()
                .border_color(theme.border)
                .bg(theme.popover.opacity(opacity(cx)))
                .text_color(theme.popover_foreground)
                .overflow_hidden()
                .p_4()
                .child(login)
                .child(
                    div().absolute().top_2().right_2().child(
                        Button::new("quick-search-close")
                            .ghost()
                            .icon(IconName::Close)
                            .tooltip(t!("autotype.cancel_esc"))
                            .on_click(cx.listener(|_, _, window, _| window.remove_window())),
                    ),
                );
            return self.fit(card, cx);
        }
        if let Some(locked) = &self.locked {
            let card = self.locked_card(locked, cx);
            return self.fit(card, cx);
        }
        let card = v_flex()
            .w_full()
            .rounded(px(CARD_RADIUS))
            .border_1()
            .border_color(border)
            .bg(popover.opacity(opacity(cx)))
            .text_color(fg)
            .overflow_hidden()
            .child(if self.autotype {
                // HUD: only the search and the close button.
                h_flex()
                    .gap_2()
                    .px_3()
                    .pt_3()
                    .pb_2()
                    .child(
                        div().flex_1().child(
                            Input::new(&self.query)
                                .id("quick-search-input")
                                .prefix(Icon::new(IconName::Search).small())
                                .large(),
                        ),
                    )
                    .child(
                        Button::new("quick-search-close")
                            .ghost()
                            .icon(IconName::Close)
                            .tooltip(t!("autotype.cancel_esc"))
                            .on_click(cx.listener(|_, _, window, _| window.remove_window())),
                    )
                    .into_any_element()
            } else {
                v_flex()
                    .gap_1()
                    .px_4()
                    .pt_3()
                    .pb_2()
                    .child(div().text_xs().text_color(muted).truncate().child(target))
                    .child(Input::new(&self.query).id("quick-search-input").large())
                    .into_any_element()
            })
            .child(
                v_flex()
                    .id("quick-search-results")
                    .max_h(px(MAX_LIST_HEIGHT))
                    .overflow_y_scroll()
                    .py_1()
                    .gap_px()
                    .children(rows)
                    .when(empty, |el| {
                        el.child(
                            v_flex()
                                .py_6()
                                .items_center()
                                .justify_center()
                                .gap_2()
                                .text_sm()
                                .text_color(muted)
                                .child(Icon::new(IconName::Search))
                                .child(t!("quick.nothing_found")),
                        )
                    }),
            )
            .when(!self.autotype, |el| {
                el.child(
                    div()
                        .id("quick-search-footer")
                        .test_support()
                        .aria_label(footer.clone())
                        .px_4()
                        .py_2()
                        .border_t_1()
                        .border_color(border)
                        .text_xs()
                        .text_color(if self.flash.is_some() {
                            appearance::accent(cx)
                        } else {
                            muted
                        })
                        .truncate()
                        .child(footer),
                )
            });
        self.fit(card, cx)
    }
}

impl QuickSearch {
    /// The HUD's root: the card at its natural height, and the window follows
    /// it (it grows or shrinks downward, from the top).
    fn fit(&self, card: impl IntoElement, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("quick-search")
            .w_full()
            .capture_key_down(cx.listener(Self::on_key))
            .child(
                div().relative().w_full().child(card).child(
                    canvas(
                        |_, _, _| {},
                        |bounds, _, window, cx| {
                            let height = bounds.size.height.ceil();
                            let viewport = window.viewport_size();
                            if (f32::from(height) - f32::from(viewport.height)).abs() >= 1. {
                                window.defer(cx, move |window, _| {
                                    window.resize(size(viewport.width, height));
                                });
                            }
                        },
                    )
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
                ),
            )
            .into_any_element()
    }

    /// Password step: lock icon, the field, the error and the close button.
    fn locked_card(&self, locked: &Locked, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme();
        let (border, popover, fg, muted) = (
            theme.border,
            theme.popover,
            theme.popover_foreground,
            theme.muted_foreground,
        );
        let status: SharedString = match (&locked.error, locked.busy) {
            (Some(e), _) => e.clone(),
            (None, true) => t!("unlock.opening").into(),
            (None, false) => t!("hud.locked_hint").into(),
        };
        v_flex()
            .w_full()
            .rounded(px(CARD_RADIUS))
            .border_1()
            .border_color(border)
            .bg(popover.opacity(opacity(cx)))
            .text_color(fg)
            .overflow_hidden()
            .p_3()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .size(px(36.))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(9.))
                            .bg(appearance::accent(cx).opacity(0.14))
                            .text_color(appearance::accent(cx))
                            .child(Icon::new(IconName::Lock).small()),
                    )
                    .child(
                        div().flex_1().child(
                            Input::new(&locked.password)
                                .id("quick-search-password")
                                .large()
                                .disabled(locked.busy),
                        ),
                    )
                    .child(
                        Button::new("quick-search-close")
                            .ghost()
                            .icon(IconName::Close)
                            .tooltip(t!("autotype.cancel_esc"))
                            .on_click(cx.listener(|_, _, window, _| window.remove_window())),
                    ),
            )
            .child(
                div()
                    .id("quick-search-status")
                    .test_support()
                    .aria_label(status.clone())
                    .px_1()
                    .text_xs()
                    .text_color(if locked.error.is_some() {
                        cx.theme().danger
                    } else {
                        muted
                    })
                    .child(status),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{search, QuickItem};
    use gpui_kit::assets::IconName;

    fn item(id: &str, name: &str, detail: &str, favorite: bool) -> QuickItem {
        QuickItem {
            id: id.into(),
            name: name.into(),
            detail: detail.into(),
            icon: IconName::Globe.into(),
            color: None,
            uris: vec![format!("https://{id}.example")],
            favorite,
            has_username: true,
            has_password: true,
            has_totp: false,
            typeable: true,
        }
    }

    #[test]
    fn ranks_prefix_then_word_then_detail() {
        let items = vec![
            item("a", "Conta do Banco", "eu", false),
            item("b", "Banco Inter", "eu", false),
            item("c", "Steam", "banco@mail", false),
            item("d", "Netflix", "x", true),
        ];
        let ids = |q: &str| {
            search(&items, q)
                .into_iter()
                .map(|i| i.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids("banco"), ["b", "a", "c"]);
        assert_eq!(ids("d.example"), ["d"]);
        assert!(ids("nada-disso").is_empty());
        // No search: favorites first.
        assert_eq!(ids("")[0], "d");
    }
}
