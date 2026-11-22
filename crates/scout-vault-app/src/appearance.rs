//! Scout Vault appearance: light/dark theme, accent color, text size and
//! corners. Builds the gpui-kit themes from its own palette.
//!
//! The visual identity is deliberately restrained: warm neutrals (light paper
//! or deep graphite) and **one** accent color, used only where it matters —
//! selection, focus, primary button and the brand.

use std::rc::Rc;

use gpui_kit::component::theme::{Theme, ThemeConfig, ThemeMode, ThemeRegistry};
use gpui_kit::{App, Hsla, Rgba, SharedString, Window, WindowAppearance};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ThemeChoice {
    /// Follows the operating system's light/dark mode.
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeChoice {
    pub const ALL: [ThemeChoice; 3] = [Self::System, Self::Light, Self::Dark];

    pub fn id(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::System => t!("appearance.theme_system"),
            Self::Light => t!("appearance.theme_light"),
            Self::Dark => t!("appearance.theme_dark"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Accent {
    #[default]
    Jade,
    Indigo,
    Brass,
    Coral,
    Plum,
    Graphite,
}

/// The colors of an accent color in both modes.
struct AccentColors {
    light: String,
    on_light: String,
    dark: String,
    on_dark: String,
}

impl Accent {
    pub const ALL: [Accent; 6] = [
        Self::Jade,
        Self::Indigo,
        Self::Brass,
        Self::Coral,
        Self::Plum,
        Self::Graphite,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::Jade => "jade",
            Self::Indigo => "indigo",
            Self::Brass => "brass",
            Self::Coral => "coral",
            Self::Plum => "plum",
            Self::Graphite => "graphite",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Jade => t!("appearance.accent_jade"),
            Self::Indigo => t!("appearance.accent_indigo"),
            Self::Brass => t!("appearance.accent_brass"),
            Self::Coral => t!("appearance.accent_coral"),
            Self::Plum => t!("appearance.accent_plum"),
            Self::Graphite => t!("appearance.accent_graphite"),
        }
    }

    // Tones chosen so the primary button text passes 4.5:1 (WCAG AA).
    fn colors(self) -> AccentColors {
        let (light, on_light, dark, on_dark) = match self {
            Self::Jade => ("#0b7d62", "#ffffff", "#3ccf9f", "#04261b"),
            Self::Indigo => ("#4c44d4", "#ffffff", "#9497fa", "#12123a"),
            Self::Brass => ("#946010", "#ffffff", "#e3b155", "#2b1d04"),
            Self::Coral => ("#c43d2a", "#ffffff", "#ff8f78", "#3a0d05"),
            Self::Plum => ("#7d3cbf", "#ffffff", "#c6a0f6", "#250b40"),
            Self::Graphite => ("#2a2926", "#ffffff", "#e6e4df", "#151514"),
        };
        AccentColors {
            light: light.into(),
            on_light: on_light.into(),
            dark: dark.into(),
            on_dark: on_dark.into(),
        }
    }

    /// The main color in the given mode (for swatches on the settings screen).
    pub fn swatch(self, dark: bool) -> Hsla {
        let c = self.colors();
        hex(if dark { &c.dark } else { &c.light })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TextSize {
    Small,
    #[default]
    Medium,
    Large,
}

impl TextSize {
    pub const ALL: [TextSize; 3] = [Self::Small, Self::Medium, Self::Large];

    pub fn id(self) -> &'static str {
        match self {
            Self::Small => "small",
            Self::Medium => "medium",
            Self::Large => "large",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Small => t!("appearance.text_small"),
            Self::Medium => t!("appearance.text_medium"),
            Self::Large => t!("appearance.text_large"),
        }
    }

    fn px(self) -> f32 {
        match self {
            Self::Small => 14.,
            Self::Medium => 15.,
            Self::Large => 17.,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Corners {
    Square,
    #[default]
    Soft,
    Round,
}

impl Corners {
    pub const ALL: [Corners; 3] = [Self::Square, Self::Soft, Self::Round];

    pub fn id(self) -> &'static str {
        match self {
            Self::Square => "square",
            Self::Soft => "soft",
            Self::Round => "round",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Square => t!("appearance.corners_square"),
            Self::Soft => t!("appearance.corners_soft"),
            Self::Round => t!("appearance.corners_round"),
        }
    }

    /// (common radius, radius of cards and dialogs)
    fn radii(self) -> (usize, usize) {
        match self {
            Self::Square => (2, 3),
            Self::Soft => (6, 10),
            Self::Round => (10, 16),
        }
    }
}

/// Background art animations on the entry screens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Motion {
    #[default]
    On,
    Off,
}

impl Motion {
    pub const ALL: [Motion; 2] = [Self::On, Self::Off];

    pub fn id(self) -> &'static str {
        match self {
            Self::On => "on",
            Self::Off => "off",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::On => t!("appearance.motion_on"),
            Self::Off => t!("appearance.motion_off"),
        }
    }
}

/// Appearance preferences. They live in `settings.json` (nothing sensitive).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    pub theme: ThemeChoice,
    pub accent: Accent,
    pub text_size: TextSize,
    pub corners: Corners,
    pub motion: Motion,
    /// A free accent color (color picker); `None` uses [`Self::accent`].
    pub custom_accent: Option<crate::colors::Rgb>,
}

impl Appearance {
    fn accent_colors(&self) -> AccentColors {
        match self.custom_accent {
            Some(rgb) => {
                let (light, on_light, dark, on_dark) = crate::colors::accent_tones(rgb);
                AccentColors {
                    light,
                    on_light,
                    dark,
                    on_dark,
                }
            }
            None => self.accent.colors(),
        }
    }

    /// The free accent color as it shows in the given mode.
    pub fn custom_swatch(&self, dark: bool) -> Option<Hsla> {
        self.custom_accent?;
        let c = self.accent_colors();
        Some(hex(if dark { &c.dark } else { &c.light }))
    }
}

/// Neutrals of each mode.
struct Neutrals {
    background: &'static str,
    surface: &'static str,
    sidebar: &'static str,
    raised: &'static str,
    hover: &'static str,
    pressed: &'static str,
    border: &'static str,
    input_border: &'static str,
    foreground: &'static str,
    muted: &'static str,
    scrollbar: &'static str,
}

const LIGHT: Neutrals = Neutrals {
    background: "#fbfaf8",
    surface: "#ffffff",
    sidebar: "#f3f1ec",
    raised: "#efece6",
    hover: "#ebe8e1",
    pressed: "#e2ded5",
    border: "#e6e2da",
    input_border: "#dcd7cd",
    foreground: "#1c1b19",
    // 4.6:1 or more on every light surface (WCAG AA), hover included.
    muted: "#6b665e",
    scrollbar: "#b9b3a8",
};

const DARK: Neutrals = Neutrals {
    background: "#121314",
    surface: "#18191a",
    sidebar: "#0d0e0f",
    raised: "#1e2021",
    hover: "#222426",
    pressed: "#2a2c2e",
    border: "#25272a",
    input_border: "#303336",
    foreground: "#ecebe8",
    muted: "#8d918f",
    scrollbar: "#45494c",
};

/// Builds a mode's theme from gpui-kit's default theme, swapping the
/// colors for Scout Vault's palette.
fn build(base: &ThemeConfig, mode: ThemeMode, a: &Appearance) -> ThemeConfig {
    let dark = mode.is_dark();
    let n = if dark { &DARK } else { &LIGHT };
    let c = a.accent_colors();
    let (accent, on_accent): (&str, &str) = if dark {
        (&c.dark, &c.on_dark)
    } else {
        (&c.light, &c.on_light)
    };
    let tint = |alpha: &str| format!("{accent}{alpha}");

    let colors: Vec<(&str, String)> = vec![
        ("background", n.background.into()),
        ("foreground", n.foreground.into()),
        ("border", n.border.into()),
        ("input.border", n.input_border.into()),
        ("caret", accent.into()),
        ("ring", accent.into()),
        ("selection.background", tint("4d")),
        ("muted.background", n.raised.into()),
        ("muted.foreground", n.muted.into()),
        // gpui-kit's "accent" is the hover background of menus and lists.
        ("accent.background", n.hover.into()),
        ("accent.foreground", n.foreground.into()),
        ("primary.background", accent.into()),
        ("primary.hover.background", tint("e0")),
        ("primary.active.background", tint("c8")),
        ("primary.foreground", on_accent.into()),
        ("secondary.background", n.raised.into()),
        ("secondary.hover.background", n.hover.into()),
        ("secondary.active.background", n.pressed.into()),
        ("secondary.foreground", n.foreground.into()),
        ("popover.background", n.surface.into()),
        ("popover.foreground", n.foreground.into()),
        ("group_box.background", n.surface.into()),
        ("group_box.foreground", n.foreground.into()),
        ("accordion.background", n.surface.into()),
        ("list.background", n.background.into()),
        ("list.hover.background", n.hover.into()),
        ("list.active.background", tint("1f")),
        ("list.active.border", tint("00")),
        ("list.even.background", n.background.into()),
        ("list.head.background", n.background.into()),
        ("sidebar.background", n.sidebar.into()),
        ("sidebar.foreground", n.foreground.into()),
        ("sidebar.border", n.border.into()),
        ("sidebar.accent.background", tint("1f")),
        ("sidebar.accent.foreground", accent.into()),
        ("sidebar.primary.background", accent.into()),
        ("sidebar.primary.foreground", on_accent.into()),
        ("link.foreground", accent.into()),
        ("link.hover.foreground", tint("cc")),
        ("link.active.foreground", accent.into()),
        ("switch.background", n.pressed.into()),
        ("slider.bar.background", accent.into()),
        ("slider.thumb.background", n.surface.into()),
        ("progress_bar.background", accent.into()),
        ("skeleton.background", n.raised.into()),
        ("scrollbar.background", format!("{}00", n.background)),
        ("scrollbar.thumb.background", format!("{}cc", n.scrollbar)),
        ("scrollbar.thumb.hover.background", n.scrollbar.into()),
        ("title_bar.background", n.sidebar.into()),
        ("title_bar.border", n.border.into()),
        ("status_bar.background", n.sidebar.into()),
        ("status_bar.border", n.border.into()),
        ("window.border", n.border.into()),
        ("tab_bar.background", n.raised.into()),
        ("tab_bar.segmented.background", n.raised.into()),
        ("tab.active.background", n.surface.into()),
        ("tab.active.foreground", n.foreground.into()),
        ("tab.foreground", n.muted.into()),
        ("drag_border", accent.into()),
        ("drop_target.background", tint("26")),
        (
            "overlay",
            if dark { "#00000080" } else { "#1c1b1926" }.into(),
        ),
    ];

    let (radius, radius_lg) = a.corners.radii();
    let name = if dark {
        "Scout Vault Escuro"
    } else {
        "Scout Vault Claro"
    };

    let Ok(mut value) = serde_json::to_value(base) else {
        return base.clone();
    };
    value["name"] = name.into();
    // Key names as in gpui-kit's theme JSON.
    value["font.size"] = a.text_size.px().into();
    value["mono_font.size"] = (a.text_size.px() - 1.).into();
    value["radius"] = radius.into();
    value["radius.lg"] = radius_lg.into();
    value["shadow"] = false.into();
    if let Some(map) = value["colors"].as_object_mut() {
        for (key, color) in colors {
            map.insert(key.into(), color.into());
        }
    }
    serde_json::from_value(value).unwrap_or_else(|_| base.clone())
}

pub fn mode_for(choice: ThemeChoice, system: WindowAppearance) -> ThemeMode {
    match choice {
        ThemeChoice::System => system.into(),
        ThemeChoice::Light => ThemeMode::Light,
        ThemeChoice::Dark => ThemeMode::Dark,
    }
}

/// Applies the appearance to the global theme and redraws every window.
pub fn apply(appearance: &Appearance, window: Option<&Window>, cx: &mut App) {
    let registry = ThemeRegistry::global(cx);
    let light = build(registry.default_light_theme(), ThemeMode::Light, appearance);
    let dark = build(registry.default_dark_theme(), ThemeMode::Dark, appearance);
    let system = window
        .map(|w| w.appearance())
        .unwrap_or_else(|| cx.window_appearance());
    let mode = mode_for(appearance.theme, system);
    Theme::update(cx, |theme| {
        theme.light_theme = Rc::new(light);
        theme.dark_theme = Rc::new(dark);
    });
    // `change` reloads the mode's theme even if the mode didn't change.
    Theme::change(mode, None, cx);
    crate::backdrop::set_motion(appearance.motion == Motion::On, cx);
    // Applies to every interface animation (GPUI jumps straight to the final state).
    cx.set_reduce_motion(appearance.motion == Motion::Off);
}

/// The accent color in use (for hand-drawn details).
pub fn accent(cx: &App) -> Hsla {
    cx.global::<Theme>().primary
}

/// The translucent accent color, for selection backgrounds.
pub fn accent_tint(cx: &App, opacity: f32) -> Hsla {
    accent(cx).opacity(opacity)
}

fn hex(s: &str) -> Hsla {
    let s = s.trim_start_matches('#');
    let v = u32::from_str_radix(s, 16).unwrap_or(0);
    Rgba {
        r: ((v >> 16) & 0xff) as f32 / 255.,
        g: ((v >> 8) & 0xff) as f32 / 255.,
        b: (v & 0xff) as f32 / 255.,
        a: 1.,
    }
    .into()
}

/// Short label of the effective theme (for tests and debugging).
pub fn theme_name(cx: &App) -> SharedString {
    cx.global::<Theme>().theme_name().clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appearance_defaults_when_missing_from_json() {
        let a: Appearance = serde_json::from_str("{}").unwrap();
        assert_eq!(a, Appearance::default());
        let a: Appearance = serde_json::from_str(r#"{"accent":"Coral"}"#).unwrap();
        assert_eq!(a.accent, Accent::Coral);
        assert_eq!(a.theme, ThemeChoice::System);
    }

    #[test]
    fn explicit_choice_ignores_system() {
        assert_eq!(
            mode_for(ThemeChoice::Light, WindowAppearance::Dark),
            ThemeMode::Light
        );
        assert_eq!(
            mode_for(ThemeChoice::Dark, WindowAppearance::Light),
            ThemeMode::Dark
        );
        assert!(mode_for(ThemeChoice::System, WindowAppearance::Dark).is_dark());
    }
}
