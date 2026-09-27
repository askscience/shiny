//! Power panel API — reboot, power off or suspend the host, through freedesktop
//! **logind** (see [`crate::services::power`]).
//!
//! Reads (`status`) report which actions the session allows and are available
//! to any authenticated client (a remote client sees `available:false`). The
//! actions change the machine's power state, and the server binds `0.0.0.0`, so
//! they are accepted only from the local machine (loopback).

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, State};
use axum::http::HeaderMap;
use axum::Json;
use serde_json::{json, Value};

use crate::api::AppState;
use crate::errors::AppError;

/// GET /api/power/status
pub async fn status(State(state): State<AppState>, headers: HeaderMap) -> Json<Value> {
    let data = crate::api::remote::gate_host_status(
        serde_json::to_value(state.power.status().await).unwrap_or_default(),
        &headers,
    );
    Json(json!({ "success": true, "data": data }))
}

/// POST /api/power/reboot
pub async fn reboot(
    State(state): State<AppState>,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
) -> Result<Json<Value>, AppError> {
    require_local(&remote)?;
    state.power.reboot().await.map_err(AppError::BadRequest)?;
    Ok(Json(json!({ "success": true })))
}

/// POST /api/power/off
pub async fn power_off(
    State(state): State<AppState>,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
) -> Result<Json<Value>, AppError> {
    require_local(&remote)?;
    state.power.power_off().await.map_err(AppError::BadRequest)?;
    Ok(Json(json!({ "success": true })))
}

/// POST /api/power/suspend
pub async fn suspend(
    State(state): State<AppState>,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
) -> Result<Json<Value>, AppError> {
    require_local(&remote)?;
    state.power.suspend().await.map_err(AppError::BadRequest)?;
    Ok(Json(json!({ "success": true })))
}

/// Powering the machine is a host-level capability and the server is reachable
/// on the LAN, so anything that changes it must come from the machine itself.
fn require_local(remote: &SocketAddr) -> Result<(), AppError> {
    if remote.ip().is_loopback() {
        return Ok(());
    }
    Err(AppError::Unauthorized(
        "power changes are only allowed from the local machine".into(),
    ))
}
