//! Pointer settings API — the mouse and touchpad options of Settings.
//!
//! Reads are open to any authenticated client (a remote one is told the host
//! is unavailable); the write changes how the machine's pointer behaves, so it
//! is accepted only from the local machine, like the other host panels. See
//! `services/pointer.rs`.

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, State};
use axum::Json;
use serde_json::Value;

use crate::api::AppState;
use crate::errors::AppError;

/// GET /api/pointer
pub async fn status(State(state): State<AppState>, headers: axum::http::HeaderMap) -> Json<Value> {
    let data = crate::api::remote::gate_host_status(state.pointer.status().await, &headers);
    Json(serde_json::json!({ "success": true, "data": data }))
}

/// PUT /api/pointer — a partial change, e.g. `{ "touchpad": { "tap_to_click": true } }`.
/// A `null` value returns that option to the driver default.
pub async fn update(
    State(state): State<AppState>,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, AppError> {
    if !remote.ip().is_loopback() {
        return Err(AppError::Unauthorized(
            "pointer changes are only allowed from the local machine".into(),
        ));
    }
    let data = state.pointer.update(body).await.map_err(AppError::BadRequest)?;
    Ok(Json(serde_json::json!({ "success": true, "data": data })))
}
