//! Bluetooth panel API — the host's adapter and devices for the top-bar chip
//! and the Bluetooth menu. Backed by [`crate::services::bluetooth`], which
//! caches snapshots from BlueZ over D-Bus and broadcasts them; this module
//! never talks to D-Bus itself.
//!
//! Reads (`status`, `events`) are available to any authenticated client.
//! Mutations change what the machine is paired with, and the server binds
//! `0.0.0.0`, so they are accepted only from the local machine (loopback).

use std::convert::Infallible;
use std::net::SocketAddr;

use axum::extract::{ConnectInfo, State};
use axum::response::sse::{Event, Sse};
use axum::Json;
use futures::Stream;
use serde::Deserialize;
use serde_json::Value;

use crate::api::AppState;
use crate::errors::AppError;

/// GET /api/bluetooth/status
pub async fn status(State(state): State<AppState>, headers: axum::http::HeaderMap) -> Json<Value> {
    let data = crate::api::remote::gate_host_status(
        serde_json::to_value(state.bluetooth.status()).unwrap_or_default(),
        &headers,
    );
    Json(serde_json::json!({ "success": true, "data": data }))
}

/// GET /api/bluetooth/events
pub async fn events(
    State(state): State<AppState>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let receiver = state.bluetooth.subscribe();
    let initial = state.bluetooth.status();
    Ok(crate::api::sse::snapshot_stream("bluetooth", receiver, initial))
}

#[derive(Deserialize)]
pub struct PowerBody {
    enabled: bool,
}

/// POST /api/bluetooth/power
pub async fn power(
    State(state): State<AppState>,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    Json(body): Json<PowerBody>,
) -> Result<Json<Value>, AppError> {
    require_local(&remote)?;
    state
        .bluetooth
        .set_power(body.enabled)
        .await
        .map_err(AppError::BadRequest)?;
    Ok(Json(serde_json::json!({ "success": true })))
}

/// POST /api/bluetooth/scan
pub async fn scan(
    State(state): State<AppState>,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
) -> Result<Json<Value>, AppError> {
    require_local(&remote)?;
    state.bluetooth.scan().await.map_err(AppError::BadRequest)?;
    Ok(Json(serde_json::json!({ "success": true })))
}

#[derive(Deserialize)]
pub struct DeviceBody {
    id: String,
}

/// POST /api/bluetooth/pair
pub async fn pair(
    State(state): State<AppState>,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    Json(body): Json<DeviceBody>,
) -> Result<Json<Value>, AppError> {
    require_local(&remote)?;
    state
        .bluetooth
        .pair(&body.id)
        .await
        .map_err(AppError::BadRequest)?;
    Ok(Json(serde_json::json!({ "success": true })))
}

/// POST /api/bluetooth/connect
pub async fn connect(
    State(state): State<AppState>,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    Json(body): Json<DeviceBody>,
) -> Result<Json<Value>, AppError> {
    require_local(&remote)?;
    state
        .bluetooth
        .connect(&body.id)
        .await
        .map_err(AppError::BadRequest)?;
    Ok(Json(serde_json::json!({ "success": true })))
}

/// POST /api/bluetooth/disconnect
pub async fn disconnect(
    State(state): State<AppState>,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    Json(body): Json<DeviceBody>,
) -> Result<Json<Value>, AppError> {
    require_local(&remote)?;
    state
        .bluetooth
        .disconnect(&body.id)
        .await
        .map_err(AppError::BadRequest)?;
    Ok(Json(serde_json::json!({ "success": true })))
}

/// POST /api/bluetooth/forget
pub async fn forget(
    State(state): State<AppState>,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    Json(body): Json<DeviceBody>,
) -> Result<Json<Value>, AppError> {
    require_local(&remote)?;
    state
        .bluetooth
        .forget(&body.id)
        .await
        .map_err(AppError::BadRequest)?;
    Ok(Json(serde_json::json!({ "success": true })))
}

#[derive(Deserialize)]
pub struct TrustBody {
    id: String,
    trusted: bool,
}

/// POST /api/bluetooth/trust
pub async fn trust(
    State(state): State<AppState>,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    Json(body): Json<TrustBody>,
) -> Result<Json<Value>, AppError> {
    require_local(&remote)?;
    state
        .bluetooth
        .set_trusted(&body.id, body.trusted)
        .await
        .map_err(AppError::BadRequest)?;
    Ok(Json(serde_json::json!({ "success": true })))
}

/// Bluetooth management is a host-level capability and the server is reachable
/// on the LAN, so anything that changes what is paired must come from the
/// machine itself.
fn require_local(remote: &SocketAddr) -> Result<(), AppError> {
    if remote.ip().is_loopback() {
        return Ok(());
    }
    Err(AppError::Unauthorized(
        "Bluetooth changes are only allowed from the local machine".into(),
    ))
}

