//! Generator (`Ctrl+G`): passwords, passphrases and usernames, opened on top of the
//! current screen like settings. The options are saved in `settings.json`; what was
//! generated, only in memory.

use gpui_kit::assets::IconName;
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{button::*, *};
use gpui_kit::prelude::*;
use gpui_kit::*;
use scout_vault_core::generator::{
    self, GeneratorKind, PassphraseOptions, PasswordOptions, UsernameKind, UsernameOptions,
};
use zeroize::Zeroizing;

use crate::settings::SettingsStore;
use crate::ui::{self, Segment};
use crate::{appearance, clipboard};

/// Password length on the slider.
pub const MIN_LENGTH: usize = 8;
pub const MAX_LENGTH: usize = 64;
/// Passphrase words on the slider.
pub const MIN_WORDS: usize = 3;
pub const MAX_WORDS: usize = 10;
/// Passphrase separators offered (the last one is "none").
pub const SEPARATORS: [&str; 5] = ["-", ".", "_", " ", ""];
const KINDS: [GeneratorKind; 3] = [
    GeneratorKind::Password,
    GeneratorKind::Passphrase,
    GeneratorKind::Username,
];

pub enum GeneratorEvent {
    Close,
}

pub struct GeneratorView {
    store: SettingsStore,
    kind: GeneratorKind,
    options: PasswordOptions,
    phrase: PassphraseOptions,
    username: UsernameOptions,
    /// Account email, for "email with +" usernames (empty when signed out).
    email: String,
    value: Zeroizing<String>,
    /// How many values were generated so far: restarts the animation without putting the value in the id.
    generation: u32,
    length_slider: Entity<SliderState>,
    words_slider: Entity<SliderState>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<GeneratorEvent> for GeneratorView {}

impl GeneratorView {
    pub fn new(store: SettingsStore, email: String, cx: &mut Context<Self>) -> Self {
        let settings = store.load();
        let length = settings.generator.length.clamp(MIN_LENGTH, MAX_LENGTH);
        let words = settings.passphrase.words.clamp(MIN_WORDS, MAX_WORDS);
        let length_slider = cx.new(|_| {
            SliderState::new()
                .min(MIN_LENGTH as f32)
                .max(MAX_LENGTH as f32)
                .step(1.)
                .default_value(length as f32)
        });
        let words_slider = cx.new(|_| {
            SliderState::new()
                .min(MIN_WORDS as f32)
                .max(MAX_WORDS as f32)
                .step(1.)
                .default_value(words as f32)
        });
        let subscriptions = vec![
            cx.subscribe(&length_slider, |this, _, event: &SliderEvent, cx| {
                let SliderEvent::Change(v) = event else {
                    return;
                };
                let n = v.start().round() as usize;
                if n != this.options.length {
                    this.edit(|o| o.length = n, cx);
                }
            }),
            cx.subscribe(&words_slider, |this, _, event: &SliderEvent, cx| {
                let SliderEvent::Change(v) = event else {
                    return;
                };
                let n = v.start().round() as usize;
                if n != this.phrase.words {
                    this.edit_phrase(|o| o.words = n, cx);
                }
            }),
        ];
        let mut view = Self {
            store,
            kind: settings.generator_kind,
            options: settings.generator,
            phrase: settings.passphrase,
            username: settings.username_generator,
            email,
            value: Zeroizing::new(String::new()),
            generation: 0,
            length_slider,
            words_slider,
            _subscriptions: subscriptions,
        };
        view.value = view.generate();
        view
    }

    fn generate(&self) -> Zeroizing<String> {
        match self.kind {
            GeneratorKind::Password => generator::password(&self.options),
            GeneratorKind::Passphrase => generator::passphrase(&self.phrase),
            GeneratorKind::Username => {
                Zeroizing::new(generator::username(&self.username, &self.email))
            }
        }
    }

    /// What is on screen (password, passphrase or username).
    pub fn password(&self) -> &str {
        &self.value
    }

    pub fn kind(&self) -> GeneratorKind {
        self.kind
    }

    pub fn options(&self) -> PasswordOptions {
        self.options
    }

    pub fn passphrase_options(&self) -> &PassphraseOptions {
        &self.phrase
    }

    pub fn username_options(&self) -> UsernameOptions {
        self.username
    }

    pub fn regenerate(&mut self, cx: &mut Context<Self>) {
        self.value = self.generate();
        self.generation += 1;
        cx.notify();
    }

