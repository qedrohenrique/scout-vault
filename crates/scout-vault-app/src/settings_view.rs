//! Settings screen. New sections go in as another `group` in
//! [`SettingsView::render`].

use gpui_kit::assets::IconName;
use gpui_kit::component::{button::*, *};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::appearance::{Accent, Appearance, Corners, Motion, TextSize, ThemeChoice};
use crate::settings::{AutoTypeSettings, SettingsStore};
use crate::ui::{self, Segment};
use gpui_kit::component::color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState};
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::switch::Switch;

pub enum SettingsEvent {
    Close,
    /// Turned Windows Hello on/off (the app creates or deletes the key).
    WindowsHello(bool),
    /// Changed the interface language (already applied; the app rebuilds what holds text).
    Language,
    /// A global shortcut changed (the app registers the new combination).
    Hotkeys,
    /// Recording a shortcut started (`true`) or ended: the app pauses its own
    /// global shortcuts meanwhile, or Windows would take the current combination.
    RecordingShortcut(bool),
}

/// Which global shortcut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyKind {
    AutoType,
    QuickSearch,
}

impl HotkeyKind {
    fn id(self) -> &'static str {
        match self {
            Self::AutoType => "shortcut-autotype",
            Self::QuickSearch => "shortcut-quick",
        }
    }
}

/// Options of a [`SettingsView::number_choice`].
struct NumberChoice {
    id: &'static str,
    /// Prefix of each option's id (`{prefix}-{value}`).
    prefix: &'static str,
    options: &'static [u32],
    /// Label for 0 ("Always", "Never").
    zero: &'static str,
    label: fn(u32) -> SharedString,
    current: u32,
    set: fn(&mut SettingsView, u32, &mut Context<SettingsView>),
}

pub struct SettingsView {
    store: SettingsStore,
    appearance: Appearance,
    /// Code of the chosen language; empty = the system's.
    language: String,
    autotype: AutoTypeSettings,
    remember_session: bool,
    close_to_tray: bool,
    start_with_windows: bool,
    lock_with_windows: bool,
    lock_on_minimize: bool,
    clipboard_clear_secs: u32,
    hud_opacity: u32,
    windows_hello: bool,
    background_sync: bool,
    site_icons: bool,
    quick_search: bool,
    unlock_memory_days: u32,
    relogin_days: u32,
    auto_lock_minutes: u32,
    browser_fill: bool,
    browser_error: Option<SharedString>,
    /// Floating windows opacity, 1–100%.
    hud_slider: Entity<SliderState>,
    hotkeys: crate::shortcut::Hotkeys,
    /// Waiting for the new combination of this shortcut.
    recording: Option<HotkeyKind>,
    /// Why the last pressed combination wasn't accepted.
    shortcut_error: Option<(HotkeyKind, SharedString)>,
    focus: FocusHandle,
    _hud_slider: Subscription,
    /// "Any color" for the accent.
    accent_picker: Entity<ColorPickerState>,
    _accent_picker: Subscription,
}

impl EventEmitter<SettingsEvent> for SettingsView {}

impl SettingsView {
    pub fn new(store: SettingsStore, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let loaded = store.load();
        let accent_picker = cx.new(|cx| {
            let state = ColorPickerState::new(window, cx);
            match loaded.appearance.custom_accent {
                Some(rgb) => state.default_value(rgb.to_hsla()),
                None => state,
            }
        });
        let _accent_picker = cx.subscribe_in(
            &accent_picker,
            window,
            |this, _, event: &ColorPickerEvent, window, cx| {
                let ColorPickerEvent::Change(Some(color)) = event else {
                    return;
                };
                let rgb = crate::colors::Rgb::from_hsla(*color);
                this.set_custom_accent(Some(rgb), window, cx);
            },
        );
        let hud_slider = cx.new(|_| {
            SliderState::new()
                .min(1.)
                .max(100.)
                .step(1.)
                .default_value(loaded.hud_opacity.clamp(1, 100) as f32)
        });
        let _hud_slider = cx.subscribe(&hud_slider, |this, _, event: &SliderEvent, cx| {
            let SliderEvent::Change(value) = event else {
                return;
            };
            let percent = value.start().round().clamp(1., 100.) as u32;
            if percent != this.hud_opacity {
                this.set_hud_opacity(percent, cx);
            }
        });
        Self {
            store,
            appearance: loaded.appearance,
            language: loaded.language,
            autotype: loaded.autotype,
            remember_session: loaded.remember_session,
            close_to_tray: loaded.close_to_tray,
            start_with_windows: crate::autostart::is_enabled(),
            lock_with_windows: loaded.lock_with_windows,
            lock_on_minimize: loaded.lock_on_minimize,
            clipboard_clear_secs: loaded.clipboard_clear_secs,
            hud_opacity: loaded.hud_opacity,
            windows_hello: loaded.windows_hello,
            background_sync: loaded.background_sync,
            site_icons: loaded.site_icons,
            quick_search: loaded.quick_search,
            unlock_memory_days: loaded.unlock_memory_days,
            relogin_days: loaded.relogin_days,
            auto_lock_minutes: loaded.auto_lock_minutes,
            browser_fill: loaded.browser_fill,
            browser_error: None,
            hud_slider,
            _hud_slider,
            accent_picker,
            _accent_picker,
            hotkeys: loaded.hotkeys.clone(),
            recording: None,
            shortcut_error: None,
            focus: cx.focus_handle(),
        }
    }

