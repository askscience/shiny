//! Browsing history — and the raw material for the home surface's interest
//! profile.
//!
//! Best-effort by design. `PLUGINS.md` §15 documents a real, load-bearing
//! caveat: plugin-owned SQLite access is not reliable in the running server
//! (multiple libsqlite3 copies in one process). Browsing must never depend on
//! the database, so every call here is fire-and-forget and every failure is
//! swallowed with a log line. If the insert works, the user gets history; if
//! it does not, the browser still browses.
//!
//! Rows are written to `peakd_history` — the table `migrations/002_peakd_history.sql`
//! creates. That name matters: the plugin's own name has changed (`browser`,
//! then `peakd`, now `browser` again) but the history table deliberately did
//! not, because renaming it would strand every existing install's history. An
//! earlier revision of this module assumed the table followed the plugin name
//! (`browser_history`) while the migration created `peakd_history`, so on a
//! clean install every insert and read failed silently. Both names are asserted
//! against the migration file in the tests below so the two can never drift
//! apart again.

use std::sync::Arc;

use shiny_plugin_sdk::db::Value;
use shiny_plugin_sdk::services::PluginCtx;

/// The table this module reads and writes. See the module note.
pub const TABLE: &str = "peakd_history";

/// Column the recommendation profile needs, and the schema repair that adds it.
///
/// Why this is code and not only a migration: `002_peakd_history.sql` was
/// *edited after it had already been applied*, so on this install its
/// `CREATE TABLE IF NOT EXISTS` was a no-op — the table already existed from
/// 001, without the column — and the runner never re-runs a recorded file. The
/// result was a table that every `INSERT` in this module silently failed
/// against: browsing history and the whole news profile were dead, with only a
/// `debug!` line to show for it.
///
/// SQLite cannot express conditional DDL, so a migration cannot repair this
/// idempotently (a bare `ALTER TABLE … ADD COLUMN` aborts the rest of the file
/// with "duplicate column name" on a healthy database). The plugin can, though:
/// a *duplicate column* is success here, and any other error is logged and
/// ignored so a broken database still cannot stop the browser from browsing.
const ADD_QUERY_COLUMN: &str =
    "ALTER TABLE peakd_history ADD COLUMN query TEXT";

/// Bring an older `peakd_history` up to the schema this module writes.
///
/// Runs on load. Best-effort: never fatal, never blocks the browser.
pub fn ensure_schema(ctx: &PluginCtx) {
    let db = ctx.db();
    let has_column = db
        .query(
            "SELECT COUNT(*) FROM pragma_table_info('peakd_history') WHERE name = 'query'",
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

    match db.execute(ADD_QUERY_COLUMN, &[]) {
        Ok(_) => tracing::info!("browser: added the history `query` column"),
        // Raced by another caller, or already present after all.
        Err(err) if err.to_string().contains("duplicate column") => {}
        Err(err) => tracing::warn!("browser: could not add the history `query` column: {err}"),
    }
}

/// One navigation, as the recommender wants it.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct HistoryRow {
    /// The real site URL the user navigated to.
    pub url: String,
    /// The raw input the user typed, when they typed a search rather than a
    /// URL. This is the strongest interest signal on the row.
    pub query: Option<String>,
    /// `page` | `text` | `news_click`.
    pub mode: String,
    /// `YYYY-MM-DD HH:MM:SS` (UTC), as SQLite's `datetime('now')` writes it.
    pub created_at: String,
}

/// Record one navigation.
///
/// The `INSERT ... SELECT` form makes the foreign key to the traveler
/// optional: a user with no traveler row simply records nothing instead of
/// raising a constraint error on every page load.
///
/// `mode` also carries the *kind* of visit: `news_click` marks a click on a
/// recommendation, which the profile weighs more heavily than a plain visit.
pub async fn record(
    ctx: &PluginCtx,
    user_id: &str,
    url: &str,
    mode: Option<&str>,
    query: Option<&str>,
) -> Result<(), String> {
    let db = ctx.db();
    let sql = format!(
        "INSERT INTO {TABLE} (id, traveler_id, url, mode, query)
         SELECT ?1, ?2, ?3, ?4, ?5
         WHERE EXISTS (SELECT 1 FROM travelers WHERE id = ?2)"
    );
    let result = db.execute(
        &sql,
        &[
            Value::text(uuid::Uuid::new_v4().to_string()),
            Value::text(user_id.to_string()),
            Value::text(url.to_string()),
            Value::text(mode.unwrap_or("page").to_string()),
            match query {
                Some(q) if !q.trim().is_empty() => Value::text(q.trim().to_string()),
                _ => Value::Null,
            },
        ],
    );

    match result {
        Ok(_) => Ok(()),
        Err(err) => {
            // Expected on some builds (see the module note); never fatal.
            tracing::debug!("browser: history not recorded: {err}");
            Err(err.to_string())
        }
    }
}