    /// Saves everything the screen chose and generates again.
    fn save_and_regenerate(&mut self, cx: &mut Context<Self>) {
        let mut settings = self.store.load();
        settings.generator = self.options;
        settings.generator_kind = self.kind;
        settings.passphrase = self.phrase.clone();
        settings.username_generator = self.username;
        self.store.save(&settings);
        self.regenerate(cx);
    }

    pub fn set_kind(&mut self, kind: GeneratorKind, cx: &mut Context<Self>) {
        self.kind = kind;
        self.save_and_regenerate(cx);
    }

    /// Changes the password options, generates again and saves.
    pub fn set_options(&mut self, next: PasswordOptions, cx: &mut Context<Self>) {
        self.options = next;
        self.save_and_regenerate(cx);
    }

    fn edit(&mut self, f: impl FnOnce(&mut PasswordOptions), cx: &mut Context<Self>) {
        let mut next = self.options;
        f(&mut next);
        self.set_options(next, cx);
    }

    pub fn edit_phrase(&mut self, f: impl FnOnce(&mut PassphraseOptions), cx: &mut Context<Self>) {
        f(&mut self.phrase);
        self.save_and_regenerate(cx);
    }

    pub fn edit_username(&mut self, f: impl FnOnce(&mut UsernameOptions), cx: &mut Context<Self>) {
        f(&mut self.username);
        self.save_and_regenerate(cx);
    }

    pub fn copy(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        clipboard::copy_and_clear_later(&self.value, cx);
        let _ = window;
        let note = clipboard::clear_note(cx);
        let message = match self.kind {
            GeneratorKind::Password => t!("generator.copied", note = note),
            GeneratorKind::Passphrase => t!("generator.copied_phrase", note = note),
            GeneratorKind::Username => t!("generator.copied_username", note = note),
        };
        crate::toast::success(message, cx);
    }

    /// Entropy of what is on screen (`None` for usernames: they aren't secrets).
    fn entropy_bits(&self) -> Option<f64> {
        match self.kind {
            GeneratorKind::Password => Some(self.options.entropy_bits()),
            GeneratorKind::Passphrase => Some(self.phrase.entropy_bits()),
            GeneratorKind::Username => None,
        }
    }
}

/// Strength level 1–4 and its name, from the entropy bits.
fn strength(bits: f64) -> (usize, &'static str) {
    match bits {
        b if b < 40.0 => (1, t!("generator.weak")),
        b if b < 64.0 => (2, t!("generator.fair")),
        b if b < 100.0 => (3, t!("generator.strong")),
        _ => (4, t!("generator.very_strong")),
    }
}

fn kind_label(kind: GeneratorKind) -> &'static str {
    match kind {
        GeneratorKind::Password => t!("generator.kind_password"),
        GeneratorKind::Passphrase => t!("generator.kind_passphrase"),
        GeneratorKind::Username => t!("generator.kind_username"),
    }
}

fn kind_icon(kind: GeneratorKind) -> IconName {
    match kind {
        GeneratorKind::Password => IconName::KeyRound,
        GeneratorKind::Passphrase => IconName::FileText,
        GeneratorKind::Username => IconName::User,
    }
}

fn kind_id(kind: GeneratorKind) -> &'static str {
    match kind {
        GeneratorKind::Password => "gen-kind-password",
        GeneratorKind::Passphrase => "gen-kind-passphrase",
        GeneratorKind::Username => "gen-kind-username",
    }
}

fn separator_label(sep: &str) -> SharedString {
    match sep {
        " " => t!("generator.sep_space").into(),
        "" => t!("generator.sep_none").into(),
        other => other.to_string().into(),
    }
}

/// A row of the options card: title (and a note) on the left, control on the right.
fn row(
    title: &'static str,
    note: Option<SharedString>,
    control: impl IntoElement,
    cx: &App,
) -> Div {
    h_flex()
        .gap_6()
        .px_5()
        .py_3p5()
        .justify_between()
        .child(
            v_flex()
                .min_w_0()
                .gap_0p5()
                .child(div().text_sm().font_semibold().child(title))
                .when_some(note, |el, note| {
                    el.child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(note),
                    )
                }),
        )
        .child(div().flex_shrink_0().child(control))
}

