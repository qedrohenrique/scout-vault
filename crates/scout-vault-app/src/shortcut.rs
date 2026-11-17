//! Global shortcuts the person can change (auto-type and quick search).
//!
//! Stored in the settings as text in a fixed, English form (`"Ctrl+Alt+A"`,
//! `"Ctrl+Shift+Space"`); shown in the interface's language (`Espaço`). A global
//! shortcut needs Ctrl, Alt or Win — otherwise it would steal ordinary typing.

use gpui_kit::{App, Global, Keystroke};
use serde::{Deserialize, Serialize};

pub const DEFAULT_AUTOTYPE: &str = "Ctrl+Alt+A";
pub const DEFAULT_QUICK_SEARCH: &str = "Ctrl+Shift+Space";

/// A key combination (modifiers + one key).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shortcut {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    /// `A`–`Z`, `0`–`9`, `F1`–`F24` or `Space`.
    pub key: String,
}

/// Why a pressed combination can't be a global shortcut.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShortcutError {
    /// Only Shift (or nothing) with the key: would steal ordinary typing.
    NeedsModifier,
    /// A key that can't be registered (arrows, symbols…).
    UnsupportedKey,
}

impl ShortcutError {
    pub fn message(&self) -> &'static str {
        match self {
            Self::NeedsModifier => t!("shortcuts.needs_modifier"),
            Self::UnsupportedKey => t!("shortcuts.unsupported_key"),
        }
    }
}

/// Normalizes a key name (`a`, `space`, `f5`) to the stored form (`A`, `Space`, `F5`).
fn canonical_key(key: &str) -> Option<String> {
    let lower = key.trim().to_ascii_lowercase();
    if lower == "space" {
        return Some("Space".into());
    }
    let mut chars = lower.chars();
    if let (Some(c), None) = (chars.next(), chars.clone().next()) {
        if c.is_ascii_alphanumeric() {
            return Some(c.to_ascii_uppercase().to_string());
        }
    }
    let n: u32 = lower.strip_prefix('f')?.parse().ok()?;
    (1..=24).contains(&n).then(|| format!("F{n}"))
}

impl Shortcut {
    /// Reads the stored form (`"Ctrl+Alt+A"`). `None` if it isn't valid.
    pub fn parse(text: &str) -> Option<Self> {
        let mut s = Shortcut {
            ctrl: false,
            alt: false,
            shift: false,
            win: false,
            key: String::new(),
        };
        for part in text.split('+').map(str::trim) {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => s.ctrl = true,
                "alt" => s.alt = true,
                "shift" => s.shift = true,
                "win" | "super" | "meta" => s.win = true,
                other => {
                    if !s.key.is_empty() {
                        return None;
                    }
                    s.key = canonical_key(other)?;
                }
            }
        }
        s.validate().ok()?;
        Some(s)
    }

    /// From a key pressed in the app. `Ok(None)`: only modifiers so far (keep waiting).
    pub fn from_keystroke(k: &Keystroke) -> Result<Option<Self>, ShortcutError> {
        if matches!(
            k.key.as_str(),
            "control" | "alt" | "shift" | "platform" | "function" | "super" | "win"
        ) {
            return Ok(None);
        }
        let key = canonical_key(&k.key).ok_or(ShortcutError::UnsupportedKey)?;
        let s = Shortcut {
            ctrl: k.modifiers.control,
            alt: k.modifiers.alt,
            shift: k.modifiers.shift,
            win: k.modifiers.platform,
            key,
        };
        s.validate()?;
        Ok(Some(s))
    }

    fn validate(&self) -> Result<(), ShortcutError> {
        if self.key.is_empty() || canonical_key(&self.key).is_none() {
            return Err(ShortcutError::UnsupportedKey);
        }
        if !(self.ctrl || self.alt || self.win) {
            return Err(ShortcutError::NeedsModifier);
        }
        Ok(())
    }

    /// Stored form, always in the same order: `Ctrl+Alt+Shift+Win+Key`.
    pub fn to_storage(&self) -> String {
        let mut parts: Vec<&str> = Vec::new();
        if self.ctrl {
            parts.push("Ctrl");
        }
        if self.alt {
            parts.push("Alt");
        }
        if self.shift {
            parts.push("Shift");
        }
        if self.win {
            parts.push("Win");
        }
        parts.push(&self.key);
        parts.join("+")
    }

    /// As shown to the person (`Ctrl+Shift+Espaço`).
    pub fn label(&self) -> String {
        let storage = self.to_storage();
        if self.key == "Space" {
            format!(
                "{}{}",
                storage.trim_end_matches("Space"),
                t!("shortcuts.space")
            )
        } else {
            storage
        }
    }

    /// Windows virtual-key code of the key.
    pub fn virtual_key(&self) -> u32 {
        match self.key.as_str() {
            "Space" => 0x20,
            k if k.len() == 1 => k.as_bytes()[0] as u32, // 'A'–'Z' and '0'–'9' match their VK
            k => 0x6F + k[1..].parse::<u32>().unwrap_or(1), // F1 = 0x70
        }
    }
}

