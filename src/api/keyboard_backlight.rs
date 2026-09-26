//! Keyboard backlight API — the Mac's `kbd_backlight` LED.
//!
//! Reads (`GET`) are available to any authenticated client. The write changes
//! the host, and the server binds `0.0.0.0`, so it is accepted only from the
//! local machine (loopback) — the same rule the audio and display panels use.

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, State};
use axum::Json;
use serde_json::{json, Value};

use crate::api::AppState;
use crate::errors::AppError;

/// GET /api/keyboard/backlight
pub async fn status(State(state): State<AppState>, headers: axum::http::HeaderMap) -> Json<Value> {
    let data = crate::api::remote::gate_host_status(
        serde_json::to_value(state.keyboard_backlight.status()).unwrap_or_default(),
        &headers,
    );
    Json(json!({ "success": true, "data": data }))
}

/// POST /api/keyboard/backlight — body `{ "delta": -10 }` or `{ "percent": 40 }`.
pub async fn set(
    State(state): State<AppState>,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, AppError> {
    require_local(&remote)?;

    let data = if let Some(delta) = body.get("delta").and_then(Value::as_i64) {
        state
            .keyboard_backlight
            .adjust(delta.clamp(-100, 100))
            .map_err(AppError::BadRequest)?
    } else if let Some(percent) = body.get("percent").and_then(Value::as_i64) {
        state
            .keyboard_backlight
            .set_percent(percent.clamp(0, 100) as u32)
            .map_err(AppError::BadRequest)?
    } else {
        return Err(AppError::BadRequest(
            "expected a `delta` or `percent` value".into(),
        ));
    };

    Ok(Json(json!({ "success": true, "data": data })))
}

/// The backlight is a host-level capability and the server is reachable on the
/// LAN, so a write must come from the machine itself.
fn require_local(remote: &SocketAddr) -> Result<(), AppError> {
    if remote.ip().is_loopback() {
        return Ok(());
    }
    Err(AppError::Unauthorized(
        "keyboard backlight changes are only allowed from the local machine".into(),
    ))
}
