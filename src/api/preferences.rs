//! Per-user preferences API. Backs the frontend's settings, desktop/tiling
//! layout, plugin window layout, assistant name/model — every user has an
//! isolated key/value space in `user_preferences` (scoped by user_id).

use axum::extract::{Extension, State};
use axum::Json;
use serde_json::Value;

use crate::api::AppState;
use crate::errors::AppError;
use crate::models::Traveler;

/// GET /api/preferences — all key/value pairs for the current user.
pub async fn get_preferences(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
) -> Result<Json<Value>, AppError> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT key, value FROM user_preferences WHERE user_id = ?1",
    )
    .bind(&traveler.id)
    .fetch_all(&state.pool)
    .await
    .map_err(AppError::Database)?;

    let data: serde_json::Map<String, Value> = rows
        .into_iter()
        .map(|(key, value)| (key, Value::String(value)))
        .collect();

    Ok(Json(serde_json::json!({ "success": true, "data": data })))
}

/// PUT /api/preferences — upsert an object of key/value pairs for the current
/// user. Values are stored as strings (the frontend JSON-encodes structured
/// values such as workspaces and the tiling layout).
pub async fn put_preferences(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, AppError> {
    let obj = body
        .as_object()
        .ok_or_else(|| AppError::BadRequest("expected a JSON object".into()))?;

    for (key, value) in obj {
        let raw = match value {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        sqlx::query(
            "INSERT INTO user_preferences (user_id, key, value, updated_at) \
             VALUES (?1, ?2, ?3, datetime('now')) \
             ON CONFLICT(user_id, key) DO UPDATE SET value = excluded.value, updated_at = datetime('now')",
        )
        .bind(&traveler.id)
        .bind(key)
        .bind(&raw)
        .execute(&state.pool)
        .await
        .map_err(AppError::Database)?;
    }

    Ok(Json(serde_json::json!({ "success": true })))
}
