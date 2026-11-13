//! JSON com chaves normalizadas para camelCase.

use serde::de::DeserializeOwned;
use serde_json::Value;

/// Desserializa depois de normalizar as chaves do JSON para camelCase.
///
/// O servidor oficial antigo responde em PascalCase (`"Name"`), o atual e o
/// Vaultwarden em camelCase (`"name"`). Normalizar uma vez evita aliases em todo campo.
pub fn from_slice_normalized<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, serde_json::Error> {
    let value: Value = serde_json::from_slice(bytes)?;
    serde_json::from_value(normalize_keys(value))
}

fn normalize_keys(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(k, v)| (camel_case(&k), normalize_keys(v)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(normalize_keys).collect()),
        other => other,
    }
}

pub(crate) fn camel_case(key: &str) -> String {
    // Siglas inteiras ("SSN") viram minúsculas; "PrivateKey" vira "privateKey";
    // chaves já em camelCase ou snake_case ("access_token") ficam iguais.
    if !key.chars().any(char::is_lowercase) {
        return key.to_lowercase();
    }
    let mut chars = key.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}
