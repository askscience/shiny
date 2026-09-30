//! Per-user Browser settings (the shield toggle, incognito, the search
//! engine), best-effort on SQLite.
//!
//! Settings live in `browser_settings` (migration `003`, extended by `005`).
//! Like `history.rs`, every call swallows DB errors: a broken database must
//! never stop the window from opening or the ad blocker from running with its
//! default (on).

use std::sync::Arc;

use shiny_plugin_sdk::db::Value;
use shiny_plugin_sdk::services::PluginCtx;

/// The search engines the window offers.
///
/// The names are the values stored in `browser_settings.search_engine`; the
/// URLs they map to live in [`search_url`].
pub const SEARCH_ENGINES: [&str; 4] = ["duckduckgo", "brave", "google", "bing"];

/// The engine a fresh install searches with.
pub const DEFAULT_SEARCH_ENGINE: &str = "duckduckgo";

/// True when `value` names one of [`SEARCH_ENGINES`].
pub fn is_search_engine(value: &str) -> bool {
    SEARCH_ENGINES.contains(&value)
}

/// The URL `query` becomes on `engine`, percent-encoded into `{encoded}`.
///
/// An unknown engine name falls back to the default rather than building a URL
/// with an empty template. `SEARXNG_URL` is deliberately not consulted here:
/// [`crate::routes::Target::into_url`] applies that override first, because a
/// self-hosted instance still wins over any named engine.
pub fn search_url(engine: &str, query: &str) -> String {
    let encoded = url::form_urlencoded::byte_serialize(query.as_bytes()).collect::<String>();
    let template = match engine {
        "brave" => "https://search.brave.com/search?q={encoded}",
        "google" => "https://www.google.com/search?q={encoded}",
        "bing" => "https://www.bing.com/search?q={encoded}",
        // DuckDuckGo is the default, and the fallback for an unknown name.
        _ => "https://duckduckgo.com/?q={encoded}",
    };
    template.replace("{encoded}", &encoded)
}

/// The Browser's per-user settings.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Settings {
    pub adblock: bool,
    pub incognito: bool,
    /// A name from [`SEARCH_ENGINES`]; `SEARXNG_URL` still overrides it when
    /// the window builds a search URL.
    pub search_engine: String,
}

impl Default for Settings {
    fn default() -> Self {
        // Ad blocking is on by default; incognito is a mode the window enters.
        Self {
            adblock: true,
            incognito: false,
            search_engine: DEFAULT_SEARCH_ENGINE.to_string(),
        }
    }
}

/// Ensure the table exists on an install whose migration has not run.
///
/// Idempotent; mirrors `history::ensure_schema`. Cheap (one no-op statement).
pub fn ensure_schema(ctx: &PluginCtx) {
    let db = ctx.db();
    let _ = db.execute(
        "CREATE TABLE IF NOT EXISTS browser_settings (\
             traveler_id TEXT PRIMARY KEY, \
             adblock INTEGER NOT NULL DEFAULT 1, \
             incognito INTEGER NOT NULL DEFAULT 0, \
             search_engine TEXT NOT NULL DEFAULT 'duckduckgo', \
             updated_at TEXT NOT NULL DEFAULT (datetime('now')))",
        &[],
    );

    // Repair a table created before `search_engine` existed. Migration 005 adds
    // the column, but migration files are recorded by filename and never
    // re-run, so an install that already has `browser_settings` keeps the old
    // shape unless this upgrades it. SQLite has no conditional DDL: a duplicate
    // column is success, and anything else is logged and ignored so a broken
    // database still cannot stop the browser.
    let has_column = db
        .query(
            "SELECT COUNT(*) FROM pragma_table_info('browser_settings') WHERE name = 'search_engine'",
            &[],
        )
        .ok()
        .and_then(|rows| rows.into_iter().next())
        .and_then(|row| row.into_iter().next())
        .map(|value| matches!(value, Value::Int(n) if n > 0))
        .unwrap_or(true); // unreadable schema: leave it alone rather than guess

    if has_column {
        return;
    }

    match db.execute(ADD_SEARCH_ENGINE_COLUMN, &[]) {
        Ok(_) => tracing::info!("browser: added the settings `search_engine` column"),
        // Raced by another caller, or already present after all.
        Err(err) if err.to_string().contains("duplicate column") => {}
        Err(err) => {
            tracing::warn!("browser: could not add the settings `search_engine` column: {err}")
        }
    }
}

/// Column the search-engine setting needs, and the schema repair that adds it.
///
/// See [`ensure_schema`] for why this is code and not only a migration: the
/// migration only runs once, and only on installs that had not applied it yet.
const ADD_SEARCH_ENGINE_COLUMN: &str =
    "ALTER TABLE browser_settings ADD COLUMN search_engine TEXT NOT NULL DEFAULT 'duckduckgo'";

/// Read a user's settings, falling back to the defaults on any error.
///
/// An unreadable or simply absent `search_engine` column takes the whole read
/// down, so this returns [`Settings::default`] — which is the same state an
/// unset row reports. `ensure_schema` upgrades the column on load.
pub fn get(ctx: &Arc<PluginCtx>, user_id: &str) -> Settings {
    let rows = ctx
        .db()
        .query(
            "SELECT adblock, incognito, search_engine FROM browser_settings WHERE traveler_id = ?1",
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
        // A value outside the allowed set (an older/newer build, a hand-edited
        // row) is treated as unset rather than passed through to a broken URL.
        search_engine: value_text(it.next())
            .filter(|engine| is_search_engine(engine))
            .unwrap_or_else(|| DEFAULT_SEARCH_ENGINE.to_string()),
    }
}

