//! The window's own strings, from the app's catalogs.
//!
//! `web/i18n/<code>.json` is the same flat map the UI fetches
//! (`web/js/i18n.js`): dotted keys, `{name}` placeholders. The window already
//! holds a client for the app origin, so it reads the catalog over HTTP at
//! startup instead of embedding a copy — the files are served unauthenticated.
//!
//! A missing catalog is *not* a 404: the static fallback answers `index.html`
//! with HTTP 200, so a response only counts once it parses into a JSON object,
//! never by its status. Every failure keeps the English fallbacks: the window
//! must always come up.

use std::collections::HashMap;

/// The session language: `LC_ALL` → `LC_MESSAGES` → `LANG`, else `en`.
///
/// `en_US.UTF-8@euro` becomes `en-us`, the code the catalogs are named after.
/// Empty values and the `C`/`POSIX` locales mean "no preference".
pub fn language() -> String {
    for key in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        let Ok(value) = std::env::var(key) else { continue };
        let value = value.trim();
        if value.is_empty() || value == "C" || value == "POSIX" {
            continue;
        }
        let code = value.split('.').next().unwrap_or(value);
        let code = code.split('@').next().unwrap_or(code);
        return code.replace('_', "-").to_lowercase();
    }
    "en".to_string()
}

/// `{base}/i18n/<code>.json` plus the base language (`it-IT` → `it`) as
/// filler: the requested code wins, the base language covers the keys a
/// partial translation leaves out. Empty when nothing fetches or parses.
pub fn load(
    client: &reqwest::blocking::Client,
    base: &str,
    code: &str,
) -> HashMap<String, String> {
    let mut wanted = vec![code];
    let base_language = code.split('-').next().unwrap_or(code);
    if base_language != code {
        wanted.push(base_language);
    }
    let mut strings = HashMap::new();
    for want in wanted {
        for (key, text) in fetch(client, base, want) {
            strings.entry(key).or_insert(text);
        }
    }
    strings
}

/// One catalog as `(key, text)` pairs — nothing when the response is not a
/// JSON object (a missing file serves `index.html`).
fn fetch(client: &reqwest::blocking::Client, base: &str, code: &str) -> Vec<(String, String)> {
    match client
        .get(format!("{base}/i18n/{code}.json"))
        .send()
        .and_then(|response| response.json::<serde_json::Value>())
    {
        Ok(serde_json::Value::Object(catalog)) => catalog
            .into_iter()
            .filter_map(|(key, value)| value.as_str().map(|text| (key, text.to_string())))
            .collect(),
        _ => Vec::new(),
    }
}

/// `key` from the catalog, else the built-in English `fallback`.
pub fn tr(strings: &HashMap<String, String>, key: &str, fallback: &str) -> String {
    strings
        .get(key)
        .cloned()
        .unwrap_or_else(|| fallback.to_string())
}

/// Fill the `{name}` placeholders the catalogs use; an unknown one stays
/// visible on purpose, as in the UI.
pub fn fill(text: &str, params: &[(&str, &str)]) -> String {
    let mut out = text.to_string();
    for (name, value) in params {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}
