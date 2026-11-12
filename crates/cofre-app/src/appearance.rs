//! Aparência do Cofre: tema claro/escuro, cor de destaque, tamanho do texto e
//! cantos. Gera os temas do gpui-kit a partir de uma paleta própria.
//!
//! A identidade visual é deliberadamente contida: neutros quentes (papel claro
//! ou grafite profundo) e **uma** cor de destaque, usada só onde importa —
//! seleção, foco, botão principal e a marca.

use std::rc::Rc;

use gpui_kit::component::theme::{Theme, ThemeConfig, ThemeMode, ThemeRegistry};
use gpui_kit::{App, Hsla, Rgba, SharedString, Window, WindowAppearance};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ThemeChoice {
    /// Acompanha o claro/escuro do sistema operacional.
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
            Self::System => "Sistema",
            Self::Light => "Claro",
            Self::Dark => "Escuro",
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

/// As cores de uma cor de destaque nos dois modos.
struct AccentColors {
    light: &'static str,
    on_light: &'static str,
    dark: &'static str,
    on_dark: &'static str,
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
            Self::Jade => "Jade",
            Self::Indigo => "Índigo",
            Self::Brass => "Latão",
            Self::Coral => "Coral",
            Self::Plum => "Ameixa",
            Self::Graphite => "Grafite",
        }
    }

    // Tons escolhidos para o texto do botão principal passar de 4,5:1 (WCAG AA).
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
            light,
            on_light,
            dark,
            on_dark,
        }
    }

    /// A cor principal no modo dado (para amostras na tela de configurações).
    pub fn swatch(self, dark: bool) -> Hsla {
        let c = self.colors();
        hex(if dark { c.dark } else { c.light })
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
            Self::Small => "Pequeno",
            Self::Medium => "Padrão",
            Self::Large => "Grande",
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
            Self::Square => "Retos",
            Self::Soft => "Suaves",
            Self::Round => "Redondos",
        }
    }

    /// (raio comum, raio de cartões e diálogos)
    fn radii(self) -> (usize, usize) {
        match self {
            Self::Square => (2, 3),
            Self::Soft => (6, 10),
            Self::Round => (10, 16),
        }
    }
}

/// Preferências de aparência. Ficam no `settings.json` (nada sensível).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    pub theme: ThemeChoice,
    pub accent: Accent,
    pub text_size: TextSize,
    pub corners: Corners,
}

/// Neutros de cada modo.
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
    muted: "#77726a",
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

/// Monta o tema de um modo a partir do tema padrão do gpui-kit, trocando as
/// cores pela paleta do Cofre.
fn build(base: &ThemeConfig, mode: ThemeMode, a: &Appearance) -> ThemeConfig {
    let dark = mode.is_dark();
    let n = if dark { &DARK } else { &LIGHT };
    let c = a.accent.colors();
    let (accent, on_accent) = if dark {
        (c.dark, c.on_dark)
    } else {
        (c.light, c.on_light)
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
        // "accent" do gpui-kit é o fundo de hover de menus e listas.
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
    let name = if dark { "Cofre Escuro" } else { "Cofre Claro" };

    let Ok(mut value) = serde_json::to_value(base) else {
        return base.clone();
    };
    value["name"] = name.into();
    // Nomes das chaves como no JSON de temas do gpui-kit.
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

/// Aplica a aparência ao tema global e redesenha todas as janelas.
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
    // `change` recarrega o tema do modo mesmo se o modo não mudou.
    Theme::change(mode, None, cx);
}

/// A cor de destaque em uso (para detalhes desenhados à mão).
pub fn accent(cx: &App) -> Hsla {
    cx.global::<Theme>().primary
}

/// A cor de destaque translúcida, para fundos de seleção.
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

/// Rótulo curto do tema efetivo (para testes e depuração).
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
