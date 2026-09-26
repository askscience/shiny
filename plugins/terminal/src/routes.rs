//! Terminal REST routes.
//!
//! Session lifecycle plus an SSE output stream. Keystrokes are posted as
//! plain JSON (`{"session": "...", "data": "ls\n"}`); output comes back as
//! base64 chunks on `GET /api/terminal/stream?session=...` so arbitrary bytes
//! survive the text-only SSE framing.

use std::convert::Infallible;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{FromRequestParts, Request};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use bytes::Bytes;
use futures::StreamExt;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::json;

use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::routes::{bridged_route, is_remote_request, user_id_from_request, RouteHandler};
use shiny_plugin_sdk::services::PluginCtx;

use crate::pty::{self, Event, Session};

/// Largest request body we accept (keystroke batches are tiny).
const BODY_LIMIT: usize = 256 * 1024;

pub fn handle(ctx: &Arc<PluginCtx>, tag: &str) -> Option<RouteHandler> {
    let ctx = ctx.clone();
    Some(match tag {
        "sessions" => create_session(ctx),
        "stream" => stream(),
        "input" => input(),
        "resize" => resize(),
        "close" => close(),
        _ => return None,
    })
}

/// The Terminal runs a shell on the server, so it is the highest-impact surface
/// to expose. Deny it to remote (Iroh) clients unless the user opted in with
/// `remote.allow_terminal`. Local sessions are always allowed.
fn remote_allowed(ctx: &PluginCtx, uid: &str, req: &Request) -> bool {
    if !is_remote_request(req) {
        return true;
    }
    let rows = ctx.db().query(
        "SELECT value FROM user_preferences WHERE user_id = ?1 AND key = 'remote.allow_terminal'",
        &[shiny_plugin_sdk::db::Value::text(uid)],
    );
    matches!(
        rows.ok().and_then(|r| r.into_iter().next()).and_then(|row| row.into_iter().next()),
        Some(shiny_plugin_sdk::db::Value::Text(v)) if v == "true"
    )
}

fn user_id(req: &Request) -> Result<String, AppError> {
    user_id_from_request(req).ok_or_else(|| AppError::Unauthorized("not authenticated".into()))
}

/// Look up a session and check it belongs to the caller.
fn owned(uid: &str, id: &str) -> Result<Arc<Session>, AppError> {
    let session =
        pty::get(id).ok_or_else(|| AppError::NotFound("terminal session not found".into()))?;
    if session.user_id != uid {
        return Err(AppError::Unauthorized("not your terminal session".into()));
    }
    Ok(session)
}

fn ok(data: serde_json::Value) -> Response {
    axum::Json(json!({ "success": true, "data": data })).into_response()
}

async fn json_body<T: DeserializeOwned>(req: Request) -> Result<T, AppError> {
    let bytes = axum::body::to_bytes(req.into_body(), BODY_LIMIT)
        .await
        .map_err(|e| AppError::BadRequest(format!("invalid body: {e}")))?;
    serde_json::from_slice(&bytes).map_err(|e| AppError::BadRequest(format!("invalid JSON body: {e}")))
}

async fn take_query<T: DeserializeOwned + Send + 'static>(req: Request) -> Result<T, AppError> {
    let (mut parts, _body) = req.into_parts();
    let query = axum::extract::Query::<T>::from_request_parts(&mut parts, &())
        .await
        .map_err(|e| AppError::BadRequest(format!("invalid query: {e}")))?;
    Ok(query.0)
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct CreateBody {
    cols: Option<u16>,
    rows: Option<u16>,
}

/// `POST /api/terminal/sessions` — spawn a shell.
fn create_session(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            if !remote_allowed(&ctx, &uid, &req) {
                return Err(AppError::Unauthorized(
                    "the Terminal is disabled for remote clients — enable it in Remote settings".into(),
                ));
            }
            let body: CreateBody = json_body(req).await?;
            let session = pty::create(&uid, body.cols.unwrap_or(80), body.rows.unwrap_or(24))?;
            Ok(ok(session.meta()))
        }
    })
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct StreamQuery {
    session: Option<String>,
}

/// `GET /api/terminal/stream?session=...` — SSE: `ready`, `out`, `exit`, `ping`.
fn stream() -> RouteHandler {
    bridged_route(move |req: Request| async move {
        let uid = user_id(&req)?;
        let query: StreamQuery = take_query(req).await?;
        let id = query
            .session
            .ok_or_else(|| AppError::BadRequest("session required".into()))?;
        let session = owned(&uid, &id)?;

        let (snapshot, live, alive) = session.subscribe();

        let mut head = vec![session.ready_event().to_sse()];
        if !snapshot.is_empty() {
            let encoded = base64::engine::general_purpose::STANDARD.encode(&snapshot);
            head.push(Event::Out(encoded).to_sse());
        }
        if !alive {
            head.push(Event::Exit { code: None }.to_sse());
        }

        let head = futures::stream::iter(
            head.into_iter().map(|frame| Ok::<Bytes, Infallible>(Bytes::from(frame))),
        );
        let live = live.map(|event| Ok::<Bytes, Infallible>(Bytes::from(event.to_sse())));
        let stream = head.chain(live);

        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/event-stream")
            .header(header::CACHE_CONTROL, "no-cache, no-transform")
            .header(header::CONNECTION, "keep-alive")
            .header("x-accel-buffering", "no")
            .body(Body::from_stream(stream))
            .map_err(|e| AppError::Internal(format!("sse response failed: {e}")))
    })
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct InputBody {
    session: Option<String>,
    data: Option<String>,
}

/// `POST /api/terminal/input` — write keystrokes to the shell.
fn input() -> RouteHandler {
    bridged_route(move |req: Request| async move {
        let uid = user_id(&req)?;
        let body: InputBody = json_body(req).await?;
        let id = body
            .session
            .ok_or_else(|| AppError::BadRequest("session required".into()))?;
        let data = body.data.unwrap_or_default();
        let session = owned(&uid, &id)?;
        session.write_input(data.as_bytes())?;
        Ok(ok(json!({ "written": data.len() })))
    })
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ResizeBody {
    session: Option<String>,
    cols: Option<u16>,
    rows: Option<u16>,
}

/// `POST /api/terminal/resize` — keep the PTY window size in step with xterm.
fn resize() -> RouteHandler {
    bridged_route(move |req: Request| async move {
        let uid = user_id(&req)?;
        let body: ResizeBody = json_body(req).await?;
        let id = body
            .session
            .ok_or_else(|| AppError::BadRequest("session required".into()))?;
        let session = owned(&uid, &id)?;
        session.resize(body.cols.unwrap_or(80), body.rows.unwrap_or(24))?;
        Ok(ok(json!({ "cols": body.cols, "rows": body.rows })))
    })
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct CloseBody {
    session: Option<String>,
}

/// `POST /api/terminal/close` — kill the shell and forget the session.
fn close() -> RouteHandler {
    bridged_route(move |req: Request| async move {
        let uid = user_id(&req)?;
        let body: CloseBody = json_body(req).await?;
        let id = body
            .session
            .ok_or_else(|| AppError::BadRequest("session required".into()))?;
        let session = owned(&uid, &id)?;
        session.close()?;
        Ok(ok(json!({ "closed": true })))
    })
}