/// A slider with its value beside it.
fn slider_row(
    title: &'static str,
    id: &'static str,
    state: &Entity<SliderState>,
    value: usize,
    cx: &App,
) -> Div {
    v_flex()
        .gap_2()
        .px_5()
        .py_3p5()
        .child(
            h_flex()
                .justify_between()
                .child(div().text_sm().font_semibold().child(title))
                .child(
                    div()
                        .id(id)
                        .test_support()
                        .aria_label(SharedString::from(value.to_string()))
                        .min_w(px(36.))
                        .px_2()
                        .py_0p5()
                        .rounded(cx.theme().radius)
                        .bg(cx.theme().secondary)
                        .text_sm()
                        .font_semibold()
                        .text_center()
                        .child(value.to_string()),
                ),
        )
        .child(Slider::new(state).horizontal().accessibility_label(title))
}

/// A character class: (element id, label, on, setter).
type CharSet = (
    &'static str,
    &'static str,
    bool,
    fn(&mut PasswordOptions, bool),
);

/// Character class of a pill (A–Z, 0–9…): on/off, drawn like a chip.
fn pill(
    id: &'static str,
    label: &'static str,
    on: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let accent = appearance::accent(cx);
    let theme = cx.theme();
    h_flex()
        .id(id)
        .test_support()
        .role(Role::CheckBox)
        .aria_label(SharedString::from(label))
        .gap_1p5()
        .px_3()
        .py_1p5()
        .rounded_full()
        .border_1()
        .cursor_pointer()
        .font_family(theme.mono_font_family.clone())
        .text_sm()
        .map(|el| {
            if on {
                el.border_color(accent.opacity(0.6))
                    .bg(accent.opacity(0.12))
                    .text_color(accent)
            } else {
                el.border_color(theme.border)
                    .text_color(theme.muted_foreground)
                    .hover(|s| s.bg(theme.secondary_hover))
            }
        })
        .when(on, |el| el.child(Icon::new(IconName::Check).xsmall()))
        .child(label)
        .on_click(on_click)
        .into_any_element()
}

/// What was generated, with digits and symbols in their own colors (easier to read out).
fn colored_value(value: &str, kind: GeneratorKind, cx: &App) -> Div {
    let dark = cx.theme().mode.is_dark();
    let l = if dark { 0.68 } else { 0.45 };
    let digit = hsla(0.6, 0.75, l, 1.);
    let symbol = hsla(0.02, 0.75, l, 1.);
    let muted = cx.theme().muted_foreground;
    let mut out = h_flex().flex_wrap().justify_center();
    for c in value.chars() {
        let color = if c.is_ascii_digit() {
            Some(digit)
        } else if c.is_alphabetic() {
            None
        } else if kind == GeneratorKind::Password {
            Some(symbol)
        } else {
            Some(muted)
        };
        // A space would collapse: show a no-break space.
        let shown = if c == ' ' { '\u{a0}' } else { c };
        out = out.child(
            div()
                .when_some(color, |el, color| el.text_color(color))
                .child(shown.to_string()),
        );
    }
    out
}

impl GeneratorView {
    fn password_rows(&self, cx: &mut Context<Self>) -> Vec<Div> {
        let o = self.options;
        let sets: [CharSet; 4] = [
            ("gen-upper", "A–Z", o.uppercase, |o, v| o.uppercase = v),
            ("gen-lower", "a–z", o.lowercase, |o, v| o.lowercase = v),
            ("gen-digits", "0–9", o.digits, |o, v| o.digits = v),
            ("gen-symbols", "!@#$%^&*", o.symbols, |o, v| o.symbols = v),
        ];
        let mut pills = h_flex().flex_wrap().gap_2();
        for (id, label, on, set) in sets {
            let listener = cx.listener(move |this, _, _, cx| this.edit(|o| set(o, !on), cx));
            pills = pills.child(pill(id, label, on, listener, cx));
        }
        vec![
            slider_row(
                t!("generator.length"),
                "generator-length-value",
                &self.length_slider,
                o.length.clamp(generator::MIN_LENGTH, generator::MAX_LENGTH),
                cx,
            ),
            v_flex()
                .gap_2p5()
                .px_5()
                .py_3p5()
                .child(
                    div()
                        .text_sm()
                        .font_semibold()
                        .child(t!("generator.characters")),
                )
                .child(pills),
            row(
                t!("generator.avoid_ambiguous_title"),
                Some(t!("generator.avoid_ambiguous_desc").into()),
                Switch::new("gen-ambiguous")
                    .checked(o.avoid_ambiguous)
                    .on_click(cx.listener(|this, v: &bool, _, cx| {
                        let v = *v;
                        this.edit(|o| o.avoid_ambiguous = v, cx)
                    })),
                cx,
            ),
        ]
    }

