//! Server-side translations for the strings the core itself produces.
//!
//! Two callers use this:
//!
//! - **API error messages.** Handlers build `AppError`s with English text; the
//!   wire rendering (`crates/shiny-plugin-sdk/src/errors.rs`) runs the message
//!   through [`translate`] against the request's locale, so a toast reads in the
//!   user's language without touching the ~60 construction sites.
//! - **Plugin metadata.** `/api/plugins` serves the `plugin.toml` description,
//!   summary and category; when the plugin ships
//!   `web/i18n/<code>.json` (`plugin.<name>.meta.*`), the localized text wins.
//!
//! The catalogues are the same JSON files the web client fetches
//! (`web/i18n/<code>.json`, from the configured `WEB_DIR`), read once and cached.
//! The locale comes from `Accept-Language` — `apiFetch()` sends the interface
//! language on every call — and the message→key mapping is built from the
//! English catalogue, including `{placeholder}` templates.

use std::collections::HashMap;
use std::sync::OnceLock;

use axum::http::{header, HeaderMap};

static WEB_DIR: OnceLock<String> = OnceLock::new();
static PLUGINS_DIR: OnceLock<String> = OnceLock::new();
static CATALOGUES: OnceLock<HashMap<String, HashMap<String, String>>> = OnceLock::new();
static PATTERNS: OnceLock<Patterns> = OnceLock::new();

/// Remember where the catalogues live (the served UI dir and the installed
/// plugins dir). Called once at startup.
pub fn init(web_dir: &str, plugins_dir: &str) {
    let _ = WEB_DIR.set(web_dir.to_string());
    let _ = PLUGINS_DIR.set(plugins_dir.to_string());
}

fn catalogues() -> &'static HashMap<String, HashMap<String, String>> {
    CATALOGUES.get_or_init(|| {
        let dir = std::path::Path::new(WEB_DIR.get().map(String::as_str).unwrap_or("web")).join("i18n");
        let mut out = HashMap::new();
        let Ok(entries) = std::fs::read_dir(&dir) else {
            tracing::warn!("i18n: no catalogue directory at {}", dir.display());
            return out;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Some(code) = path.file_stem().and_then(|s| s.to_str()) else { continue };
            if code == "index" {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(&text)
            else {
                tracing::warn!("i18n: {} is not a flat JSON object", path.display());
                continue;
            };
            let table: HashMap<String, String> = map
                .into_iter()
                .filter_map(|(k, v)| match v {
                    serde_json::Value::String(s) => Some((k, s)),
                    _ => None,
                })
                .collect();
            out.insert(code.to_lowercase(), table);
        }
        out
    })
}

/// The catalogue for a language code: `pt-BR` → `pt-br`, then `pt`.
fn table(locale: &str) -> Option<&'static HashMap<String, String>> {
    let cat = catalogues();
    let code = locale.trim().to_lowercase();
    if let Some(t) = cat.get(&code) {
        return Some(t);
    }
    cat.get(code.split(['-', '_']).next().unwrap_or("en"))
}

/// Translate a catalogue key for a locale (`None` when there is no entry).
pub fn t(locale: &str, key: &str) -> Option<String> {
    table(locale).and_then(|t| t.get(key).cloned())
}

