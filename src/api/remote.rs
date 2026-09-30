//! Remote-access API.
//!
//! `status` / `enable` / `rotate` drive the Iroh endpoint, and the
//! `x-shiny-remote` header helpers gate the host-capability endpoints: the Iroh
//! client proxy sets (and overwrites) that header on every forwarded request,
//! so a request carrying it arrived from a remote client even though its TCP
//! peer is `127.0.0.1`.

use axum::extract::{Extension, State};
use axum::http::HeaderMap;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::api::AppState;
use crate::errors::AppError;
use crate::models::Traveler;

/// Header the Iroh client proxy sets on every forwarded request (single source
/// of truth is the plugin SDK, so plugins gate on the same string).
pub const REMOTE_HEADER: &str = shiny_plugin_sdk::routes::REMOTE_HEADER;

/// Headers a reverse proxy (Tailscale Serve/Funnel, Cloudflare, nginx) adds when
/// it forwards a request from off-box. Presence of any is the second remote
/// signal; Tailscale sets all three, so checking more than one is belt-and-braces.
pub const FORWARDED_FOR: &str = "x-forwarded-for";
pub const FORWARDED_PROTO: &str = "x-forwarded-proto";
pub const FORWARDED_HOST: &str = "x-forwarded-host";

/// Did this request arrive from off-box — over Iroh, or through a TLS reverse
/// proxy such as Tailscale Serve/Funnel?
///
/// The Iroh client proxy sets `x-shiny-remote`; Tailscale Serve/Funnel (an L7
/// reverse proxy) adds `x-forwarded-*`. A plain local kiosk request has neither.
/// Either marker means the caller is remote, so host-capability actions are
/// refused. Local callers are already trusted, so spoofing a marker only
/// reduces the caller's own privileges.
pub fn is_remote(headers: &HeaderMap) -> bool {
    headers.contains_key(REMOTE_HEADER)
        || headers.contains_key(FORWARDED_FOR)
        || headers.contains_key(FORWARDED_PROTO)
        || headers.contains_key(FORWARDED_HOST)
}

/// `$XDG_RUNTIME_DIR/shiny-remote.state` — the `on`/`off` flag `shiny-session`
/// reads to pick between the kiosk and the server-mode window.
fn state_path() -> Option<std::path::PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|d| !d.is_empty())
        .map(|d| std::path::PathBuf::from(d).join("shiny-remote.state"))
}

/// Record whether server mode is on. Best-effort; the supervisor retries.
pub fn write_state(on: bool) {
    if let Some(path) = state_path() {
        let _ = std::fs::write(&path, if on { "on" } else { "off" });
    }
}

/// Force `available:false` on a host-capability status payload for a remote
/// client. The frontend already hides a chip when `!available`, so no UI change
/// is needed.
pub fn gate_host_status(data: Value, headers: &HeaderMap) -> Value {
    if !is_remote(headers) {
        return data;
    }
    let mut data = data;
    if let Some(obj) = data.as_object_mut() {
        obj.insert("available".into(), json!(false));
        obj.insert("reason".into(), json!("host control is unavailable to remote clients"));
    }
    data
}

/// GET /api/remote/status — the endpoint state plus whether *this* request is
/// remote.
pub async fn status(State(state): State<AppState>, headers: HeaderMap) -> Json<Value> {
    let status = state.iroh.status().await;
    let tailscale = state.tailscale.status().await;
    let mut value = serde_json::to_value(&status).unwrap_or_default();
    if let Some(obj) = value.as_object_mut() {
        obj.insert("remote_client".into(), json!(is_remote(&headers)));
        obj.insert(
            "tailscale".into(),
            serde_json::to_value(&tailscale).unwrap_or_default(),
        );
    }
    Json(value)
}

#[derive(Deserialize)]
pub struct EnableBody {
    pub enabled: bool,
}

/// POST /api/remote/enable — turn server mode on/off.
///
/// Enabling is **local-only** (a remote client must not expose the server);
/// disabling is allowed from an authenticated remote client so they can end the
/// session.
pub async fn enable(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
    headers: HeaderMap,
    Json(body): Json<EnableBody>,
) -> Result<Json<Value>, AppError> {
    if body.enabled && is_remote(&headers) {
        return Err(AppError::Unauthorized(
            "server mode can only be enabled from the local machine".into(),
        ));
    }

    if body.enabled {
        let status = state.iroh.start(state.config.server_port).await?;
        set_preference(&state, &traveler.id, "remote.enabled", "true").await;
        write_state(true);
        Ok(Json(serde_json::to_value(&status).unwrap_or_default()))
    } else {
        set_preference(&state, &traveler.id, "remote.enabled", "false").await;
        write_state(false);
        // Stop *after* the response is on the wire: closing the endpoint tears
        // down the very connection a remote "stop" request arrived on, so a
        // synchronous stop would lose the reply.
        let state = state.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            if let Err(e) = state.iroh.stop().await {
                tracing::warn!("iroh stop failed: {e:?}");
            }
        });
        Ok(Json(json!({ "enabled": false })))
    }
}