    fn passphrase_rows(&self, cx: &mut Context<Self>) -> Vec<Div> {
        let p = self.phrase.clone();
        let this = cx.entity().downgrade();
        let separator = ui::segmented(
            "generator-separator",
            SEPARATORS
                .iter()
                .enumerate()
                .map(|(ix, sep)| Segment {
                    id: format!("sep-{ix}").into(),
                    label: separator_label(sep),
                    icon: None,
                    selected: *sep == p.separator,
                })
                .collect(),
            move |ix, _, cx| {
                this.update(cx, |v, cx| {
                    v.edit_phrase(|o| o.separator = SEPARATORS[ix].into(), cx)
                })
                .ok();
            },
            cx,
        );
        vec![
            slider_row(
                t!("generator.words"),
                "generator-words-value",
                &self.words_slider,
                p.words.clamp(generator::MIN_WORDS, generator::MAX_WORDS),
                cx,
            ),
            row(t!("generator.separator"), None, separator, cx),
            row(
                t!("generator.capitalize"),
                None,
                Switch::new("gen-capitalize")
                    .checked(p.capitalize)
                    .on_click(cx.listener(|this, v: &bool, _, cx| {
                        let v = *v;
                        this.edit_phrase(|o| o.capitalize = v, cx)
                    })),
                cx,
            ),
            row(
                t!("generator.include_number"),
                None,
                Switch::new("gen-number")
                    .checked(p.include_number)
                    .on_click(cx.listener(|this, v: &bool, _, cx| {
                        let v = *v;
                        this.edit_phrase(|o| o.include_number = v, cx)
                    })),
                cx,
            ),
        ]
    }

    fn username_rows(&self, cx: &mut Context<Self>) -> Vec<Div> {
        let u = self.username;
        let this = cx.entity().downgrade();
        let kinds = [UsernameKind::Word, UsernameKind::PlusEmail];
        let kind = ui::segmented(
            "generator-username-kind",
            kinds
                .iter()
                .map(|k| Segment {
                    id: match k {
                        UsernameKind::Word => "user-kind-word",
                        UsernameKind::PlusEmail => "user-kind-plus",
                    }
                    .into(),
                    label: match k {
                        UsernameKind::Word => t!("generator.username_word"),
                        UsernameKind::PlusEmail => t!("generator.username_plus"),
                    }
                    .into(),
                    icon: None,
                    selected: *k == u.kind,
                })
                .collect(),
            move |ix, _, cx| {
                this.update(cx, |v, cx| v.edit_username(|o| o.kind = kinds[ix], cx))
                    .ok();
            },
            cx,
        );
        let mut rows = vec![row(t!("generator.username_type"), None, kind, cx)];
        match u.kind {
            UsernameKind::Word => {
                rows.push(row(
                    t!("generator.capitalize"),
                    None,
                    Switch::new("gen-user-capitalize")
                        .checked(u.capitalize)
                        .on_click(cx.listener(|this, v: &bool, _, cx| {
                            let v = *v;
                            this.edit_username(|o| o.capitalize = v, cx)
                        })),
                    cx,
                ));
                rows.push(row(
                    t!("generator.include_number"),
                    None,
                    Switch::new("gen-user-number")
                        .checked(u.include_number)
                        .on_click(cx.listener(|this, v: &bool, _, cx| {
                            let v = *v;
                            this.edit_username(|o| o.include_number = v, cx)
                        })),
                    cx,
                ));
            }
            UsernameKind::PlusEmail => {
                let hint = match self.email.split_once('@') {
                    Some((local, domain)) if !local.is_empty() => t!(
                        "generator.username_plus_hint",
                        example =
                            format!("{}+tag@{domain}", local.split('+').next().unwrap_or(local))
                    )
                    .to_string(),
                    _ => t!("generator.username_plus_missing").to_string(),
                };
                rows.push(
                    div()
                        .px_5()
                        .py_3p5()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(hint),
                );
            }
        }
        rows
    }
}

