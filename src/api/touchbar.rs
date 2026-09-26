//! Touch Bar capability API.
//!
//! One read: whether this machine has a T2 Touch Bar the web UI should listen
//! to. Any authenticated client may ask; there is nothing to mutate.

use axum::extract::State;
use axum::Json;
use serde_json::Value;

use crate::api::AppState;

/// GET /api/touchbar
pub async fn status(State(state): State<AppState>, headers: axum::http::HeaderMap) -> Json<Value> {
    let data = crate::api::remote::gate_host_status(
        serde_json::to_value(state.touchbar.status()).unwrap_or_default(),
        &headers,
    );
    Json(serde_json::json!({ "success": true, "data": data }))
}
