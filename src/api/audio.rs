//! Sound panel API — the host's PipeWire volume/mute/default devices for the
//! top-bar chip and the Sound menu. Backed by [`crate::services::audio`],
//! which caches snapshots from `pactl` and broadcasts them over a channel;
//! this module never spawns a process itself.
//!
//! Reads (`status`, `events`) are available to any authenticated client.
//! Mutations change the machine's audio, and the server binds `0.0.0.0`, so
//! they are accepted only from the local machine (loopback) — the same rule
//! the network panel uses.

use std::convert::Infallible;
use std::net::SocketAddr;

use axum::extract::{ConnectInfo, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::Json;
use futures::Stream;
use futures::StreamExt;
use serde::Deserialize;
use serde_json::Value;
use tokio_stream::wrappers::BroadcastStream;

use crate::api::AppState;
use crate::errors::AppError;
use crate::services::audio::{AudioStatus, AudioTarget};

/// GET /api/audio/status
pub async fn status(State(state): State<AppState>, headers: axum::http::HeaderMap) -> Json<Value> {
    let data = crate::api::remote::gate_host_status(
        serde_json::to_value(state.audio.status()).unwrap_or_default(),
        &headers,
    );
    Json(serde_json::json!({ "success": true, "data": data }))
}

/// GET /api/audio/events
pub async fn events(
    State(state): State<AppState>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    // Subscribe before reading the snapshot: an update that lands in between
    // is delivered by the live stream, never dropped.
    let receiver = state.audio.subscribe();
    let initial = state.audio.status();
    let live = BroadcastStream::new(receiver).filter_map(|message| async move {
        message
            .ok()
            .map(|snapshot| Ok::<Event, Infallible>(snapshot_event(&snapshot)))
    });
    let first =
        futures::stream::once(async move { Ok::<Event, Infallible>(snapshot_event(&initial)) });

    Ok(Sse::new(first.chain(live)).keep_alive(KeepAlive::default()))
}

#[derive(Deserialize)]
pub struct VolumeBody {
    target: AudioTarget,
    /// Node index from the latest snapshot; omit for the default device.
    #[serde(default)]
    id: Option<u32>,
    /// Absolute volume, 0–100.
    percent: u8,
}

/// POST /api/audio/volume
pub async fn volume(
    State(state): State<AppState>,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    Json(body): Json<VolumeBody>,
) -> Result<Json<Value>, AppError> {
    require_local(&remote)?;
    state
        .audio
        .set_volume(body.target, body.id, body.percent)
        .await
        .map_err(AppError::BadRequest)?;
    Ok(Json(serde_json::json!({ "success": true })))
}

#[derive(Deserialize)]
pub struct MuteBody {
    target: AudioTarget,
    #[serde(default)]
    id: Option<u32>,
    /// `true`/`false` to set, omitted to toggle.
    #[serde(default)]
    muted: Option<bool>,
}

/// POST /api/audio/mute
pub async fn mute(
    State(state): State<AppState>,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    Json(body): Json<MuteBody>,
) -> Result<Json<Value>, AppError> {
    require_local(&remote)?;
    state
        .audio
        .set_mute(body.target, body.id, body.muted)
        .await
        .map_err(AppError::BadRequest)?;
    Ok(Json(serde_json::json!({ "success": true })))
}

#[derive(Deserialize)]
pub struct DefaultBody {
    target: AudioTarget,
    /// Node index from the latest snapshot.
    id: u32,
}

/// POST /api/audio/default
pub async fn default_device(
    State(state): State<AppState>,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    Json(body): Json<DefaultBody>,
) -> Result<Json<Value>, AppError> {
    require_local(&remote)?;
    state
        .audio
        .set_default(body.target, body.id)
        .await
        .map_err(AppError::BadRequest)?;
    Ok(Json(serde_json::json!({ "success": true })))
}

/// Volume, mute and routing are host-level capabilities and the server is
/// reachable on the LAN, so anything that changes them must come from the
/// machine itself.
fn require_local(remote: &SocketAddr) -> Result<(), AppError> {
    if remote.ip().is_loopback() {
        return Ok(());
    }
    Err(AppError::Unauthorized(
        "sound changes are only allowed from the local machine".into(),
    ))
}

fn snapshot_event(snapshot: &AudioStatus) -> Event {
    Event::default()
        .event("audio")
        .data(serde_json::to_string(snapshot).unwrap_or_default())
}