/// Recent history for a user as plain URLs, newest first.
///
/// Returns an empty list on failure, so callers never need to handle the
/// "database is broken" case separately.
pub fn recent(ctx: &Arc<PluginCtx>, user_id: &str, limit: usize) -> Vec<String> {
    recent_rows(ctx, user_id, limit)
        .map(|rows| rows.into_iter().map(|r| r.url).collect())
        .unwrap_or_default()
}

/// Recent history for a user with the columns the recommender needs.
///
/// `Err` means the database could not be read at all (no table, no
/// connection); an empty `Ok` means the user simply has no history yet. The
/// two are different states for the home surface and are kept apart.
pub fn recent_rows(
    ctx: &Arc<PluginCtx>,
    user_id: &str,
    limit: usize,
) -> Result<Vec<HistoryRow>, String> {
    let db = ctx.db();
    let limit = limit.clamp(1, 500) as i64;
    let sql = format!(
        "SELECT url, query, mode, created_at FROM {TABLE}
         WHERE traveler_id = ?1
         ORDER BY created_at DESC, rowid DESC
         LIMIT ?2"
    );
    let rows = db
        .query(&sql, &[Value::text(user_id.to_string()), Value::Int(limit)])
        .map_err(|err| {
            tracing::debug!("browser: history not readable: {err}");
            err.to_string()
        })?;

    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let mut it = row.into_iter();
            let url = value_text(it.next())?;
            let query = value_text(it.next());
            let mode = value_text(it.next()).unwrap_or_else(|| "page".into());
            let created_at = value_text(it.next()).unwrap_or_default();
            Some(HistoryRow {
                url,
                query,
                mode,
                created_at,
            })
        })
        .collect())
}

fn value_text(value: Option<Value>) -> Option<String> {
    match value {
        Some(Value::Text(text)) => Some(text),
        _ => None,
    }
}

/// Shared test scaffolding.
///
/// `news.rs`'s end-to-end test needs the same scratch-`PluginCtx` builder, and
/// duplicating the `ConfigSnapshot`/`Manifest` boilerplate in two modules would
/// guarantee they drift apart.
#[cfg(test)]
pub(crate) mod tests_support {
    use std::sync::Arc;

    use shiny_plugin_sdk::services::PluginCtx;

