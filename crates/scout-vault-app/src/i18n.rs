//! Interface languages.
//!
//! Texts live in `locales/<code>.json` (a flat `key → text` object),
//! embedded in the executable. To add a language: create the file and add
//! a variant to [`Language`] (code, name and the `include_str!`). The tests
//! check that every language has exactly the same keys and the same
//! `{name}` placeholders.
//!
//! Usage: `t!("key")` returns `&'static str`; `t!("key", n = 3)` replaces `{n}`
//! in the text and returns `String`. The language is global ([`set`]); switching and
//! redrawing the windows (`cx.refresh_windows()`) is enough for the interface to change instantly.

use std::collections::HashMap;
use std::fmt::Display;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

/// Available languages. The first one is the reference (and the default in tests).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    PtBr,
    En,
}

impl Language {
    pub const ALL: [Language; 2] = [Self::PtBr, Self::En];

    /// BCP 47 code, used in the catalog file and in the preferences.
    pub fn code(self) -> &'static str {
        match self {
            Self::PtBr => "pt-BR",
            Self::En => "en",
        }
    }

    /// The language's name in itself (not translated).
    pub fn native_name(self) -> &'static str {
        match self {
            Self::PtBr => "Português",
            Self::En => "English",
        }
    }

    /// The embedded catalog.
    pub fn source(self) -> &'static str {
        match self {
            Self::PtBr => include_str!("../locales/pt-BR.json"),
            Self::En => include_str!("../locales/en.json"),
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|l| l.code().eq_ignore_ascii_case(code.trim()))
    }

    /// Language for a system tag (`pt-BR`, `pt_PT`, `en-US`, `de`…):
    /// Portuguese → pt-BR; anything else → English.
    pub fn for_system_tag(tag: &str) -> Self {
        let primary = tag
            .split(['-', '_', '.'])
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        match primary.as_str() {
            "pt" => Self::PtBr,
            _ => Self::En,
        }
    }

    /// The operating system's interface language.
    pub fn system() -> Self {
        if let Some(lang) = std::env::var("SCOUT_VAULT_LANG")
            .ok()
            .and_then(|v| Self::from_code(&v))
        {
            return lang;
        }
        Self::for_system_tag(&system_tag())
    }

    /// Saved preference (`""` = follow the system).
    pub fn from_setting(setting: &str) -> Self {
        Self::from_code(setting).unwrap_or_else(Self::system)
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|l| *l == self).unwrap_or(0)
    }
}

#[cfg(windows)]
fn system_tag() -> String {
    // User interface LANGID; the primary language is in the low 10 bits.
    // SAFETY: function with no arguments and no side effects.
    let langid = unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() };
    const LANG_PORTUGUESE: u16 = 0x16;
    if langid & 0x3ff == LANG_PORTUGUESE {
        "pt-BR".into()
    } else {
        "en".into()
    }
}

#[cfg(not(windows))]
fn system_tag() -> String {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|v| std::env::var(v).ok())
        .find(|v| !v.is_empty())
        .unwrap_or_default()
}

static CURRENT: AtomicUsize = AtomicUsize::new(0);

/// Current interface language.
pub fn current() -> Language {
    Language::ALL[CURRENT.load(Ordering::Relaxed).min(Language::ALL.len() - 1)]
}

/// Switches the language (including gpui-kit's components). The caller redraws
/// the windows.
pub fn set(lang: Language) {
    CURRENT.store(lang.index(), Ordering::Relaxed);
    gpui_kit::component::set_locale(lang.code());
}

/// Keys and texts of a language.
pub fn catalog(lang: Language) -> &'static HashMap<String, String> {
    static CATALOGS: OnceLock<Vec<HashMap<String, String>>> = OnceLock::new();
    let all = CATALOGS.get_or_init(|| {
        Language::ALL
            .iter()
            .map(|l| {
                serde_json::from_str(l.source())
                    .unwrap_or_else(|e| panic!("catálogo {} inválido: {e}", l.code()))
            })
            .collect()
    });
    &all[lang.index()]
}

