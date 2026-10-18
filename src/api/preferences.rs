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
    let data = crate::services::preferences::PreferenceStore::new(&state.pool)
        .list(&traveler.id)
        .await?;
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
    crate::services::preferences::PreferenceStore::new(&state.pool)
        .put(&traveler.id, &body)
        .await?;

    Ok(Json(serde_json::json!({ "success": true })))
}