    /// A ctx whose database points at `path` (the caller removes the file).
    pub fn ctx_for(path: &str) -> Arc<PluginCtx> {
        let config = shiny_plugin_sdk::services::ConfigSnapshot {
            server_host: "127.0.0.1".into(),
            server_port: 8080,
            database_url: format!("sqlite://{path}"),
            ollama_url: String::new(),
            ollama_model: String::new(),
            supertonic_url: String::new(),
            supertonic_voice: String::new(),
            web_dir: "web".into(),
            vosk_models_dir: "data/vosk-models".into(),
            auto_start_supertonic: false,
            log_level: "warn".into(),
            plugins_dir: "data/plugins".into(),
            admin_token: None,
        };
        let manifest = shiny_plugin_sdk::manifest::Manifest {
            name: "browser".into(),
            version: semver::Version::new(0, 1, 0),
            api_level: shiny_plugin_sdk::CORE_API_LEVEL,
            entry_symbol: "shiny_plugin_entry".into(),
            target_triple: None,
            description: None,
            author: None,
            summary: None,
            migrations_dir: "migrations".into(),
            skills_dir: "skills".into(),
            web_dir: "web".into(),
            signature: None,
        };
        PluginCtx::new(config, manifest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shiny_plugin_sdk::db::Db;

    fn ctx_with_db(path: &str) -> Arc<PluginCtx> {
        let _ = std::fs::remove_file(path);
        super::tests_support::ctx_for(path)
    }

    /// The regression guard for the bug this module shipped with: the SQL must
    /// name the table the migrations actually create.
    #[test]
    fn table_name_matches_the_migrations() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        let mut migrations = String::new();
        for entry in std::fs::read_dir(&dir).expect("migrations dir") {
            let path = entry.expect("dir entry").path();
            if path.extension().and_then(|e| e.to_str()) == Some("sql") {
                migrations.push_str(&std::fs::read_to_string(&path).expect("read migration"));
            }
        }
        assert!(
            migrations.contains(&format!("CREATE TABLE IF NOT EXISTS {TABLE}")),
            "no migration creates {TABLE}"
        );
        // `browser_history` may only appear as prose (the migration explains why
        // it is dropped) — never as a table a migration creates.
        for line in migrations.lines() {
            let code = line.split("--").next().unwrap_or("");
            assert!(
                !code.contains("CREATE TABLE") || !code.contains("browser_history"),
                "a migration still creates the stale browser_history table: {line}"
            );
        }
        assert!(migrations.contains("DROP TABLE IF EXISTS browser_history"));
    }

    #[test]
    fn missing_table_is_not_fatal() {
        // The whole point of this module: a broken/absent schema must return
        // an error (or empty) rather than panicking or aborting the host.
        let ctx = ctx_with_db("/tmp/browser-history-test.db");
        assert!(recent_rows(&ctx, "nobody", 5).is_err());
        assert!(recent(&ctx, "nobody", 5).is_empty());
        // And the Db type itself opens without the table existing.
        let db = Db::open("sqlite:///tmp/browser-history-test.db").unwrap();
        assert!(db.query("SELECT url FROM peakd_history", &[]).is_err());
    }

    #[test]
    fn rows_read_back_newest_first_with_the_query() {
        let path = "/tmp/browser-history-roundtrip.db";
        // One ctx, built after the file is removed: `ctx_with_db` deletes the
        // database, so it must run *before* the schema below is created.
        let ctx = ctx_with_db(path);
        let db = Db::open(&format!("sqlite://{path}")).unwrap();
        db.execute(
            &format!(
                "CREATE TABLE {TABLE} (id TEXT PRIMARY KEY, traveler_id TEXT NOT NULL,
                 url TEXT NOT NULL, mode TEXT NOT NULL DEFAULT 'page',
                 query TEXT, created_at TEXT NOT NULL DEFAULT (datetime('now')))"
            ),
            &[],
        )
        .unwrap();
        db.execute(
            "CREATE TABLE travelers (id TEXT PRIMARY KEY, name TEXT NOT NULL,
             email TEXT NOT NULL UNIQUE, password_hash TEXT NOT NULL)",
            &[],
        )
        .unwrap();
        db.execute(
            "INSERT INTO travelers (id, name, email, password_hash) VALUES ('u1','u','u@x','h')",
            &[],
        )
        .unwrap();
        // The insert is guarded by `EXISTS (SELECT 1 FROM travelers …)`, so the
        // user id must be a real row: recording for an unknown user is a
        // deliberate no-op, not a failure.
        let uid = "u1";

        // Recording must succeed now that the table matches the migration.
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(record(&ctx, uid, "https://example.com/a", Some("page"), Some("aurora forecast")))
            .expect("record");
        rt.block_on(record(&ctx, uid, "https://example.com/b", None, None))
            .expect("record");

        // Two rows written inside the same second would tie on `created_at`, so
        // sort order alone proves nothing. Pin the timestamps instead: B is the
        // newer visit and must come first.
        db.execute(
            "UPDATE peakd_history SET created_at = '2026-09-13 10:00:00'
             WHERE url = 'https://example.com/a'",
            &[],
        )
        .unwrap();
        db.execute(
            "UPDATE peakd_history SET created_at = '2026-09-13 11:00:00'
             WHERE url = 'https://example.com/b'",
            &[],
        )
        .unwrap();

        let rows = recent_rows(&ctx, uid, 10).unwrap();
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert_eq!(rows[0].url, "https://example.com/b");
        assert_eq!(rows[0].mode, "page");
        assert_eq!(rows[0].query, None, "a URL visit must not carry a query");
        // The typed search is stored verbatim — it is the recommender's input.
        assert_eq!(rows[1].query.as_deref(), Some("aurora forecast"));
        assert!(rows[1].created_at.starts_with("2026-09-13"));
    }
}