    pub fn appearance(&self) -> Appearance {
        self.appearance
    }

    /// Applies right away and saves.
    pub fn set_appearance(
        &mut self,
        next: Appearance,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if next == self.appearance {
            return;
        }
        let prev = self.appearance;
        self.appearance = next;
        let mut settings = self.store.load();
        settings.appearance = next;
        self.store.save(&settings);
        // With a transition: light↔dark "paints" the window from the click; the rest, a fade.
        let origin = window.mouse_position();
        crate::theme_fx::change(&prev, &next, origin, window, cx);
        cx.notify();
    }

    /// Switches the language right away (`""` = the system's), saves and redraws the windows.
    pub fn set_language(&mut self, code: &str, cx: &mut Context<Self>) {
        if code == self.language {
            return;
        }
        self.language = code.to_string();
        let mut settings = self.store.load();
        settings.language = self.language.clone();
        self.store.save(&settings);
        crate::i18n::set(crate::i18n::Language::from_setting(&self.language));
        cx.emit(SettingsEvent::Language);
        cx.refresh_windows();
        cx.notify();
    }

    /// Open without a password for N days (0 = always ask). Turning it off deletes the stored key.
    pub fn set_unlock_memory_days(&mut self, days: u32, cx: &mut Context<Self>) {
        self.unlock_memory_days = days;
        let mut settings = self.store.load();
        settings.unlock_memory_days = days;
        self.store.save(&settings);
        if days == 0 {
            crate::saved_session::forget_quick_unlock(&self.store);
        }
        cx.notify();
    }

    pub fn set_relogin_days(&mut self, days: u32, cx: &mut Context<Self>) {
        self.relogin_days = days;
        let mut settings = self.store.load();
        settings.relogin_days = days;
        self.store.save(&settings);
        cx.notify();
    }

    pub fn set_auto_lock_minutes(&mut self, minutes: u32, cx: &mut Context<Self>) {
        self.auto_lock_minutes = minutes;
        let mut settings = self.store.load();
        settings.auto_lock_minutes = minutes;
        self.store.save(&settings);
        cx.notify();
    }

    /// Segmented control for numeric options (days, minutes).
    fn number_choice(&self, choice: NumberChoice, cx: &mut Context<Self>) -> impl IntoElement {
        let NumberChoice {
            id,
            prefix,
            options,
            zero,
            label,
            current,
            set,
        } = choice;
        let this = cx.entity().downgrade();
        ui::segmented_compact(
            id,
            options
                .iter()
                .map(|v| Segment {
                    id: format!("{prefix}-{v}").into(),
                    label: if *v == 0 { zero.into() } else { label(*v) },
                    icon: None,
                    selected: *v == current,
                })
                .collect(),
            move |ix, _, cx| {
                let value = options[ix];
                this.update(cx, |v, cx| set(v, value, cx)).ok();
            },
            cx,
        )
    }

    pub fn set_quick_search(&mut self, on: bool, cx: &mut Context<Self>) {
        self.quick_search = on;
        let mut settings = self.store.load();
        settings.quick_search = on;
        self.store.save(&settings);
        cx.notify();
    }

    /// Website icons on/off (applied at once; downloading happens when settings close).
    pub fn set_site_icons(&mut self, on: bool, cx: &mut Context<Self>) {
        self.site_icons = on;
        let mut settings = self.store.load();
        settings.site_icons = on;
        self.store.save(&settings);
        crate::site_icons::set_enabled(on, cx);
        cx.notify();
    }

    pub fn set_background_sync(&mut self, on: bool, cx: &mut Context<Self>) {
        self.background_sync = on;
        let mut settings = self.store.load();
        settings.background_sync = on;
        self.store.save(&settings);
        cx.notify();
    }

