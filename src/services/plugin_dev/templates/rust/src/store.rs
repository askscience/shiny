//! The plugin's data: one table, created by `migrations/001_init.sql`.
//!
//! All database access goes through `ctx.db()`, the plugin's own synchronous
//! SQLite connection. Every row belongs to a user: always filter by `user_id`.

use serde_json::{json, Value as Json};
use shiny_plugin_sdk::db::Value;
use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::services::PluginCtx;
use shiny_plugin_sdk::util::as_text;

pub const MAX_TEXT: usize = 500;

/// Add one item for `user_id` and return it.
pub fn add(ctx: &PluginCtx, user_id: &str, text: &str) -> Result<Json, AppError> {
    let text: String = text.trim().chars().take(MAX_TEXT).collect();
    if text.is_empty() {
        return Err(AppError::BadRequest("text required".into()));
    }
    let id = uuid::Uuid::new_v4().to_string();
    ctx.db().execute(
        "INSERT INTO @@IDENT@@_items (id, user_id, text) VALUES (?1, ?2, ?3)",
        &[Value::text(&id), Value::text(user_id), Value::text(&text)],
    )?;
    Ok(json!({ "id": id, "text": text }))
}

/// The user's items, newest first.
pub fn list(ctx: &PluginCtx, user_id: &str, limit: i64) -> Result<Vec<Json>, AppError> {
    let rows = ctx.db().query(
        "SELECT id, text, created_at FROM @@IDENT@@_items WHERE user_id = ?1 \
         ORDER BY created_at DESC, rowid DESC LIMIT ?2",
        &[Value::text(user_id), Value::Int(limit.clamp(1, 200))],
    )?;
    Ok(rows
        .iter()
        .map(|row| json!({ "id": as_text(&row[0]), "text": as_text(&row[1]), "created_at": as_text(&row[2]) }))
        .collect())
}