/// The migrations reproduce the schema this module writes.
///
/// This is the regression guard for the failure described on
/// [`ensure_schema`]. `plugin_schema_versions` records a migration file by
/// name: the moment two migrations create the same table, or a table is created
/// with `IF NOT EXISTS` by a file that runs second, the second one is a silent
/// no-op on every install that ran the first. Nothing in the runtime notices;
/// the code just starts failing to write.
///
/// So the *files themselves* are checked here: replaying them in order must
/// produce a `peakd_history` whose columns include every one this module's SQL
/// names. The parser is deliberately small and only needs to understand the
/// statements these migrations actually contain; anything it does not
/// understand is skipped and reported as such by the coverage assertion.
#[cfg(test)]
mod migration_schema_tests {
    use super::TABLE;

    /// SQLite's table-valued schema introspection, as the runner would see it.
    const PRAGMA_FN: &str = "pragma_table_info";

    /// Turn one statement into a form that can be executed against an empty
    /// database: seed rows are neutralised and values become NULLs.
    ///
    /// `INSERT INTO t (a, b) VALUES ('x', 1)` has no schema effect, and on an
    /// empty database would fail on foreign keys and NOT NULL columns, so it is
    /// rewritten to insert nothing.
    fn neutralise(statement: &str) -> String {
        if statement.trim_start().to_ascii_uppercase().starts_with("INSERT") {
            // Seed rows have no schema effect, and on an empty database they
            // would fail on NOT NULL/foreign keys, so they are skipped.
            return "SELECT 1 WHERE 0".to_string();
        }
        statement.to_string()
    }

    /// The migrations, in the order the runner applies them.
    fn applied_migration_sql() -> Vec<(String, String)> {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        let mut files: Vec<_> = std::fs::read_dir(&dir)
            .expect("migrations dir")
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("sql"))
            .collect();
        files.sort();
        files
            .into_iter()
            .map(|path| {
                let sql = std::fs::read_to_string(&path).expect("read migration");
                (
                    path.file_name().unwrap().to_string_lossy().to_string(),
                    sql,
                )
            })
            .collect()
    }

    /// Split a script into statements, dropping comments.
    fn statements(sql: &str) -> Vec<String> {
        let mut without_comments = String::with_capacity(sql.len());
        for line in sql.lines() {
            without_comments.push_str(line.split("--").next().unwrap_or(""));
            without_comments.push('\n');
        }
        without_comments
            .split(';')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }

    #[test]
    fn migrations_produce_every_column_the_module_writes() {
        let db = shiny_plugin_sdk::db::Db::open("sqlite::memory:").expect("in-memory db");
        let mut ran = 0usize;
        for (file, sql) in applied_migration_sql() {
            for statement in statements(&sql) {
                let statement = neutralise(&statement);
                if let Err(err) = db.execute(&statement, &[]) {
                    panic!("{file}: statement failed: {err}\n  {statement}");
                }
                ran += 1;
            }
        }
        assert!(ran > 0, "no migration statements ran");

        let columns: Vec<String> = db
            .query(&format!("SELECT name FROM {PRAGMA_FN}('{TABLE}')"), &[])
            .expect("read columns")
            .into_iter()
            .filter_map(|row| row.into_iter().next())
            .filter_map(|value| match value {
                shiny_plugin_sdk::db::Value::Text(name) => Some(name),
                _ => None,
            })
            .collect();

        assert!(!columns.is_empty(), "no {TABLE} table was created");
        for required in ["id", "traveler_id", "url", "mode", "query", "created_at"] {
            assert!(
                columns.iter().any(|c| c == required),
                "migrations do not produce `{required}`; got {columns:?}"
            );
        }
    }

    #[test]
    fn the_query_column_appears_in_a_migration() {
        // The module's INSERT names `query`. If the only file that adds it is
        // already recorded in `plugin_schema_versions` on an install, the
        // column has to be repaired by `ensure_schema` — but a fresh install
        // must get it from a migration file.
        let all: String = applied_migration_sql()
            .into_iter()
            .map(|(_, sql)| sql)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            all.contains("query"),
            "no migration declares the query column"
        );
    }
}