    /// Turns Windows Hello unlock on/off (the app does the rest).
    pub fn set_windows_hello(&mut self, on: bool, cx: &mut Context<Self>) {
        self.show_windows_hello(on, cx);
        let mut settings = self.store.load();
        settings.windows_hello = on;
        self.store.save(&settings);
        cx.emit(SettingsEvent::WindowsHello(on));
    }

    /// Only updates the switch on screen (the app undid the option, for example).
    pub fn show_windows_hello(&mut self, on: bool, cx: &mut Context<Self>) {
        self.windows_hello = on;
        cx.notify();
    }

    pub fn set_lock_with_windows(&mut self, on: bool, cx: &mut Context<Self>) {
        self.lock_with_windows = on;
        let mut settings = self.store.load();
        settings.lock_with_windows = on;
        self.store.save(&settings);
        cx.notify();
    }

    pub fn set_lock_on_minimize(&mut self, on: bool, cx: &mut Context<Self>) {
        self.lock_on_minimize = on;
        let mut settings = self.store.load();
        settings.lock_on_minimize = on;
        self.store.save(&settings);
        cx.notify();
    }

    /// Time until the clipboard is cleared; applies from the next copy.
    pub fn set_clipboard_clear_secs(&mut self, secs: u32, cx: &mut Context<Self>) {
        self.clipboard_clear_secs = secs;
        let mut settings = self.store.load();
        settings.clipboard_clear_secs = secs;
        self.store.save(&settings);
        crate::clipboard::set_clear_after_secs(secs, cx);
        cx.notify();
    }

    pub fn is_recording(&self) -> bool {
        self.recording.is_some()
    }

    pub fn hotkeys(&self) -> &crate::shortcut::Hotkeys {
        &self.hotkeys
    }

    /// Starts waiting for the new combination (the next keys pressed in the app).
    pub fn record_shortcut(
        &mut self,
        kind: HotkeyKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let was = self.recording.replace(kind).is_some();
        self.shortcut_error = None;
        self.focus.focus(window, cx);
        if !was {
            cx.emit(SettingsEvent::RecordingShortcut(true));
        }
        cx.notify();
    }

    fn stop_recording(&mut self, cx: &mut Context<Self>) {
        if self.recording.take().is_some() {
            cx.emit(SettingsEvent::RecordingShortcut(false));
        }
        cx.notify();
    }

    /// A key while recording: Esc cancels; a valid combination is saved.
    pub fn record_key(&mut self, keystroke: &Keystroke, cx: &mut Context<Self>) {
        let Some(kind) = self.recording else {
            return;
        };
        let m = &keystroke.modifiers;
        if keystroke.key == "escape" && !(m.control || m.alt || m.shift || m.platform) {
            self.stop_recording(cx);
            return;
        }
        match crate::shortcut::Shortcut::from_keystroke(keystroke) {
            Ok(None) => {}
            Ok(Some(shortcut)) => {
                let other = match kind {
                    HotkeyKind::AutoType => self.hotkeys.quick_search(),
                    HotkeyKind::QuickSearch => self.hotkeys.autotype(),
                };
                if other == shortcut {
                    self.shortcut_error = Some((kind, t!("shortcuts.in_use").into()));
                    cx.notify();
                    return;
                }
                self.set_hotkey(kind, Some(shortcut), cx);
                self.stop_recording(cx);
            }
            Err(e) => {
                self.shortcut_error = Some((kind, e.message().into()));
                cx.notify();
            }
        }
    }

    /// Saves a shortcut (`None` = back to the default).
    pub fn set_hotkey(
        &mut self,
        kind: HotkeyKind,
        shortcut: Option<crate::shortcut::Shortcut>,
        cx: &mut Context<Self>,
    ) {
        let text = match (kind, &shortcut) {
            (_, Some(s)) => s.to_storage(),
            (HotkeyKind::AutoType, None) => crate::shortcut::DEFAULT_AUTOTYPE.into(),
            (HotkeyKind::QuickSearch, None) => crate::shortcut::DEFAULT_QUICK_SEARCH.into(),
        };
        match kind {
            HotkeyKind::AutoType => self.hotkeys.autotype = text,
            HotkeyKind::QuickSearch => self.hotkeys.quick_search = text,
        }
        self.shortcut_error = None;
        let mut settings = self.store.load();
        settings.hotkeys = self.hotkeys.clone();
        self.store.save(&settings);
        crate::shortcut::set_current(self.hotkeys.clone(), cx);
        cx.emit(SettingsEvent::Hotkeys);
        cx.notify();
    }

