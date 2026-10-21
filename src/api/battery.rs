//! Battery panel API — the host's charge level for the top-bar chip. Backed by
//! [`crate::services::battery`], which caches snapshots read from the kernel's
//! sysfs power-supply class and broadcasts them; this module never touches the
//! filesystem itself.
//!
//! Reads (`status`, `events`) are available to any authenticated client. There
//! are no mutations — the battery is read-only — so a remote client only sees
//! `available:false` on the status payload (the host panel gate).

use std::convert::Infallible;

use axum::extract::State;
use axum::response::sse::{Event, Sse};
use axum::Json;
use futures::Stream;
use serde_json::Value;

use crate::api::AppState;
use crate::errors::AppError;

/// GET /api/battery/status
pub async fn status(State(state): State<AppState>, headers: axum::http::HeaderMap) -> Json<Value> {
    let data = crate::api::remote::gate_host_status(
        serde_json::to_value(state.battery.status()).unwrap_or_default(),
        &headers,
    );
    Json(serde_json::json!({ "success": true, "data": data }))
}

/// GET /api/battery/events
pub async fn events(
    State(state): State<AppState>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let receiver = state.battery.subscribe();
    let initial = state.battery.status();
    Ok(crate::api::sse::snapshot_stream("battery", receiver, initial))
}

