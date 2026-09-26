//! Per-user Browser settings (the shield toggle), best-effort on SQLite.
//!
//! Settings live in `browser_settings` (migration `003`). Like `history.rs`,
//! every call swallows DB errors: a broken database must never stop the window
//! from opening or the ad blocker from running with its default (on).

use std::sync::Arc;

use shiny_plugin_sdk::db::Value;
use shiny_plugin_sdk::services::PluginCtx;

/// The Browser's per-user settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct Settings {
    pub adblock: bool,
    pub incognito: bool,
}

impl Default for Settings {
    fn default() -> Self {
        // Ad blocking is on by default; incognito is a mode the window enters.
        Self {
            adblock: true,
            incognito: false,
        }
    }
}

/// Ensure the table exists on an install whose migration has not run.
///
/// Idempotent; mirrors `history::ensure_schema`. Cheap (one no-op statement).
pub fn ensure_schema(ctx: &PluginCtx) {
    let _ = ctx.db().execute(
        "CREATE TABLE IF NOT EXISTS browser_settings (\
             traveler_id TEXT PRIMARY KEY, \
             adblock INTEGER NOT NULL DEFAULT 1, \
             incognito INTEGER NOT NULL DEFAULT 0, \
             updated_at TEXT NOT NULL DEFAULT (datetime('now')))",
        &[],
    );
}

/// Read a user's settings, falling back to the defaults on any error.
pub fn get(ctx: &Arc<PluginCtx>, user_id: &str) -> Settings {
    let rows = ctx
        .db()
        .query(
            "SELECT adblock, incognito FROM browser_settings WHERE traveler_id = ?1",
            &[Value::text(user_id.to_string())],
        )
        .unwrap_or_default();
    let Some(row) = rows.into_iter().next() else {
        return Settings::default();
    };
    let mut it = row.into_iter();
    Settings {
        adblock: int_bool(it.next()).unwrap_or(true),
        incognito: int_bool(it.next()).unwrap_or(false),
    }
}

/// Update one or both settings and return the resulting state.
pub fn set(
    ctx: &Arc<PluginCtx>,
    user_id: &str,
    adblock: Option<bool>,
    incognito: Option<bool>,
) -> Settings {
    let current = get(ctx, user_id);
    let next = Settings {
        adblock: adblock.unwrap_or(current.adblock),
        incognito: incognito.unwrap_or(current.incognito),
    };
    let _ = ctx.db().execute(
        "INSERT INTO browser_settings (traveler_id, adblock, incognito, updated_at) \
         VALUES (?1, ?2, ?3, datetime('now')) \
         ON CONFLICT(traveler_id) DO UPDATE SET \
             adblock = ?2, incognito = ?3, updated_at = datetime('now')",
        &[
            Value::text(user_id.to_string()),
            Value::Int(i64::from(next.adblock)),
            Value::Int(i64::from(next.incognito)),
        ],
    );
    next
}

fn int_bool(value: Option<Value>) -> Option<bool> {
    match value {
        Some(Value::Int(n)) => Some(n != 0),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shiny_plugin_sdk::db::Db;

    fn ctx(path: &str) -> Arc<PluginCtx> {
        let _ = std::fs::remove_file(path);
        crate::history::tests_support::ctx_for(path)
    }

    #[test]
    fn defaults_are_adblock_on() {
        let ctx = ctx("/tmp/browser-settings-default.db");
        ensure_schema(&ctx);
        assert_eq!(get(&ctx, "u1"), Settings { adblock: true, incognito: false });
    }

    #[test]
    fn set_round_trips_and_keeps_the_other_field() {
        let ctx = ctx("/tmp/browser-settings-roundtrip.db");
        ensure_schema(&ctx);
        let updated = set(&ctx, "u1", Some(false), None);
        assert_eq!(updated, Settings { adblock: false, incognito: false });
        let incog = set(&ctx, "u1", None, Some(true));
        assert_eq!(incog, Settings { adblock: false, incognito: true });
        // A different user is unaffected.
        assert_eq!(get(&ctx, "u2").adblock, true);
    }

    #[test]
    fn a_missing_table_is_not_fatal() {
        // No `ensure_schema`: reads fall back to defaults, writes are swallowed.
        let ctx = ctx("/tmp/browser-settings-missing.db");
        assert_eq!(get(&ctx, "u1").adblock, true);
        let _ = set(&ctx, "u1", Some(false), None);
        let _ = Db::open("sqlite:///tmp/browser-settings-missing.db").unwrap();
    }
}