/// The locale of this request, from `Accept-Language`; `"en"` when nothing
/// matches a shipped catalogue.
pub fn locale_from(headers: &HeaderMap) -> String {
    let raw = headers
        .get(header::ACCEPT_LANGUAGE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    for tag in raw.split(',') {
        let code = tag.split(';').next().unwrap_or("").trim().to_lowercase();
        if code.is_empty() || code == "*" {
            continue;
        }
        if table(&code).is_some() {
            return code;
        }
    }
    "en".to_string()
}

/* ── English → key, for messages built as English ─────────────── */

struct Patterns {
    /// Exact English value -> key.
    exact: HashMap<String, String>,
    /// Single-`{placeholder}` templates: `"Moved {name} to workspace {n}"` is
    /// matched by prefix/suffix once `{n}` is the last placeholder.
    templated: Vec<Template>,
}

struct Template {
    prefix: String,
    suffix: String,
    /// The `{name}` of the (single) placeholder.
    placeholder: String,
    key: String,
}

impl Patterns {
    /// The catalogue entry matching an English message, if any.
    fn find<'a>(&'a self, english: &'a str) -> Option<Match<'a>> {
        if let Some(key) = self.exact.get(english) {
            return Some(Match::Exact(key));
        }
        for tpl in &self.templated {
            if english.len() > tpl.prefix.len() + tpl.suffix.len()
                && english.starts_with(&tpl.prefix)
                && english.ends_with(&tpl.suffix)
            {
                let value = &english[tpl.prefix.len()..english.len() - tpl.suffix.len()];
                return Some(Match::One {
                    key: &tpl.key,
                    placeholder: &tpl.placeholder,
                    value,
                });
            }
        }
        None
    }
}

enum Match<'a> {
    Exact(&'a str),
    One {
        key: &'a str,
        placeholder: &'a str,
        value: &'a str,
    },
}

fn patterns() -> &'static Patterns {
    PATTERNS.get_or_init(|| {
        let mut exact = HashMap::new();
        let mut templated = Vec::new();
        // The shell catalogue first, then every installed plugin's English
        // catalogue, so plugin tool errors ("folder not found") resolve too.
        let mut english: Vec<HashMap<String, String>> = Vec::new();
        if let Some(en) = catalogues().get("en") {
            english.push(en.clone());
        }
        if let Some(dir) = PLUGINS_DIR.get() {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let path = entry.path().join("web/i18n/en.json");
                    let Ok(text) = std::fs::read_to_string(&path) else { continue };
                    if let Ok(serde_json::Value::Object(map)) =
                        serde_json::from_str::<serde_json::Value>(&text)
                    {
                        english.push(
                            map.into_iter()
                                .filter_map(|(k, v)| match v {
                                    serde_json::Value::String(s) => Some((k, s)),
                                    _ => None,
                                })
                                .collect(),
                        );
                    }
                }
            }
        }
        for table in &english {
            for (key, value) in table {
                match (value.find('{'), value.matches('{').count()) {
                    (None, _) => {
                        exact.entry(value.clone()).or_insert_with(|| key.clone());
                    }
                    (Some(open), 1) => {
                        let Some(close) = value[open..].find('}').map(|i| i + open) else { continue };
                        templated.push(Template {
                            prefix: value[..open].to_string(),
                            suffix: value[close + 1..].to_string(),
                            placeholder: value[open + 1..close].to_string(),
                            key: key.clone(),
                        });
                    }
                    // Two or more placeholders cannot be matched from the
                    // rendered text; such messages stay English.
                    _ => {}
                }
            }
        }
        Patterns { exact, templated }
    })
}

/// Translate an English user-facing message into `locale`; unchanged when the
/// message has no catalogue entry (dynamic text, developer errors, `en` itself).
pub fn translate(locale: &str, english: &str) -> String {
    if locale.is_empty() || locale.eq_ignore_ascii_case("en") {
        return english.to_string();
    }
    let Some(found) = patterns().find(english) else {
        return english.to_string();
    };
    let key = match &found {
        Match::Exact(key) => *key,
        Match::One { key, .. } => *key,
    };
    let Some(template) = t(locale, key) else {
        return english.to_string();
    };
    match found {
        Match::Exact(_) => template,
        Match::One { placeholder, value, .. } => {
            let token = format!("{{{placeholder}}}");
            if template.contains(&token) {
                template.replace(&token, value)
            } else {
                fill_first_placeholder(&template, value).unwrap_or(template)
            }
        }
    }
}

/// Replace the first `{anything}` in `template` with `value`.
fn fill_first_placeholder(template: &str, value: &str) -> Option<String> {
    let open = template.find('{')?;
    let close = template[open..].find('}')? + open;
    Some(format!("{}{}{}", &template[..open], value, &template[close + 1..]))
}

/* ── Request locale (task-local) ──────────────────────────────── */

tokio::task_local! {
    static LOCALE: String;
}

/// Run a request future with its locale available to [`translate_current`].
pub async fn scope<F>(locale: String, fut: F) -> F::Output
where
    F: std::future::Future,
{
    LOCALE.scope(locale, fut).await
}

/// The current request's locale, or `"en"` outside a scoped request.
pub fn current_locale() -> String {
    LOCALE.try_with(|l| l.clone()).unwrap_or_else(|_| "en".to_string())
}

/// [`translate`] for the current request — the function the SDK's error
/// renderer is wired to.
pub fn translate_current(english: &str) -> String {
    translate(&current_locale(), english)
}

/// Plugin metadata in the request's language: `<plugin>/web/i18n/<code>.json`
/// keys `plugin.<name>.meta.<field>`, falling back to the `plugin.toml` text.
pub fn plugin_meta(
    plugin_name: &str,
    web_dir: &std::path::Path,
    field: &str,
    fallback: Option<String>,
) -> Option<String> {
    let locale = current_locale();
    if locale.eq_ignore_ascii_case("en") {
        return fallback;
    }
    let key = format!("plugin.{plugin_name}.meta.{field}");
    // `<code>` then its base language, like the client's own resolution.
    let base = locale.split(['-', '_']).next().unwrap_or("en").to_string();
    for code in [locale.as_str(), base.as_str()] {
        let path = web_dir.join("i18n").join(format!("{code}.json"));
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(&text)
        else {
            continue;
        };
        if let Some(serde_json::Value::String(value)) = map.get(&key) {
            return Some(value.clone());
        }
    }
    fallback
}