    /// The shortcut's control: its keys (click to record) and "Default".
    fn shortcut_control(&self, kind: HotkeyKind, cx: &mut Context<Self>) -> impl IntoElement {
        let (current, default) = match kind {
            HotkeyKind::AutoType => (self.hotkeys.autotype(), crate::shortcut::DEFAULT_AUTOTYPE),
            HotkeyKind::QuickSearch => (
                self.hotkeys.quick_search(),
                crate::shortcut::DEFAULT_QUICK_SEARCH,
            ),
        };
        let recording = self.recording == Some(kind);
        let accent = crate::appearance::accent(cx);
        let theme = cx.theme();
        let label: SharedString = if recording {
            t!("shortcuts.press").into()
        } else {
            current.label().into()
        };
        let error = self
            .shortcut_error
            .as_ref()
            .filter(|(k, _)| *k == kind)
            .map(|(_, e)| e.clone());
        v_flex()
            .items_end()
            .gap_1()
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        div()
                            .id(kind.id())
                            .test_support()
                            .role(Role::Button)
                            .aria_label(label.clone())
                            .min_w(px(150.))
                            .px_3()
                            .py_1p5()
                            .rounded(theme.radius)
                            .border_1()
                            .border_color(if recording { accent } else { theme.border })
                            .bg(if recording {
                                accent.opacity(0.12)
                            } else {
                                theme.secondary
                            })
                            .text_sm()
                            .font_family(theme.mono_font_family.clone())
                            .text_center()
                            .text_color(if recording { accent } else { theme.foreground })
                            .cursor_pointer()
                            .hover(|s| s.opacity(0.85))
                            .child(label)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if this.recording == Some(kind) {
                                    this.stop_recording(cx);
                                } else {
                                    this.record_shortcut(kind, window, cx);
                                }
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("{}-reset", kind.id())))
                            .ghost()
                            .small()
                            .icon(IconName::Undo2)
                            .tooltip(t!("shortcuts.reset"))
                            .disabled(current.to_storage() == default)
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.set_hotkey(kind, None, cx)),
                            ),
                    ),
            )
            .children(error.map(|e| {
                div()
                    .id(SharedString::from(format!("{}-error", kind.id())))
                    .test_support()
                    .aria_label(e.clone())
                    .max_w(px(260.))
                    .text_xs()
                    .text_color(cx.theme().danger)
                    .child(e)
            }))
    }

    /// Opacity of the floating windows; applies the next time one opens.
    pub fn set_hud_opacity(&mut self, percent: u32, cx: &mut Context<Self>) {
        self.hud_opacity = percent;
        let mut settings = self.store.load();
        settings.hud_opacity = percent;
        self.store.save(&settings);
        crate::quick_search::set_opacity(percent, cx);
        cx.notify();
    }

    /// "Start with Windows" (a value in the registry, the same one the installer writes).
    pub fn set_start_with_windows(&mut self, on: bool, cx: &mut Context<Self>) {
        match crate::autostart::set(on) {
            Ok(()) => self.start_with_windows = on,
            Err(e) => crate::toast::error(e, cx),
        }
        cx.notify();
    }

    pub fn set_close_to_tray(&mut self, on: bool, cx: &mut Context<Self>) {
        self.close_to_tray = on;
        let mut settings = self.store.load();
        settings.close_to_tray = on;
        self.store.save(&settings);
        cx.notify();
    }

    /// Turns the remembered session on/off; turning it off deletes what was stored.
    pub fn set_remember_session(&mut self, on: bool, cx: &mut Context<Self>) {
        self.remember_session = on;
        let mut settings = self.store.load();
        settings.remember_session = on;
        self.store.save(&settings);
        if !on {
            crate::saved_session::clear(&self.store);
        }
        cx.notify();
    }

    /// Turns the browser extension on/off. In the real app, registers Scout Vault
    /// in Chrome/Edge/Brave (current user only) or undoes the registration.
    pub fn set_browser_fill(&mut self, on: bool, cx: &mut Context<Self>) {
        self.browser_error = None;
        if crate::browser::system_enabled() {
            if let Some(dir) = self.store.dir() {
                if on {
                    if let Err(e) = crate::browser::register(&dir) {
                        self.browser_error = Some(e.into());
                        cx.notify();
                        return;
                    }
                } else {
                    crate::browser::unregister(&dir);
                }
            }
        }
        self.browser_fill = on;
        let mut settings = self.store.load();
        settings.browser_fill = on;
        self.store.save(&settings);
        cx.notify();
    }

    pub fn autotype(&self) -> AutoTypeSettings {
        self.autotype
    }

    pub fn set_autotype(&mut self, next: AutoTypeSettings, cx: &mut Context<Self>) {
        self.autotype = next;
        let mut settings = self.store.load();
        settings.autotype = next;
        self.store.save(&settings);
        cx.notify();
    }

    /// A free accent color (`None` goes back to the chosen built-in one).
    pub fn set_custom_accent(
        &mut self,
        rgb: Option<crate::colors::Rgb>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.update(|a| a.custom_accent = rgb, window, cx);
    }

    fn update(
        &mut self,
        edit: impl FnOnce(&mut Appearance),
        window: &mut Window,
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
                    .child(div().text_sm().font_semibold().child(title))
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
        let custom = self.appearance.custom_swatch(dark);
        let _ = window;
        let presets: Vec<Hsla> = Accent::ALL.iter().map(|a| a.swatch(dark)).collect();
        let picker = ColorPicker::new(&self.accent_picker)
            .featured_colors(presets)
            .icon(IconName::Plus)
            .small()
            .accessibility_label(t!("settings.custom_accent"));
        let custom_swatch = custom.map(|color| {
            div()
                .id("accent-custom")
                .test_support()
                .role(Role::Button)
                .aria_label(SharedString::from(t!("settings.custom_accent")))
                .size(px(26.))
                .p(px(3.))
                .rounded_full()
                .border_2()
                .border_color(color)
                .child(div().size_full().rounded_full().bg(color))
        });
        h_flex()
            .gap_2()
            .items_center()
            .children(Accent::ALL.map(|accent| {
                let selected = accent == current && custom.is_none();
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
                    .active(|el| el.opacity(0.7))
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
                        this.update(
                            |a| {
                                a.accent = accent;
                                a.custom_accent = None;
                            },
                            window,
                            cx,
                        )
                    }))
                    // The chosen color's ring appears growing from the inside.
                    .with_animation(
                        crate::motion::key("cor", format!("{}:{selected}", accent.id())),
                        Animation::new(crate::motion::NORMAL).with_easing(ease_out_quint()),
                        move |el, t| {
                            if selected {
                                el.p(px(3. + 3. * (1.0 - t)))
                            } else {
                                el
                            }
                        },
                    )
            }))
            .children(custom_swatch)
            .child(picker)
    }
}

