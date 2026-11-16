//! Browser extension texts (`extension/_locales/*/messages.json`):
//! every language has the same keys and placeholders, and every key used in code
//! (`tr("…")`, `getMessage("…")`, `data-i18n="…"`, `__MSG_…__`) exists.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

fn extension_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../extension")
}

fn load(locale: &str) -> BTreeMap<String, serde_json::Value> {
    let path = extension_dir()
        .join("_locales")
        .join(locale)
        .join("messages.json");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{path:?}: {e}"))
}

fn locales() -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(extension_dir().join("_locales"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    out.sort();
    out
}

fn placeholders(entry: &serde_json::Value) -> BTreeSet<String> {
    entry
        .get("placeholders")
        .and_then(|p| p.as_object())
        .map(|p| p.keys().cloned().collect())
        .unwrap_or_default()
}

#[test]
fn every_locale_has_every_message() {
    let manifest: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(extension_dir().join("manifest.json")).unwrap(),
    )
    .unwrap();
    let default = manifest["default_locale"].as_str().expect("default_locale");
    let reference = load(default);
    let all = locales();
    assert!(all.len() >= 2, "{all:?}");
    for locale in &all {
        let messages = load(locale);
        let keys: BTreeSet<_> = messages.keys().collect();
        let expected: BTreeSet<_> = reference.keys().collect();
        assert_eq!(keys, expected, "{locale}: chaves diferentes de {default}");
        for (key, entry) in &messages {
            let text = entry["message"].as_str().unwrap_or("");
            assert!(!text.trim().is_empty(), "{locale}: {key} vazio");
            assert!(!text.contains("  "), "{locale}: {key} com espaço duplo");
            assert_eq!(
                placeholders(entry),
                placeholders(&reference[key]),
                "{locale}: marcadores diferentes em {key}"
            );
        }
    }
}

/// Keys mentioned in code: after `prefix` up to `end`.
fn keys_after(text: &str, prefix: &str, end: char) -> Vec<String> {
    text.match_indices(prefix)
        .filter_map(|(at, _)| {
            let rest = &text[at + prefix.len()..];
            let key: String = rest.chars().take_while(|c| *c != end).collect();
            (!key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
                .then_some(key)
        })
        .collect()
}

#[test]
fn every_message_used_exists() {
    let reference = load("pt_BR");
    let mut missing = Vec::new();
    for name in [
        "manifest.json",
        "popup.html",
        "popup.js",
        "background.js",
        "capture.js",
        "inline.js",
    ] {
        let text = std::fs::read_to_string(extension_dir().join(name)).unwrap();
        let mut used = keys_after(&text, "tr(\"", '"');
        used.extend(keys_after(&text, "getMessage(\"", '"'));
        used.extend(keys_after(&text, "data-i18n=\"", '"'));
        used.extend(keys_after(&text, "__MSG_", '_'));
        for key in used {
            if !reference.contains_key(&key) {
                missing.push(format!("{name}: {key}"));
            }
        }
    }
    assert!(missing.is_empty(), "mensagens sem tradução: {missing:?}");
}
