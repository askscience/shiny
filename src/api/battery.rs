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
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::Json;
use futures::Stream;
use futures::StreamExt;
use serde_json::Value;
use tokio_stream::wrappers::BroadcastStream;

use crate::api::AppState;
use crate::errors::AppError;
use crate::services::battery::BatteryStatus;

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
    // Subscribe before reading the snapshot: an update that lands in between
    // is delivered by the live stream, never dropped.
    let receiver = state.battery.subscribe();
    let initial = state.battery.status();
    let live = BroadcastStream::new(receiver).filter_map(|message| async move {
        message
            .ok()
            .map(|snapshot| Ok::<Event, Infallible>(snapshot_event(&snapshot)))
    });
    let first =
        futures::stream::once(async move { Ok::<Event, Infallible>(snapshot_event(&initial)) });

    Ok(Sse::new(first.chain(live)).keep_alive(KeepAlive::default()))
}

fn snapshot_event(snapshot: &BatteryStatus) -> Event {
    Event::default()
        .event("battery")
        .data(serde_json::to_string(snapshot).unwrap_or_default())
}
