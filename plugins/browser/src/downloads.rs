//! Download history, persisted server-side so the window's Downloads panel
//! survives a closed window or a shell restart.
//!
//! The **shell** performs every download; the window relays each lifecycle
//! event here. Best-effort by design (`PLUGINS.md` §15): a browser must keep
//! downloading even when plugin-owned SQLite is unavailable, so every write is
//! fire-and-forget and every failure is a log line.

use std::sync::Arc;

use serde_json::{json, Value as Json};
use shiny_plugin_sdk::db::Value;
use shiny_plugin_sdk::services::PluginCtx;

pub const TABLE: &str = "browser_downloads";

/// Ensure the table exists on an install whose migration has not run.
pub fn ensure_schema(ctx: &PluginCtx) {
    let _ = ctx.db().execute(
        "CREATE TABLE IF NOT EXISTS browser_downloads (\
             id TEXT NOT NULL, traveler_id TEXT NOT NULL, url TEXT NOT NULL, host TEXT, \
             file_name TEXT, file_path TEXT, mime TEXT, \
             total INTEGER NOT NULL DEFAULT 0, received INTEGER NOT NULL DEFAULT 0, \
             state TEXT NOT NULL DEFAULT 'download', error TEXT, \
             incognito INTEGER NOT NULL DEFAULT 0, \
             created_at TEXT NOT NULL DEFAULT (datetime('now')), \
             updated_at TEXT NOT NULL DEFAULT (datetime('now')), \
             PRIMARY KEY (traveler_id, id))",
        &[],
    );
}

/// Upsert one download as the shell reports it. `item` is the shell's payload.
pub fn upsert(ctx: &Arc<PluginCtx>, user_id: &str, item: &Json) {
    let id = text(item, "id");
    if id.is_empty() {
        return;
    }
    let sql = format!(
        "INSERT INTO {TABLE} \
         (id, traveler_id, url, host, file_name, file_path, mime, total, received, state, error, incognito, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, datetime('now'), datetime('now')) \
         ON CONFLICT(traveler_id, id) DO UPDATE SET \
             url = ?3, host = ?4, file_name = ?5, file_path = ?6, mime = ?7, \
             total = ?8, received = ?9, state = ?10, error = ?11, incognito = ?12, \
             updated_at = datetime('now')"
    );
    let result = ctx.db().execute(
        &sql,
        &[
            Value::text(id),
            Value::text(user_id.to_string()),
            Value::text(text(item, "url")),
            text_opt(item, "host"),
            text_opt(item, "file"),
            text_opt(item, "path"),
            text_opt(item, "mime"),
            Value::Int(item.get("total").and_then(Json::as_i64).unwrap_or(0)),
            Value::Int(item.get("received").and_then(Json::as_i64).unwrap_or(0)),
            Value::text(text(item, "state")),
            text_opt(item, "error"),
            Value::Int(i64::from(
                item.get("incognito").and_then(Json::as_bool).unwrap_or(false),
            )),
        ],
    );
    if let Err(err) = result {
        tracing::debug!("browser: download not persisted: {err}");
    }
}

/// Recent downloads, newest first.
pub fn list(ctx: &Arc<PluginCtx>, user_id: &str, limit: usize) -> Vec<Json> {
    let limit = limit.clamp(1, 200) as i64;
    let sql = format!(
        "SELECT id, url, host, file_name, file_path, mime, total, received, state, error, \
                incognito, created_at, updated_at \
         FROM {TABLE} WHERE traveler_id = ?1 ORDER BY created_at DESC, rowid DESC LIMIT ?2"
    );
    let rows = ctx
        .db()
        .query(&sql, &[Value::text(user_id.to_string()), Value::Int(limit)])
        .unwrap_or_default();
    rows.into_iter().map(row_json).collect()
}

pub fn remove(ctx: &Arc<PluginCtx>, user_id: &str, id: &str) {
    let sql = format!("DELETE FROM {TABLE} WHERE traveler_id = ?1 AND id = ?2");
    let _ = ctx
        .db()
        .execute(&sql, &[Value::text(user_id.to_string()), Value::text(id.to_string())]);
}

/// Drop completed / cancelled / interrupted rows.
pub fn clear_finished(ctx: &Arc<PluginCtx>, user_id: &str) {
    let sql = format!(
        "DELETE FROM {TABLE} WHERE traveler_id = ?1 \
         AND state IN ('completed', 'cancelled', 'interrupted')"
    );
    let _ = ctx.db().execute(&sql, &[Value::text(user_id.to_string())]);
}

fn row_json(row: Vec<Value>) -> Json {
    let mut it = row.into_iter();
    json!({
        "id": as_text(it.next()),
        "url": as_text(it.next()),
        "host": as_text(it.next()),
        "file": as_text(it.next()),
        "path": as_text(it.next()),
        "mime": as_text(it.next()),
        "total": as_int(it.next()),
        "received": as_int(it.next()),
        "state": as_text(it.next()),
        "error": as_text(it.next()),
        "incognito": as_int(it.next()) != 0,
        "created_at": as_text(it.next()),
        "updated_at": as_text(it.next()),
    })
}

fn as_text(value: Option<Value>) -> String {
    match value {
        Some(Value::Text(t)) => t,
        _ => String::new(),
    }
}

fn as_int(value: Option<Value>) -> i64 {
    match value {
        Some(Value::Int(n)) => n,
        _ => 0,
    }
}

fn text(item: &Json, key: &str) -> String {
    item.get(key).and_then(Json::as_str).unwrap_or_default().to_string()
}

fn text_opt(item: &Json, key: &str) -> Value {
    match item.get(key).and_then(Json::as_str) {
        Some(s) if !s.is_empty() => Value::text(s.to_string()),
        _ => Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_a_download() {
        let ctx = crate::history::tests_support::ctx_for("/tmp/browser-downloads.db");
        ensure_schema(&ctx);
        upsert(
            &ctx,
            "u1",
            &json!({ "id": "d1", "url": "https://example.com/a.zip", "host": "example.com",
                     "file": "a.zip", "path": "/home/u/Downloads/a.zip", "mime": "application/zip",
                     "total": 100, "received": 40, "state": "downloading" }),
        );
        let rows = list(&ctx, "u1", 10);
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0]["id"], "d1");
        assert_eq!(rows[0]["state"], "downloading");

        // A later event updates the same row.
        upsert(
            &ctx,
            "u1",
            &json!({ "id": "d1", "url": "https://example.com/a.zip", "state": "completed",
                     "total": 100, "received": 100 }),
        );
        let rows = list(&ctx, "u1", 10);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["state"], "completed");
        assert_eq!(rows[0]["received"], 100);

        remove(&ctx, "u1", "d1");
        assert!(list(&ctx, "u1", 10).is_empty());
    }

    #[test]
    fn clear_finished_keeps_active() {
        let ctx = crate::history::tests_support::ctx_for("/tmp/browser-downloads-clear.db");
        ensure_schema(&ctx);
        for (id, state) in [("d1", "completed"), ("d2", "downloading")] {
            upsert(&ctx, "u1", &json!({ "id": id, "url": "https://x/", "state": state }));
        }
        clear_finished(&ctx, "u1");
        let rows = list(&ctx, "u1", 10);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["id"], "d2");
    }

    #[test]
    fn a_missing_table_is_not_fatal() {
        let ctx = crate::history::tests_support::ctx_for("/tmp/browser-downloads-missing.db");
        assert!(list(&ctx, "u1", 10).is_empty());
        upsert(&ctx, "u1", &json!({ "id": "d1", "url": "https://x/" }));
    }
}
