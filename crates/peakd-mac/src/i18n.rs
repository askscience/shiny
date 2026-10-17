//! The shell's own strings — the Touch Bar labels, which are built before the
//! app server (the web UI's own source of catalogues) is reachable.
//!
//! `build.rs` embeds every `web/i18n/<code>.json` (the same files the frontend
//! fetches), and this module resolves the session language and hands the keys
//! the native bar needs to `touchbar.rs`.
//!
//! The web UI itself follows the per-user `ui.locale` preference; this is only
//! the native chrome (`--lang` / `PEAKD_LANG` / `$LC_ALL` / `$LC_MESSAGES` /
//! `$LANG`). Action *ids* are not catalogued: they are the wire protocol with
//! the page, so only the visible labels are translated.

use std::collections::HashMap;

include!(concat!(env!("OUT_DIR"), "/catalogs.rs"));

/// The keys the native bar asks for, with their English fallbacks.
const SHELL_KEYS: &[(&str, &str)] = &[
    ("touchbar.talk.label", "Ask"),
    ("touchbar.stop.label", "Stop"),
    ("touchbar.mute.label", "Mute"),
    ("touchbar.volume-down.label", "Vol-"),
    ("touchbar.volume-up.label", "Vol+"),
    ("touchbar.screen-down.label", "Scr-"),
    ("touchbar.screen-up.label", "Scr+"),
    ("touchbar.workspace-prev.label", "Prev"),
    ("touchbar.workspace-next.label", "Next"),
    ("touchbar.kbd-backlight-down.label", "Kbd-"),
    ("touchbar.kbd-backlight-up.label", "Kbd+"),
    ("touchbar.keyboard.label", "Keys"),
    ("touchbar.settings.label", "Setup"),
];

/// The shipped catalogue for a requested language: exact code, then base
/// language, else English.
pub fn resolve(requested: &str) -> &'static str {
    let want = requested.trim().to_lowercase();
    if want.is_empty() {
        return "en";
    }
    if let Some((code, _)) = CATALOGUES.iter().find(|(code, _)| *code == want) {
        return code;
    }
    let base = want.split(['-', '_']).next().unwrap_or("en");
    CATALOGUES
        .iter()
        .find(|(code, _)| *code == base)
        .map(|(code, _)| *code)
        .unwrap_or("en")
}

/// The English fallback for a shell key (the key itself when it is unknown).
///
/// Every label must survive a partial catalogue: a translation may carry the
/// key, but English is always there.
pub(crate) fn english(key: &str) -> &str {
    let fallback: Option<&str> = SHELL_KEYS
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, text)| *text);
    fallback.unwrap_or(key)
}

/// The shell's own strings, resolved for one language.
pub struct Strings {
    lang: &'static str,
    table: HashMap<&'static str, String>,
}

impl Strings {
    /// The catalogue code these strings came from (see [`resolve`]).
    pub fn lang(&self) -> &'static str {
        self.lang
    }

    /// The text for `key`, falling back to the English in [`SHELL_KEYS`].
    pub fn get<'a>(&'a self, key: &'a str) -> &'a str {
        self.table
            .get(key)
            .map(|text| text.as_str())
            .unwrap_or_else(|| english(key))
    }
}

/// The [`SHELL_KEYS`] a catalogue carries, as `key -> text` — every key, so a
/// partial translation simply keeps the English it does not override.
fn table_from(json: &str) -> HashMap<&'static str, String> {
    let catalogue: HashMap<String, serde_json::Value> =
        serde_json::from_str(json).unwrap_or_default();
    let mut table = HashMap::with_capacity(SHELL_KEYS.len());
    for (key, english) in SHELL_KEYS {
        let text = catalogue
            .get(*key)
            .and_then(|value| value.as_str())
            .unwrap_or(english);
        table.insert(*key, text.to_string());
    }
    table
}

/// `{ key: text }` for [`SHELL_KEYS`] in the language `requested` resolves to.
pub fn strings(requested: &str) -> Strings {
    let code = resolve(requested);
    let json = CATALOGUES
        .iter()
        .find(|(c, _)| *c == code)
        .map(|(_, json)| *json)
        .unwrap_or("");
    Strings {
        lang: code,
        table: table_from(json),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolution_prefers_exact_then_base_then_english() {
        assert_eq!(resolve("en"), "en");
        assert_eq!(resolve("en-US"), "en");
        assert_eq!(resolve("  EN_GB "), "en");
        assert_eq!(resolve(""), "en");
        // A language with no catalogue still resolves, to the fallback.
        assert_eq!(resolve("zz"), "en");
    }

    #[test]
    fn every_shell_key_has_english_text() {
        assert_eq!(SHELL_KEYS.len(), 13);
        for (key, text) in SHELL_KEYS {
            assert!(!text.is_empty(), "{key} has no English text");
            assert_eq!(english(key), *text);
        }
        assert_eq!(english("touchbar.not.a.key"), "touchbar.not.a.key");
    }

    #[test]
    fn a_catalogue_overrides_the_english_fallback() {
        let table = table_from(r#"{"touchbar.talk.label": "Sprechen", "unrelated.key": "x"}"#);
        assert_eq!(
            table.get("touchbar.talk.label").map(String::as_str),
            Some("Sprechen")
        );
        assert_eq!(
            table.get("touchbar.stop.label").map(String::as_str),
            Some("Stop")
        );
        // Only the keys the shell asks for are taken from the catalogue.
        assert!(!table.contains_key("unrelated.key"));
        // A catalogue that is not an object (or does not parse) is English.
        assert_eq!(
            table_from("[]")
                .get("touchbar.talk.label")
                .map(String::as_str),
            Some("Ask")
        );
    }

    #[test]
    fn strings_resolve_and_fall_back() {
        let en = strings("en");
        assert_eq!(en.lang(), "en");
        for (key, text) in SHELL_KEYS {
            // Whatever the catalogue holds, the bar always gets a real label.
            let label = en.get(key);
            assert!(
                !label.is_empty() && label != *key,
                "{key} produced {label:?}"
            );
            // An unshipped language is English, never blank.
            assert_eq!(strings("zz").get(key), *text);
        }
    }
}
