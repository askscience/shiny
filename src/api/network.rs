//! Network panel API — the host's Wi-Fi/Ethernet state for the top-bar chip
//! and the Network window. Backed by [`crate::services::network`], which
//! caches snapshots from NetworkManager and broadcasts them over a channel;
//! this module never talks to D-Bus itself.
//!
//! Reads (`status`, `events`, `scan`) are available to any authenticated
//! client. Mutations change the machine's connectivity, and the server binds
//! `0.0.0.0`, so they are accepted only from the local machine (loopback).

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
use crate::services::network::NetworkStatus;

/// GET /api/network/status
pub async fn status(State(state): State<AppState>, headers: axum::http::HeaderMap) -> Json<Value> {
    let data = crate::api::remote::gate_host_status(
        serde_json::to_value(state.network.status()).unwrap_or_default(),
        &headers,
    );
    Json(serde_json::json!({ "success": true, "data": data }))
}

/// GET /api/network/events
pub async fn events(
    State(state): State<AppState>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    // Subscribe before reading the snapshot: an update that lands in between
    // is delivered by the live stream, never dropped.
    let receiver = state.network.subscribe();
    let initial = state.network.status();
    let live = BroadcastStream::new(receiver).filter_map(|message| async move {
        message
            .ok()
            .map(|snapshot| Ok::<Event, Infallible>(snapshot_event(&snapshot)))
    });
    let first = futures::stream::once(async move { Ok::<Event, Infallible>(snapshot_event(&initial)) });

    Ok(Sse::new(first.chain(live)).keep_alive(KeepAlive::default()))
}

/// POST /api/network/scan
pub async fn scan(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    state.network.scan().await.map_err(AppError::Internal)?;
    Ok(Json(serde_json::json!({ "success": true })))
}

#[derive(Deserialize)]
pub struct ConnectBody {
    ssid: String,
    #[serde(default)]
    interface: Option<String>,
    #[serde(default)]
    password: Option<String>,
}

/// POST /api/network/connect
pub async fn connect(
    State(state): State<AppState>,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    Json(body): Json<ConnectBody>,
) -> Result<Json<Value>, AppError> {
    require_local(&remote)?;
    state
        .network
        .connect(&body.ssid, body.interface.as_deref(), body.password.as_deref())
        .await
        .map_err(AppError::BadRequest)?;
    Ok(Json(serde_json::json!({ "success": true })))
}

/// POST /api/network/disconnect
pub async fn disconnect(
    State(state): State<AppState>,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
) -> Result<Json<Value>, AppError> {
    require_local(&remote)?;
    state
        .network
        .disconnect()
        .await
        .map_err(AppError::BadRequest)?;
    Ok(Json(serde_json::json!({ "success": true })))
}

#[derive(Deserialize)]
pub struct ForgetBody {
    uuid: String,
}

/// POST /api/network/forget
pub async fn forget(
    State(state): State<AppState>,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    Json(body): Json<ForgetBody>,
) -> Result<Json<Value>, AppError> {
    require_local(&remote)?;
    state
        .network
        .forget(&body.uuid)
        .await
        .map_err(AppError::BadRequest)?;
    Ok(Json(serde_json::json!({ "success": true })))
}

#[derive(Deserialize)]
pub struct WifiPowerBody {
    enabled: bool,
}

/// POST /api/network/wifi-power
pub async fn wifi_power(
    State(state): State<AppState>,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    Json(body): Json<WifiPowerBody>,
) -> Result<Json<Value>, AppError> {
    require_local(&remote)?;
    state
        .network
        .set_wifi_enabled(body.enabled)
        .await
        .map_err(AppError::BadRequest)?;
    Ok(Json(serde_json::json!({ "success": true })))
}

/// Wi-Fi management is a host-level capability and the server is reachable on
/// the LAN, so anything that changes connectivity must come from the machine
/// itself.
fn require_local(remote: &SocketAddr) -> Result<(), AppError> {
    if remote.ip().is_loopback() {
        return Ok(());
    }
    Err(AppError::Unauthorized(
        "network changes are only allowed from the local machine".into(),
    ))
}

fn snapshot_event(snapshot: &NetworkStatus) -> Event {
    Event::default()
        .event("network")
        .data(serde_json::to_string(snapshot).unwrap_or_default())
}