/// Text of the key in the current language (falls back to the reference language and, as a last
/// resort, to the key itself).
pub fn t(key: &'static str) -> &'static str {
    lookup(current(), key)
}

/// Text of the key in a specific language.
pub fn lookup(lang: Language, key: &'static str) -> &'static str {
    catalog(lang)
        .get(key)
        .or_else(|| catalog(Language::ALL[0]).get(key))
        .map(String::as_str)
        .unwrap_or(key)
}

/// Text with `{name}` placeholders replaced by the values.
pub fn tf(key: &'static str, args: &[(&str, &dyn Display)]) -> String {
    let mut out = t(key).to_string();
    for (name, value) in args {
        out = out.replace(&format!("{{{name}}}"), &value.to_string());
    }
    out
}

/// `t!("key")` → `&'static str`; `t!("key", name = value, …)` → `String`.
#[macro_export]
macro_rules! t {
    ($key:literal) => {
        $crate::i18n::t($key)
    };
    ($key:literal, $($name:ident = $value:expr),+ $(,)?) => {
        $crate::i18n::tf($key, &[$((stringify!($name), &$value as &dyn ::std::fmt::Display)),+])
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn placeholders(text: &str) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        let mut rest = text;
        while let Some(start) = rest.find('{') {
            let after = &rest[start + 1..];
            let Some(end) = after.find('}') else { break };
            out.insert(after[..end].to_string());
            rest = &after[end + 1..];
        }
        out
    }

    #[test]
    fn every_language_has_every_key_and_placeholder() {
        let reference = catalog(Language::ALL[0]);
        assert!(!reference.is_empty());
        for lang in Language::ALL {
            let cat = catalog(lang);
            let missing: Vec<_> = reference.keys().filter(|k| !cat.contains_key(*k)).collect();
            let extra: Vec<_> = cat.keys().filter(|k| !reference.contains_key(*k)).collect();
            assert!(missing.is_empty(), "{}: faltam {missing:?}", lang.code());
            assert!(extra.is_empty(), "{}: sobram {extra:?}", lang.code());
            for (key, text) in cat {
                assert!(!text.trim().is_empty(), "{}: {key} vazio", lang.code());
                assert_eq!(
                    placeholders(text),
                    placeholders(&reference[key]),
                    "{}: marcadores diferentes em {key}",
                    lang.code()
                );
            }
        }
    }

    /// Every key used in `t!("…")` in the code exists in the catalog.
    #[test]
    fn every_key_used_in_code_exists() {
        let reference = catalog(Language::ALL[0]);
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut missing = Vec::new();
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            // This file only talks about the macro (documentation and tests).
            if path.extension().is_none_or(|e| e != "rs") || path.ends_with("i18n.rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            for (at, _) in text.match_indices("t!(") {
                // `print!("…")`, `assert!("…")` etc. don't count.
                let before = text[..at].chars().next_back().unwrap_or(' ');
                if before.is_alphanumeric() || before == '_' {
                    continue;
                }
                let Some(rest) = text[at + 3..].trim_start().strip_prefix('"') else {
                    continue;
                };
                let key = rest.split('"').next().unwrap_or("");
                if key == "key" || key == "…" {
                    continue;
                }
                if !reference.contains_key(key) {
                    missing.push(format!("{}: {key}", path.display()));
                }
            }
        }
        assert!(missing.is_empty(), "keys without translation: {missing:?}");
    }

    #[test]
    fn system_tags_map_to_languages() {
        assert_eq!(Language::for_system_tag("pt-BR"), Language::PtBr);
        assert_eq!(Language::for_system_tag("pt_PT.UTF-8"), Language::PtBr);
        assert_eq!(Language::for_system_tag("en-US"), Language::En);
        assert_eq!(Language::for_system_tag("de-DE"), Language::En);
        assert_eq!(Language::for_system_tag(""), Language::En);
        assert_eq!(Language::from_code("EN"), Some(Language::En));
        assert_eq!(Language::from_code(""), None);
    }

    #[test]
    fn lookup_and_placeholders() {
        assert_eq!(lookup(Language::PtBr, "settings.language"), "Idioma");
        assert_eq!(lookup(Language::En, "settings.language"), "Language");
        assert_eq!(lookup(Language::En, "nao.existe"), "nao.existe");
    }
}