/// Update one or more settings and return the resulting state.
///
/// `search_engine` is validated against [`SEARCH_ENGINES`]: an unknown name
/// keeps the current value instead of being stored, and `None` leaves it as is.
pub fn set(
    ctx: &Arc<PluginCtx>,
    user_id: &str,
    adblock: Option<bool>,
    incognito: Option<bool>,
    search_engine: Option<&str>,
) -> Settings {
    let current = get(ctx, user_id);
    let search_engine = match search_engine {
        Some(engine) if is_search_engine(engine) => engine.to_string(),
        // Ignore an unknown engine rather than persist one the window cannot
        // turn into a URL.
        _ => current.search_engine.clone(),
    };
    let next = Settings {
        adblock: adblock.unwrap_or(current.adblock),
        incognito: incognito.unwrap_or(current.incognito),
        search_engine,
    };
    let _ = ctx.db().execute(
        "INSERT INTO browser_settings (traveler_id, adblock, incognito, search_engine, updated_at) \
         VALUES (?1, ?2, ?3, ?4, datetime('now')) \
         ON CONFLICT(traveler_id) DO UPDATE SET \
             adblock = ?2, incognito = ?3, search_engine = ?4, updated_at = datetime('now')",
        &[
            Value::text(user_id.to_string()),
            Value::Int(i64::from(next.adblock)),
            Value::Int(i64::from(next.incognito)),
            Value::text(next.search_engine.clone()),
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

fn value_text(value: Option<Value>) -> Option<String> {
    match value {
        Some(Value::Text(text)) => Some(text),
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
        assert_eq!(
            get(&ctx, "u1"),
            Settings {
                adblock: true,
                incognito: false,
                search_engine: "duckduckgo".into(),
            }
        );
    }

    #[test]
    fn set_round_trips_and_keeps_the_other_field() {
        let ctx = ctx("/tmp/browser-settings-roundtrip.db");
        ensure_schema(&ctx);
        let updated = set(&ctx, "u1", Some(false), None, None);
        assert_eq!(
            updated,
            Settings {
                adblock: false,
                incognito: false,
                search_engine: "duckduckgo".into(),
            }
        );
        let incog = set(&ctx, "u1", None, Some(true), None);
        assert_eq!(incog.adblock, false);
        assert_eq!(incog.incognito, true);
        assert_eq!(incog.search_engine, "duckduckgo");
        // A different user is unaffected.
        assert_eq!(get(&ctx, "u2").adblock, true);
    }

    #[test]
    fn search_engine_round_trips_and_ignores_unknown_names() {
        let ctx = ctx("/tmp/browser-settings-engine.db");
        ensure_schema(&ctx);
        assert_eq!(get(&ctx, "u1").search_engine, DEFAULT_SEARCH_ENGINE);

        // Every allowed name is stored and read back.
        for engine in SEARCH_ENGINES {
            let next = set(&ctx, "u1", None, None, Some(engine));
            assert_eq!(next.search_engine, engine);
            assert_eq!(get(&ctx, "u1").search_engine, engine);
        }

        // An unknown name is ignored: the stored choice is kept, not replaced.
        let kept = set(&ctx, "u1", None, None, Some("askjeeves"));
        assert_eq!(kept.search_engine, "bing");
        assert_eq!(get(&ctx, "u1").search_engine, "bing");

        // `None` leaves the stored engine alone too.
        let untouched = set(&ctx, "u1", Some(false), None, None);
        assert_eq!(untouched.search_engine, "bing");
        assert_eq!(untouched.adblock, false);
    }

    #[test]
    fn each_engine_builds_its_search_url() {
        assert_eq!(
            search_url("duckduckgo", "rust webview"),
            "https://duckduckgo.com/?q=rust+webview"
        );
        assert_eq!(
            search_url("brave", "rust webview"),
            "https://search.brave.com/search?q=rust+webview"
        );
        assert_eq!(
            search_url("google", "rust webview"),
            "https://www.google.com/search?q=rust+webview"
        );
        assert_eq!(
            search_url("bing", "rust webview"),
            "https://www.bing.com/search?q=rust+webview"
        );
        // An unknown engine falls back to the default.
        assert_eq!(
            search_url("altavista", "rust"),
            "https://duckduckgo.com/?q=rust"
        );
    }

    #[test]
    fn a_missing_table_is_not_fatal() {
        // No `ensure_schema`: reads fall back to defaults, writes are swallowed.
        let ctx = ctx("/tmp/browser-settings-missing.db");
        assert_eq!(get(&ctx, "u1").adblock, true);
        assert_eq!(get(&ctx, "u1").search_engine, DEFAULT_SEARCH_ENGINE);
        let _ = set(&ctx, "u1", Some(false), None, Some("google"));
        let _ = Db::open("sqlite:///tmp/browser-settings-missing.db").unwrap();
    }
}
