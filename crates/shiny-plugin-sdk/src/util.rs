//! Small text/path helpers shared by plugin routes and tools.

use crate::db::Value;

/// Text from an SDK database value (`TEXT` or `INTEGER`).
pub fn as_text(v: &Value) -> String {
    match v {
        Value::Text(s) => s.clone(),
        Value::Int(n) => n.to_string(),
        _ => String::new(),
    }
}

/// [`as_text`] that maps NULL (or empty text) to `None`.
pub fn as_opt_text(v: &Value) -> Option<String> {
    if matches!(v, Value::Null) {
        return None;
    }
    let s = as_text(v);
    if s.is_empty() { None } else { Some(s) }
}

/// Trim a user-provided title and cap it at 120 characters.
pub fn clean_title(t: &str) -> String {
    let t = t.trim();
    if t.is_empty() {
        "Untitled".into()
    } else {
        t.chars().take(120).collect()
    }
}

/// A filesystem-safe filename stem from a title, plus `extension` (no dot).
pub fn filename_for_title(t: &str, extension: &str, fallback: &str) -> String {
    let clean: String = t
        .chars()
        .map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.') { c } else { '-' })
        .collect();
    let clean = clean.trim().trim_matches('.').to_string();
    let clean = if clean.is_empty() { fallback.to_string() } else { clean };
    format!("{clean}.{extension}")
}

/// A user id safe to use as a path segment.
pub fn sanitize_user(user_id: &str) -> String {
    let cleaned: String = user_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        .take(64)
        .collect();
    if cleaned.is_empty() {
        "default".into()
    } else {
        cleaned
    }
}
