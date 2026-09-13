//! Browsing history.
//!
//! Best-effort by design. `PLUGINS.md` §15 documents a real, load-bearing
//! caveat: plugin-owned SQLite access is not reliable in the running server
//! (multiple libsqlite3 copies in one process). Browsing must never depend on
//! the database, so every call here is fire-and-forget and every failure is
//! swallowed with a log line. If the insert works, the user gets history; if
//! it does not, the browser still browses.

use std::sync::Arc;

use shiny_plugin_sdk::db::Value;
use shiny_plugin_sdk::services::PluginCtx;

/// Record one navigation.
///
/// The `INSERT ... SELECT` form makes the foreign key to the traveler
/// optional: a user with no traveler row simply records nothing instead of
/// raising a constraint error on every page load.
pub async fn record(
    ctx: &Arc<PluginCtx>,
    user_id: &str,
    url: &str,
    mode: Option<&str>,
) -> Result<(), String> {
    let db = ctx.db();
    let result = db.execute(
        "INSERT INTO browser_history (id, traveler_id, url, mode)
         SELECT ?1, ?2, ?3, ?4
         WHERE EXISTS (SELECT 1 FROM travelers WHERE id = ?2)",
        &[
            Value::text(uuid::Uuid::new_v4().to_string()),
            Value::text(user_id.to_string()),
            Value::text(url.to_string()),
            Value::text(mode.unwrap_or("page").to_string()),
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

/// Recent history for a user, newest first. Returns an empty list on failure.
pub fn recent(ctx: &Arc<PluginCtx>, user_id: &str, limit: usize) -> Vec<String> {
    let db = ctx.db();
    let limit = limit.clamp(1, 200) as i64;
    let rows = db.query(
        "SELECT url FROM browser_history
         WHERE traveler_id = ?1
         ORDER BY created_at DESC
         LIMIT ?2",
        &[Value::text(user_id.to_string()), Value::Int(limit)],
    );

    match rows {
        Ok(rows) => rows
            .into_iter()
            .filter_map(|row| row.into_iter().next())
            .filter_map(|value| match value {
                Value::Text(url) => Some(url),
                _ => None,
            })
            .collect(),
        Err(err) => {
            tracing::debug!("browser: history not readable: {err}");
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shiny_plugin_sdk::db::Db;

    fn ctx_with_db(path: &str) -> Arc<PluginCtx> {
        let _ = std::fs::remove_file(path);
        // Build a ctx whose database_url points at a scratch file, then create
        // the schema the migration would have created.
        let mut config = shiny_plugin_sdk::services::ConfigSnapshot {
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
            name: "peakd".into(),
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
        let _ = &mut config;
        PluginCtx::new(config, manifest)
    }

    #[test]
    fn missing_table_is_not_fatal() {
        // The whole point of this module: a broken/absent schema must return
        // an error (or empty) rather than panicking or aborting the host.
        let ctx = ctx_with_db("/tmp/peakd-history-test.db");
        let _ = recent(&ctx, "nobody", 5);
        // And the Db type itself opens without the table existing.
        let db = Db::open("sqlite:///tmp/peakd-history-test.db").unwrap();
        assert!(db.query("SELECT url FROM browser_history", &[]).is_err());
    }
}
