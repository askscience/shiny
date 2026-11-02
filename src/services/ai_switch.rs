//! The "No slop" switch: whether this machine runs any AI at all.
//!
//! Two layers, both reversible from Settings → Assistant:
//! - **Per user**, the `ai.enabled` preference (`"false"` = off; missing = on).
//!   A new account inherits the system default (`PEAKD_AI_DEFAULT=off`, written
//!   by the installer when the "No slop" tier is chosen), so the first login
//!   already has the AI off. Chat and agent requests are refused while it is off.
//! - **Per machine**, a small state file (`$PEAKD_DATA_DIR/ai-state`, "on"/"off")
//!   that the session script reads *before* the server starts, to decide
//!   whether to launch Ollama. The server keeps it in step with the preference
//!   and, when the AI is switched back on, starts Ollama and lets the voice
//!   watchdog bring the speech sidecars up — no restart needed.
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use sqlx::SqlitePool;

pub const PREF_KEY: &str = "ai.enabled";
const DEFAULT_ENV: &str = "PEAKD_AI_DEFAULT";
const OLLAMA_START: &str = "/usr/local/bin/peakd-ollama-start";

static MACHINE_ON: AtomicBool = AtomicBool::new(true);

/// "off" in the environment means a fresh install chose "No slop".
fn system_default_on() -> bool {
    parse_on(&std::env::var(DEFAULT_ENV).unwrap_or_default()).unwrap_or(true)
}

/// "off"/"false"/"0" → Some(false), "on"/"true"/"1" → Some(true), else None.
fn parse_on(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "off" | "false" | "0" | "no" => Some(false),
        "on" | "true" | "1" | "yes" => Some(true),
        _ => None,
    }
}

fn state_file() -> Option<PathBuf> {
    std::env::var_os("PEAKD_DATA_DIR").map(|dir| PathBuf::from(dir).join("ai-state"))
}

/// Read the machine state once at startup: the state file wins, then the
/// installer's default. Call before the voice sidecars are spawned.
pub fn init() {
    let from_file = state_file()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| parse_on(&text));
    let on = from_file.unwrap_or_else(system_default_on);
    MACHINE_ON.store(on, Ordering::Relaxed);
    if !on {
        tracing::info!("AI is switched off (No slop): no voice sidecars, chat and agent refused");
    }
}

/// Whether the machine-wide AI services (Ollama, speech sidecars) may run.
pub fn machine_on() -> bool {
    MACHINE_ON.load(Ordering::Relaxed)
}

/// Whether the client may fetch speech models on its own at login. Off for
/// the "Text only" and "No slop" tiers (`PEAKD_AI_TIER` from the session).
pub fn voice_auto_prepare() -> bool {
    !matches!(
        std::env::var("PEAKD_AI_TIER").unwrap_or_default().as_str(),
        "none" | "off"
    )
}

/// Whether this user has the AI on. A missing preference means on.
pub async fn user_enabled(pool: &SqlitePool, user_id: &str) -> bool {
    let value: Option<String> =
        sqlx::query_scalar("SELECT value FROM user_preferences WHERE user_id = ? AND key = ?")
            .bind(user_id)
            .bind(PREF_KEY)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    value.as_deref().and_then(parse_on).unwrap_or(true)
}

/// Refuse an AI request when the user has the AI off.
pub async fn require_enabled(
    pool: &SqlitePool,
    user_id: &str,
) -> Result<(), crate::errors::AppError> {
    if user_enabled(pool, user_id).await {
        Ok(())
    } else {
        Err(crate::errors::AppError::BadRequest(
            "AI is switched off. Turn it on in Settings → Assistant.".into(),
        ))
    }
}

/// A new account starts from the system default: with "No slop" installed,
/// write `ai.enabled=false` so the very first login has no AI.
pub async fn seed_new_user(pool: &SqlitePool, user_id: &str) {
    if system_default_on() {
        return;
    }
    let result = sqlx::query(
        "INSERT OR IGNORE INTO user_preferences (user_id, key, value, updated_at)
         VALUES (?, ?, 'false', datetime('now'))",
    )
    .bind(user_id)
    .bind(PREF_KEY)
    .execute(pool)
    .await;
    if let Err(e) = result {
        tracing::warn!("could not seed {PREF_KEY} for a new account: {e}");
    }
    set_machine(false);
}

/// Follow a change of the preference (from `PUT /api/preferences`).
pub fn on_preference_change(value: &serde_json::Value) {
    let raw = match value {
        serde_json::Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    // An empty value is the client removing the key: back to the default, on.
    set_machine(parse_on(&raw).unwrap_or(true));
}

fn set_machine(on: bool) {
    let was_on = MACHINE_ON.swap(on, Ordering::Relaxed);
    if let Some(path) = state_file() {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(&path, if on { "on\n" } else { "off\n" }) {
            tracing::warn!("could not write {}: {e}", path.display());
        }
    }
    if on && !was_on {
        tracing::info!("AI switched back on: starting Ollama; the voice watchdog starts speech");
        start_ollama();
    }
}

/// The session script skips Ollama when the AI is off; turning it on starts
/// it with the same helper the session uses. Missing helper (dev setups) is fine.
fn start_ollama() {
    if !std::path::Path::new(OLLAMA_START).exists() {
        return;
    }
    if let Err(e) = std::process::Command::new(OLLAMA_START)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        tracing::warn!("could not start Ollama: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::parse_on;

    #[test]
    fn parses_on_and_off_spellings() {
        assert_eq!(parse_on("off"), Some(false));
        assert_eq!(parse_on(" FALSE\n"), Some(false));
        assert_eq!(parse_on("on"), Some(true));
        assert_eq!(parse_on("true"), Some(true));
        assert_eq!(parse_on("maybe"), None);
        assert_eq!(parse_on(""), None);
    }
}