/// Stored shortcuts, with the defaults when missing or invalid.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hotkeys {
    #[serde(default = "default_autotype")]
    pub autotype: String,
    #[serde(default = "default_quick_search")]
    pub quick_search: String,
}

fn default_autotype() -> String {
    DEFAULT_AUTOTYPE.into()
}

fn default_quick_search() -> String {
    DEFAULT_QUICK_SEARCH.into()
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self {
            autotype: default_autotype(),
            quick_search: default_quick_search(),
        }
    }
}

impl Hotkeys {
    pub fn autotype(&self) -> Shortcut {
        Shortcut::parse(&self.autotype)
            .unwrap_or_else(|| Shortcut::parse(DEFAULT_AUTOTYPE).unwrap())
    }

    pub fn quick_search(&self) -> Shortcut {
        Shortcut::parse(&self.quick_search)
            .unwrap_or_else(|| Shortcut::parse(DEFAULT_QUICK_SEARCH).unwrap())
    }
}

/// The shortcuts in use, for the texts that mention them.
struct Current(Hotkeys);

impl Global for Current {}

pub fn set_current(hotkeys: Hotkeys, cx: &mut App) {
    cx.set_global(Current(hotkeys));
}

fn current(cx: &App) -> Hotkeys {
    cx.try_global::<Current>()
        .map(|c| c.0.clone())
        .unwrap_or_default()
}

/// Label of the auto-type shortcut in use.
pub fn autotype_label(cx: &App) -> String {
    current(cx).autotype().label()
}

/// Label of the quick search shortcut in use.
pub fn quick_search_label(cx: &App) -> String {
    current(cx).quick_search().label()
}

#[cfg(test)]
mod tests {
    use super::{Hotkeys, Shortcut, ShortcutError};
    use gpui_kit::Keystroke;

    #[test]
    fn stored_form_round_trips() {
        for text in ["Ctrl+Alt+A", "Ctrl+Shift+Space", "Alt+Win+F12", "Ctrl+7"] {
            assert_eq!(Shortcut::parse(text).unwrap().to_storage(), text);
        }
        assert_eq!(
            Shortcut::parse("alt + ctrl + a").unwrap().to_storage(),
            "Ctrl+Alt+A",
            "ordem e caixa normalizadas"
        );
        assert!(Shortcut::parse("Shift+A").is_none(), "sem Ctrl/Alt/Win");
        assert!(Shortcut::parse("Ctrl+Alt").is_none(), "sem tecla");
        assert!(Shortcut::parse("Ctrl+A+B").is_none(), "duas teclas");
        assert!(
            Shortcut::parse("Ctrl+Left").is_none(),
            "tecla não suportada"
        );
    }

    #[test]
    fn virtual_keys_match_windows() {
        let vk = |t: &str| Shortcut::parse(t).unwrap().virtual_key();
        assert_eq!(vk("Ctrl+Alt+A"), 0x41);
        assert_eq!(vk("Ctrl+Alt+0"), 0x30);
        assert_eq!(vk("Ctrl+Space"), 0x20);
        assert_eq!(vk("Ctrl+F1"), 0x70);
        assert_eq!(vk("Ctrl+F24"), 0x87);
    }

    #[test]
    fn pressed_keys_become_shortcuts() {
        let press = |s: &'static str| Shortcut::from_keystroke(&Keystroke::parse(s).unwrap());
        assert_eq!(
            press("ctrl-alt-k").unwrap().unwrap().to_storage(),
            "Ctrl+Alt+K"
        );
        assert_eq!(
            press("ctrl-shift-space").unwrap().unwrap().to_storage(),
            "Ctrl+Shift+Space"
        );
        assert_eq!(press("shift-a"), Err(ShortcutError::NeedsModifier));
        assert_eq!(press("ctrl-left"), Err(ShortcutError::UnsupportedKey));
    }

    #[test]
    fn broken_settings_fall_back_to_the_defaults() {
        let h = Hotkeys {
            autotype: "lixo".into(),
            quick_search: "Ctrl+Alt+Q".into(),
        };
        assert_eq!(h.autotype().to_storage(), "Ctrl+Alt+A");
        assert_eq!(h.quick_search().to_storage(), "Ctrl+Alt+Q");
    }
}