/// "1 day", "7 days"…
pub(crate) fn days_label(n: u32) -> SharedString {
    if n == 1 {
        t!("duration.days_one").into()
    } else {
        t!("duration.days", n = n).into()
    }
}

/// "5 min", "1 h"…
pub(crate) fn minutes_label(n: u32) -> SharedString {
    if n >= 60 && n.is_multiple_of(60) {
        t!("duration.hours", n = n / 60).into()
    } else {
        t!("duration.minutes", n = n).into()
    }
}

/// "10 s", "2 min"…
pub(crate) fn seconds_label(n: u32) -> SharedString {
    if n >= 60 && n.is_multiple_of(60) {
        minutes_label(n / 60)
    } else {
        t!("duration.seconds", n = n).into()
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

        let motion_control = {
            let this = this.clone();
            ui::segmented(
                "settings-motion",
                Motion::ALL
                    .iter()
                    .map(|m| Segment {
                        id: format!("motion-{}", m.id()).into(),
                        label: m.label().into(),
                        icon: None,
                        selected: *m == a.motion,
                    })
                    .collect(),
                move |ix, window, cx| {
                    this.update(cx, |v, cx| {
                        v.update(|a| a.motion = Motion::ALL[ix], window, cx)
                    })
                    .ok();
                },
                cx,
            )
        };

        let language_control = {
            let this = this.clone();
            // "System" first; then each language by its own name.
            let mut codes = vec![""];
            codes.extend(crate::i18n::Language::ALL.iter().map(|l| l.code()));
            let current = self.language.clone();
            ui::segmented(
                "settings-language",
                codes
                    .iter()
                    .map(|code| Segment {
                        id: format!("language-{}", if code.is_empty() { "system" } else { code })
                            .into(),
                        label: match crate::i18n::Language::from_code(code) {
                            Some(l) => l.native_name().into(),
                            None => t!("settings.language_system").into(),
                        },
                        icon: code.is_empty().then_some(IconName::Monitor),
                        selected: crate::i18n::Language::from_code(&current)
                            .map(|l| l.code())
                            .unwrap_or("")
                            == *code,
                    })
                    .collect(),
                move |ix, _, cx| {
                    let code = codes[ix];
                    this.update(cx, |v, cx| v.set_language(code, cx)).ok();
                },
                cx,
            )
        };

        let hud_control = h_flex()
            .id("settings-hud-opacity")
            .test_support()
            .gap_3()
            .w(px(220.))
            .child(
                div().flex_1().child(
                    Slider::new(&self.hud_slider)
                        .horizontal()
                        .accessibility_label(t!("settings.hud_opacity")),
                ),
            )
            .child(
                div()
                    .id("settings-hud-opacity-value")
                    .test_support()
                    .w(px(40.))
                    .text_sm()
                    .text_right()
                    .child(format!("{}%", self.hud_opacity)),
            );
        let appearance_group = Self::group(
            t!("settings.group_appearance"),
            vec![
                Self::row(
                    t!("settings.language"),
                    t!("settings.language_desc"),
                    language_control,
                    cx,
                ),
                Self::row(
                    t!("settings.theme"),
                    t!("settings.theme_desc"),
                    theme_control,
                    cx,
                ),
                Self::row(
                    t!("settings.accent"),
                    t!("settings.accent_desc"),
                    self.render_accents(window, cx),
                    cx,
                ),
                Self::row(
                    t!("settings.text_size"),
                    t!("settings.text_size_desc"),
                    text_control,
                    cx,
                ),
                Self::row(
                    t!("settings.corners"),
                    t!("settings.corners_desc"),
                    corners_control,
                    cx,
                ),
                Self::row(
                    t!("settings.motion"),
                    t!("settings.motion_desc"),
                    motion_control,
                    cx,
                ),
                Self::row(
                    t!("settings.hud_opacity"),
                    t!("settings.hud_opacity_desc"),
                    hud_control,
                    cx,
                ),
            ],
            cx,
        );

        let at = self.autotype;
        let autotype_group = Self::group(
            t!("settings.group_autotype"),
            vec![
                Self::row(
                    t!("settings.autotype"),
                    t!("settings.autotype_desc"),
                    Switch::new("autotype-enabled")
                        .checked(at.enabled)
                        .on_click(cx.listener(move |this, v: &bool, _, cx| {
                            this.set_autotype(AutoTypeSettings { enabled: *v, ..at }, cx)
                        })),
                    cx,
                ),
                Self::row(
                    t!("settings.autotype_enter"),
                    t!("settings.autotype_enter_desc"),
                    Switch::new("autotype-enter")
                        .checked(at.press_enter)
                        .disabled(!at.enabled)
                        .on_click(cx.listener(move |this, v: &bool, _, cx| {
                            this.set_autotype(
                                AutoTypeSettings {
                                    press_enter: *v,
                                    ..at
                                },
                                cx,
                            )
                        })),
                    cx,
                ),
            ],
            cx,
        );
        // About the project: version, links, license and credits.
        let link = |id: &'static str, icon: IconName, label: SharedString, url: &'static str| {
            Button::new(id)
                .ghost()
                .small()
                .icon(icon)
                .label(label)
                .on_click(move |_, _, cx| {
                    if let Err(e) = crate::sso::open_browser(url) {
                        crate::toast::error(e, cx);
                    }
                })
        };
        let about_hero = h_flex().gap_3().child(ui::brand_mark(px(40.), cx)).child(
            v_flex()
                .gap_0p5()
                .child(
                    h_flex()
                        .gap_2()
                        .items_baseline()
                        .child(div().text_base().font_semibold().child("Scout Vault"))
                        .child(
                            div()
                                .id("about-version")
                                .test_support()
                                .aria_label(SharedString::from(env!("CARGO_PKG_VERSION")))
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(t!("about.version", v = env!("CARGO_PKG_VERSION"))),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(t!("about.tagline")),
                ),
        );
        let about_links = h_flex()
            .flex_wrap()
            .gap_1()
            .child(link(
                "about-source",
                IconName::Code,
                t!("about.source").into(),
                "https://github.com/qedrohenrique/scout-vault",
            ))
            .child(link(
                "about-releases",
                IconName::Sparkles,
                t!("about.releases").into(),
                "https://github.com/qedrohenrique/scout-vault/releases",
            ))
            .child(link(
                "about-issues",
                IconName::MessageCircle,
                t!("about.issues").into(),
                "https://github.com/qedrohenrique/scout-vault/issues/new/choose",
            ));
        let about_group = Self::group(
            t!("settings.group_about"),
            vec![
                div().px_4().py_3p5().child(about_hero),
                div().px_3().py_2().child(about_links),
                Self::row(
                    t!("about.license"),
                    t!("about.license_desc"),
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("MIT"),
                    cx,
                ),
                Self::row(t!("about.credits"), t!("about.credits_desc"), div(), cx),
                div()
                    .px_4()
                    .py_3()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(t!("about.disclaimer")),
            ],
            cx,
        );
        let shortcuts_group = Self::group(
            t!("settings.group_shortcuts"),
            vec![
                Self::row(
                    t!("shortcuts.autotype"),
                    t!("shortcuts.autotype_desc"),
                    self.shortcut_control(HotkeyKind::AutoType, cx),
                    cx,
                ),
                Self::row(
                    t!("shortcuts.quick_search"),
                    t!("shortcuts.quick_search_desc"),
                    self.shortcut_control(HotkeyKind::QuickSearch, cx),
                    cx,
                ),
            ],
            cx,
        );
        let browser_group = {
            let mut rows = vec![Self::row(
                t!("settings.browser_fill"),
                t!("settings.browser_fill_desc"),
                Switch::new("browser-enabled")
                    .checked(self.browser_fill)
                    .on_click(cx.listener(|this, v: &bool, _, cx| this.set_browser_fill(*v, cx))),
                cx,
            )];
            if self.browser_fill {
                let dir = crate::browser::extension_dir();
                rows.push(
                    v_flex()
                        .gap_2()
                        .px_4()
                        .py_3p5()
                        .text_sm()
                        .child(
                            div()
                                .font_semibold()
                                .child(t!("settings.install_extension")),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(t!("settings.install_extension_desc")),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_xs()
                                        .font_family(cx.theme().mono_font_family.clone())
                                        .child(dir.to_string_lossy().into_owned()),
                                )
                                .child(
                                    Button::new("browser-open-folder")
                                        .outline()
                                        .small()
                                        .label(t!("settings.open_folder"))
                                        .on_click(move |_, _, cx| {
                                            cx.reveal_path(&dir.join("manifest.json"))
                                        }),
                                ),
                        ),
                );
            }
            if let Some(e) = &self.browser_error {
                rows.push(
                    div()
                        .px_4()
                        .py_3()
                        .text_sm()
                        .text_color(cx.theme().danger)
                        .child(e.clone()),
                );
            }
            Self::group(t!("settings.group_browser"), rows, cx)
        };
        let unlock_control = self.number_choice(
            NumberChoice {
                id: "settings-unlock-memory",
                prefix: "unlock",
                options: &[0, 1, 7, 30],
                zero: t!("settings.always"),
                label: days_label,
                current: self.unlock_memory_days,
                set: Self::set_unlock_memory_days,
            },
            cx,
        );
        let relogin_control = self.number_choice(
            NumberChoice {
                id: "settings-relogin",
                prefix: "relogin",
                options: &[7, 30, 90, 0],
                zero: t!("settings.never"),
                label: days_label,
                current: self.relogin_days,
                set: Self::set_relogin_days,
            },
            cx,
        );
        let lock_control = self.number_choice(
            NumberChoice {
                id: "settings-auto-lock",
                prefix: "lock",
                options: &[1, 5, 15, 60, 0],
                zero: t!("settings.never"),
                label: minutes_label,
                current: self.auto_lock_minutes,
                set: Self::set_auto_lock_minutes,
            },
            cx,
        );
        let clipboard_control = self.number_choice(
            NumberChoice {
                id: "settings-clipboard-clear",
                prefix: "clip",
                options: &[10, 30, 60, 120, 0],
                zero: t!("settings.never"),
                label: seconds_label,
                current: self.clipboard_clear_secs,
                set: Self::set_clipboard_clear_secs,
            },
            cx,
        );
        let session_group = Self::group(
            t!("settings.group_session"),
            vec![
                Self::row(
                    t!("settings.unlock_memory"),
                    t!("settings.unlock_memory_desc"),
                    unlock_control,
                    cx,
                ),
                Self::row(
                    t!("settings.relogin"),
                    t!("settings.relogin_desc"),
                    relogin_control,
                    cx,
                ),
                Self::row(
                    t!("settings.auto_lock"),
                    t!("settings.auto_lock_desc"),
                    lock_control,
                    cx,
                ),
                Self::row(
                    t!("settings.quick_search"),
                    t!("settings.quick_search_desc"),
                    Switch::new("quick-search-setting")
                        .checked(self.quick_search)
                        .on_click(
                            cx.listener(|this, v: &bool, _, cx| this.set_quick_search(*v, cx)),
                        ),
                    cx,
                ),
                Self::row(
                    t!("settings.site_icons"),
                    t!("settings.site_icons_desc"),
                    Switch::new("site-icons-setting")
                        .checked(self.site_icons)
                        .on_click(cx.listener(|this, v: &bool, _, cx| this.set_site_icons(*v, cx))),
                    cx,
                ),
                Self::row(
                    t!("settings.background_sync"),
                    t!("settings.background_sync_desc"),
                    Switch::new("background-sync")
                        .checked(self.background_sync)
                        .on_click(
                            cx.listener(|this, v: &bool, _, cx| this.set_background_sync(*v, cx)),
                        ),
                    cx,
                ),
                Self::row(
                    t!("settings.windows_hello"),
                    if crate::hello::available(cx) {
                        t!("settings.windows_hello_desc")
                    } else {
                        t!("settings.windows_hello_unavailable")
                    },
                    Switch::new("windows-hello")
                        .checked(self.windows_hello)
                        .disabled(!crate::hello::available(cx) && !self.windows_hello)
                        .on_click(
                            cx.listener(|this, v: &bool, _, cx| this.set_windows_hello(*v, cx)),
                        ),
                    cx,
                ),
                Self::row(
                    t!("settings.lock_with_windows"),
                    t!("settings.lock_with_windows_desc"),
                    Switch::new("lock-with-windows")
                        .checked(self.lock_with_windows)
                        .on_click(
                            cx.listener(|this, v: &bool, _, cx| this.set_lock_with_windows(*v, cx)),
                        ),
                    cx,
                ),
                Self::row(
                    t!("settings.lock_on_minimize"),
                    t!("settings.lock_on_minimize_desc"),
                    Switch::new("lock-on-minimize")
                        .checked(self.lock_on_minimize)
                        .on_click(
                            cx.listener(|this, v: &bool, _, cx| this.set_lock_on_minimize(*v, cx)),
                        ),
                    cx,
                ),
                Self::row(
                    t!("settings.clipboard_clear"),
                    t!("settings.clipboard_clear_desc"),
                    clipboard_control,
                    cx,
                ),
                Self::row(
                    t!("settings.start_with_windows"),
                    t!("settings.start_with_windows_desc"),
                    Switch::new("start-with-windows")
                        .checked(self.start_with_windows)
                        .on_click(
                            cx.listener(|this, v: &bool, _, cx| {
                                this.set_start_with_windows(*v, cx)
                            }),
                        ),
                    cx,
                ),
                Self::row(
                    t!("settings.close_to_tray"),
                    t!("settings.close_to_tray_desc"),
                    Switch::new("tray-on-close")
                        .checked(self.close_to_tray)
                        .on_click(
                            cx.listener(|this, v: &bool, _, cx| this.set_close_to_tray(*v, cx)),
                        ),
                    cx,
                ),
                Self::row(
                    t!("settings.remember_session"),
                    t!("settings.remember_session_desc"),
                    Switch::new("remember-session")
                        .checked(self.remember_session)
                        .on_click(
                            cx.listener(|this, v: &bool, _, cx| this.set_remember_session(*v, cx)),
                        ),
                    cx,
                ),
            ],
            cx,
        );

        let header = h_flex().child(
            v_flex()
                .gap_1()
                .child(div().text_2xl().font_semibold().child(t!("settings.title")))
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(t!("settings.subtitle")),
                ),
        );
        // "Back" stays put on the left while the page scrolls: no need to scroll
        // up to leave.
        let back = div().absolute().top_6().left_6().child(
            Button::new("settings-close")
                .ghost()
                .small()
                .icon(IconName::ArrowLeft)
                .label(t!("common.back"))
                .tooltip(t!("common.close_esc"))
                .on_click(cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::Close))),
        );

        div()
            .relative()
            .size_full()
            .track_focus(&self.focus)
            // Recording a shortcut: the keys come here first.
            .capture_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| {
                if this.recording.is_some() {
                    this.record_key(&e.keystroke, cx);
                    cx.stop_propagation();
                }
            }))
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                div().id("settings").size_full().overflow_y_scroll().child(
                    v_flex()
                        .w_full()
                        .max_w(px(680.))
                        .mx_auto()
                        .px_8()
                        .py_10()
                        .gap_8()
                        .child(header)
                        .child(appearance_group)
                        .child(session_group)
                        .child(browser_group)
                        .child(autotype_group)
                        .child(shortcuts_group)
                        .child(about_group),
                ),
            )
            .child(back)
    }
}
