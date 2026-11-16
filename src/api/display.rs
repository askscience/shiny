//! Display scale API — the host's interface scale for the kiosk webview.
//!
//! Reads (`status`) are available to any authenticated client. The write
//! changes how the whole machine renders, and the server binds `0.0.0.0`, so
//! it is accepted only from the local machine (loopback) — the same rule the
//! network and audio panels use.
//!
//! The value is persisted for the shell (`liteaura`) to pick up; this server does
//! not touch WebKit.

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, State};
use axum::Json;
use serde_json::Value;

use crate::api::AppState;
use crate::errors::AppError;

/// Refusal for a caller that is not the machine itself.
const LOCAL_ONLY: &str = "display changes are only allowed from the local machine";

/// GET /api/display
pub async fn status(State(state): State<AppState>, headers: axum::http::HeaderMap) -> Json<Value> {
    let data = crate::api::remote::gate_host_status(
        serde_json::to_value(state.display.status()).unwrap_or_default(),
        &headers,
    );
    Json(serde_json::json!({ "success": true, "data": data }))
}

/// PUT /api/display — body `{ "scale": "auto" | 1.25 }`.
pub async fn set_scale(
    State(state): State<AppState>,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, AppError> {
    crate::api::remote::require_loopback(&remote, LOCAL_ONLY)?;
    let value = body.get("scale").cloned().unwrap_or(Value::Null);
    let data = state
        .display
        .set_scale(&value)
        .map_err(AppError::BadRequest)?;
    Ok(Json(serde_json::json!({ "success": true, "data": data })))
}