impl Render for GeneratorView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let current = self.kind;
        let accent = appearance::accent(cx);
        let this = cx.entity().downgrade();

        let header = h_flex()
            .gap_4()
            .child(ui::icon_chip(IconName::Dices, px(44.), true, cx))
            .child(
                v_flex()
                    .gap_0p5()
                    .child(
                        div()
                            .text_2xl()
                            .font_semibold()
                            .child(t!("generator.title")),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(t!("generator.subtitle")),
                    ),
            );

        let kinds = ui::segmented(
            "generator-kind",
            KINDS
                .iter()
                .map(|k| Segment {
                    id: kind_id(*k).into(),
                    label: kind_label(*k).into(),
                    icon: Some(kind_icon(*k)),
                    selected: *k == current,
                })
                .collect(),
            move |ix, _, cx| {
                this.update(cx, |v, cx| v.set_kind(KINDS[ix], cx)).ok();
            },
            cx,
        );

        // Strength: four bars that fill and change color.
        let strength_el = match self.entropy_bits() {
            Some(bits) => {
                let (level, label) = strength(bits);
                let theme = cx.theme();
                let color = match level {
                    1 => theme.danger,
                    2 => theme.warning,
                    _ => theme.success,
                };
                let mut bars = h_flex().gap_1();
                for i in 1..=4 {
                    bars =
                        bars.child(div().w(px(22.)).h(px(5.)).rounded_full().bg(if i <= level {
                            color
                        } else {
                            theme.border
                        }));
                }
                h_flex()
                    .id("generator-strength")
                    .test_support()
                    .aria_label(SharedString::from(label))
                    .gap_3()
                    .child(bars)
                    .child(div().text_xs().text_color(theme.muted_foreground).child(t!(
                        "generator.strength",
                        label = label,
                        bits = bits.round() as u32
                    )))
                    .into_any_element()
            }
            None => div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(t!("generator.username_note"))
                .into_any_element(),
        };

        let output = v_flex()
            .rounded(cx.theme().radius_lg)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .overflow_hidden()
            // A thin accent line on top: the result is the star of the screen.
            .child(div().h(px(3.)).w_full().bg(accent.opacity(0.7)))
            .child(
                div()
                    .id("generator-output")
                    .test_support()
                    .aria_label(SharedString::from(self.value.to_string()))
                    .min_h(px(112.))
                    .px_8()
                    .py_8()
                    .flex()
                    .items_center()
                    .justify_center()
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_2xl()
                    .child(crate::motion::rise(
                        colored_value(&self.value, current, cx),
                        crate::motion::key("senha-gerada", self.generation),
                        6.,
                    )),
            )
            .child(
                h_flex()
                    .gap_2()
                    .px_5()
                    .py_3()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().secondary.opacity(0.5))
                    .child(div().flex_1().child(strength_el))
                    .child(
                        Button::new("generator-refresh")
                            .ghost()
                            .small()
                            .icon(IconName::RefreshCw)
                            .accessibility_label(t!("generator.again"))
                            .tooltip(t!("generator.again"))
                            .on_click(cx.listener(|this, _, _, cx| this.regenerate(cx))),
                    )
                    .child(
                        Button::new("generator-copy")
                            .primary()
                            .small()
                            .icon(IconName::Copy)
                            .label(t!("common.copy"))
                            .on_click(cx.listener(|this, _, window, cx| this.copy(window, cx))),
                    ),
            );

        let rows = match current {
            GeneratorKind::Password => self.password_rows(cx),
            GeneratorKind::Passphrase => self.passphrase_rows(cx),
            GeneratorKind::Username => self.username_rows(cx),
        };
        let count = rows.len();
        let options = v_flex()
            .rounded(cx.theme().radius_lg)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .children(rows.into_iter().enumerate().map(|(ix, r)| {
                r.when(ix + 1 < count, |r| {
                    r.border_b_1().border_color(cx.theme().border)
                })
            }));
        let footer = (current == GeneratorKind::Passphrase).then(|| {
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(t!("generator.phrase_credit"))
        });

        // "Back" stays put on the left while the page scrolls, like the other screens.
        let back = div().absolute().top_6().left_6().child(
            ui::back_button("generator-close")
                .on_click(cx.listener(|_, _, _, cx| cx.emit(GeneratorEvent::Close))),
        );

        div()
            .relative()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                div().id("generator").size_full().overflow_y_scroll().child(
                    v_flex()
                        .w_full()
                        .max_w(px(640.))
                        .mx_auto()
                        .px_8()
                        .py_10()
                        .gap_6()
                        .child(header)
                        .child(kinds)
                        .child(output)
                        .child(
                            v_flex()
                                .gap_2()
                                .child(ui::section_label(t!("generator.options"), cx).px_0())
                                .child(options)
                                .children(footer),
                        ),
                ),
            )
            .child(back)
    }
}
