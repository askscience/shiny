//! The shell's own strings — the Qt file dialogs and their name filters, which
//! live in C++ and cannot read the web UI's catalogues from there.
//!
//! `build.rs` embeds every `web/i18n/<code>.json` (the same files the frontend
//! fetches), and this module resolves the session language and hands the keys
//! the native side needs to the shim as one JSON map — before `peakd_qt_run`,
//! when the app server (the other source of catalogues) may not be up yet.
//!
//! The web UI itself follows the per-user `ui.locale` preference; this is only
//! the native chrome (`--lang` / `$LC_ALL` / `$LC_MESSAGES` / `$LANG`).

include!(concat!(env!("OUT_DIR"), "/catalogs.rs"));

/// The keys the native side asks for, with their English fallbacks.
const SHELL_KEYS: &[(&str, &str)] = &[
    ("qt.dialog.open", "Open"),
    ("qt.dialog.chooseFolder", "Choose folder"),
    ("qt.dialog.save", "Save"),
    ("qt.filter.images", "Images (*.png *.jpg *.jpeg *.gif *.webp *.bmp *.svg *.avif)"),
    ("qt.filter.videos", "Videos (*.mp4 *.webm *.mkv *.mov *.avi)"),
    ("qt.filter.audio", "Audio (*.mp3 *.ogg *.wav *.flac *.m4a *.opus)"),
    ("qt.filter.pdf", "PDF (*.pdf)"),
    ("qt.filter.text", "Text (*.txt *.md *.csv *.json *.html)"),
    ("qt.filter.any", "%1 (*)"),
    ("qt.filter.allFiles", "All files (*)"),
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

/// `{ key: text }` for [`SHELL_KEYS`] in `lang`, as JSON for the C ABI.
pub fn shell_strings(lang: &str) -> String {
    let code = resolve(lang);
    let catalogue: serde_json::Map<String, serde_json::Value> = CATALOGUES
        .iter()
        .find(|(c, _)| *c == code)
        .map(|(_, json)| *json)
        .and_then(|json| serde_json::from_str(json).ok())
        .unwrap_or_default();
    let mut out = serde_json::Map::new();
    for (key, english) in SHELL_KEYS {
        let text = catalogue
            .get(*key)
            .and_then(|value| value.as_str())
            .unwrap_or(english);
        out.insert((*key).to_string(), serde_json::Value::String(text.to_string()));
    }
    serde_json::Value::Object(out).to_string()
}