/// POST /api/remote/rotate — stop and start with a fresh identity, revoking the
/// previous ticket. Local-only.
pub async fn rotate(
    State(state): State<AppState>,
    Extension(_traveler): Extension<Traveler>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    if is_remote(&headers) {
        return Err(AppError::Unauthorized(
            "key rotation is only allowed from the local machine".into(),
        ));
    }
    let status = state.iroh.rotate(state.config.server_port).await?;
    Ok(Json(serde_json::to_value(&status).unwrap_or_default()))
}

/// POST /api/remote/pair — open a short window in which the next connecting
/// device is added to the allowlist. Local-only.
pub async fn pair(
    State(state): State<AppState>,
    Extension(_traveler): Extension<Traveler>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    if is_remote(&headers) {
        return Err(AppError::Unauthorized(
            "pairing is only allowed from the local machine".into(),
        ));
    }
    let seconds = state.iroh.begin_pairing();
    Ok(Json(json!({ "pairing": true, "seconds": seconds })))
}

/// POST /api/remote/unpair — forget every paired device, back to ticket-only
/// access. Local-only.
pub async fn unpair(
    State(state): State<AppState>,
    Extension(_traveler): Extension<Traveler>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    if is_remote(&headers) {
        return Err(AppError::Unauthorized(
            "this action is only allowed from the local machine".into(),
        ));
    }
    let removed = state.iroh.unpair_all();
    Ok(Json(json!({ "removed": removed })))
}

/// POST /api/remote/tailscale/enable — expose the app through Tailscale Funnel
/// (a public `*.ts.net` HTTPS URL). Enabling is **local-only**: a remote client
/// must not be able to re-expose the server.
pub async fn tailscale_enable(
    State(state): State<AppState>,
    Extension(_traveler): Extension<Traveler>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    if is_remote(&headers) {
        return Err(AppError::Unauthorized(
            "server mode can only be enabled from the local machine".into(),
        ));
    }
    let status = state
        .tailscale
        .enable()
        .await
        .map_err(AppError::BadRequest)?;
    Ok(Json(serde_json::to_value(&status).unwrap_or_default()))
}

/// POST /api/remote/tailscale/disable — stop the Funnel. Allowed from a remote
/// client too, so a remote user can close the exposure they are connected to.
pub async fn tailscale_disable(
    State(state): State<AppState>,
    Extension(_traveler): Extension<Traveler>,
) -> Result<Json<Value>, AppError> {
    let status = state
        .tailscale
        .disable()
        .await
        .map_err(AppError::BadRequest)?;
    Ok(Json(serde_json::to_value(&status).unwrap_or_default()))
}

/// Build the shared QR SVG (dark text on a light field).
fn qr_svg(text: &str) -> Result<String, AppError> {
    let code =
        qrcode::QrCode::new(text.as_bytes()).map_err(|e| AppError::Internal(format!("qr encode: {e}")))?;
    Ok(code
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(260, 260)
        .quiet_zone(true)
        .dark_color(qrcode::render::svg::Color("#ffffff"))
        .light_color(qrcode::render::svg::Color("#000000"))
        .build())
}

fn svg_response(svg: String) -> Result<axum::response::Response, AppError> {
    Ok(axum::response::Response::builder()
        .header(axum::http::header::CONTENT_TYPE, "image/svg+xml")
        .header(axum::http::header::CACHE_CONTROL, "no-store")
        .body(axum::body::Body::from(svg))
        .map_err(|e| AppError::Internal(format!("response: {e}")))?)
}

/// GET /api/remote/qr — the current Iroh link as an SVG QR code.
pub async fn qr(State(state): State<AppState>) -> Result<axum::response::Response, AppError> {
    let status = state.iroh.status().await;
    let ticket = status
        .ticket
        .ok_or_else(|| AppError::BadRequest("server mode is not enabled".into()))?;
    svg_response(qr_svg(&ticket)?)
}

/// GET /api/remote/tailscale/qr — the Funnel URL as an SVG QR code.
pub async fn tailscale_qr(
    State(state): State<AppState>,
) -> Result<axum::response::Response, AppError> {
    let status = state.tailscale.status().await;
    let url = status
        .url
        .ok_or_else(|| AppError::BadRequest("Tailscale Funnel is not on".into()))?;
    svg_response(qr_svg(&url)?)
}

async fn set_preference(state: &AppState, user_id: &str, key: &str, value: &str) {
    let res = sqlx::query(
        "INSERT INTO user_preferences (user_id, key, value, updated_at) \
         VALUES (?1, ?2, ?3, datetime('now')) \
         ON CONFLICT(user_id, key) DO UPDATE SET value = excluded.value, updated_at = datetime('now')",
    )
    .bind(user_id)
    .bind(key)
    .bind(value)
    .execute(&state.pool)
    .await;
    if let Err(e) = res {
        tracing::warn!("failed to persist preference {key}: {e}");
    }
}
