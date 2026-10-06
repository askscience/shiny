//! Plugin routes: the window relay and the audio worklet.
//!
//! * `GET  /api/filmcraft/next`    — long-poll target for the window (204 when idle)
//! * `POST /api/filmcraft/result`  — the window's answer to a queued request
//! * `GET  /api/filmcraft/status`  — is a window polling, and how deep is the queue
//! * `GET  /audio-worklet.js`      — see below
//!
//! # Why a worklet route exists
//!
//! The WebAssembly build loads its audio worklet with a **document-relative**
//! URL — `audioWorklet.addModule("audio-worklet.js")` in
//! `apps/filmcraft-web/src/audio.rs`. Standalone that is fine (the document is
//! `/`), but mounted in the Shiny shell the document is `/` too, so the browser
//! would ask the *core* for `/audio-worklet.js`, get `index.html` back from the
//! static fallback, and lose audio.
//!
//! This plugin therefore claims that exact path and serves the worklet from
//! `web/audio-worklet.js`, which `include_str!` embeds in the cdylib: no
//! install-directory lookup, so it works the same for a user plugin and for the
//! read-only system baseline. It is public because `AudioWorklet.addModule`
//! cannot send an `Authorization` header, and the shell page already runs
//! same-origin with the session cookie.

use axum::extract::Request;
use axum::http::header::CONTENT_TYPE;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::{json, Value};

use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::routes::{bridged_route, user_id_from_request, RouteHandler};

use crate::relay;

const MAX_BODY: usize = 1024 * 1024;

/// The worklet FilmCraft's audio clock runs on, embedded at compile time.
const AUDIO_WORKLET: &str = include_str!("../web/audio-worklet.js");

pub fn handle(tag: &str) -> Option<RouteHandler> {
    Some(match tag {
        "relay_next" => relay_next(),
        "relay_result" => relay_result(),
        "relay_status" => relay_status(),
        "audio_worklet" => audio_worklet(),
        _ => return None,
    })
}

fn user_id(req: &Request) -> Result<String, AppError> {
    user_id_from_request(req).ok_or_else(|| AppError::Unauthorized("not authenticated".into()))
}

fn ok(data: Value) -> Response {
    axum::Json(json!({ "success": true, "data": data })).into_response()
}

async fn read_json(req: Request) -> Result<Value, AppError> {
    let bytes = axum::body::to_bytes(req.into_body(), MAX_BODY)
        .await
        .map_err(|e| AppError::BadRequest(format!("invalid body: {e}")))?;
    serde_json::from_slice(&bytes).map_err(|e| AppError::BadRequest(format!("invalid json: {e}")))
}

/* ── GET /api/filmcraft/next ────────────────────────────────────────────── */

/// Hand the window the oldest queued request. 204 means "nothing to do"; the
/// window then polls again after a short delay.
fn relay_next() -> RouteHandler {
    bridged_route(|req: Request| async move {
        let uid = user_id(&req)?;
        Ok(match relay::take(&uid) {
            Some(next) => ok(next),
            None => StatusCode::NO_CONTENT.into_response(),
        })
    })
}

/* ── POST /api/filmcraft/result ─────────────────────────────────────────── */

fn relay_result() -> RouteHandler {
    bridged_route(|req: Request| async move {
        let uid = user_id(&req)?;
        let body = read_json(req).await?;
        let id = body
            .get("id")
            .and_then(Value::as_u64)
            .ok_or_else(|| AppError::BadRequest("result needs a numeric `id`".into()))?;
        let outcome = if body.get("ok").and_then(Value::as_bool) == Some(false) {
            Err(body
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("the window reported an error")
                .to_string())
        } else {
            Ok(body.get("result").cloned().unwrap_or(Value::Null))
        };
        // False means the caller already timed out and dropped the request —
        // not an error, just nothing left to answer.
        let delivered = relay::complete(&uid, id, outcome);
        Ok(ok(json!({ "delivered": delivered })))
    })
}

/* ── GET /api/filmcraft/status ──────────────────────────────────────────── */

fn relay_status() -> RouteHandler {
    bridged_route(|req: Request| async move {
        let uid = user_id(&req)?;
        Ok(ok(json!({
            "window": relay::window_online(&uid),
            "queued": relay::queued(&uid),
        })))
    })
}

/* ── GET /audio-worklet.js ──────────────────────────────────────────────── */

fn audio_worklet() -> RouteHandler {
    bridged_route(|_req: Request| async move {
        Ok(([(CONTENT_TYPE, "text/javascript")], AUDIO_WORKLET).into_response())
    })
}

/// The worklet is a compile-time constant, so a quick self-check catches a
/// stale or truncated `web/audio-worklet.js` at `cargo test` time.
#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};

    use super::*;

    #[test]
    fn worklet_is_embedded_and_is_an_audio_processor() {
        assert!(AUDIO_WORKLET.contains("registerProcessor"), "worklet source looks wrong");
        assert!(AUDIO_WORKLET.contains("AudioWorkletProcessor"), "worklet source looks wrong");
    }

    /// A request as core delivers it to a plugin route: identity in a header,
    /// never an extension (each cdylib has its own axum `TypeId`s).
    fn request_with_user(user: &str) -> Request<Body> {
        Request::builder()
            .uri("/api/filmcraft/next")
            .header("x-shiny-user-id", user)
            .body(Body::empty())
            .expect("a valid request")
    }

    /// Resolve a tag and run the handler, on the plugin's own single-threaded
    /// runtime — the same shape the core uses (`bridged_route` + `rt::bridge`).
    async fn call(tag: &str, req: Request<Body>) -> Response {
        let handler = handle(tag).expect("the tag resolves");
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime");
        // A fresh runtime per call: these tests run concurrently, and a shared
        // one would serialise them (and block on the ones that wait).
        tokio::task::spawn_blocking(move || rt.block_on(handler(req)))
            .await
            .expect("the task ran")
    }

    #[tokio::test]
    async fn next_is_204_until_a_window_polls() {
        let user = "route-user-1";
        // An idle window: the heartbeat keeps it "online" but the queue is empty.
        crate::relay::heartbeat(user);
        let response = call("relay_next", request_with_user(user)).await;
        assert_eq!(response.status(), StatusCode::NO_CONTENT, "an empty queue must be 204");
    }

    #[tokio::test]
    async fn next_hands_over_a_queued_request_and_result_answers_it() {
        let user = "route-user-2";
        crate::relay::heartbeat(user);

        // Queue a request the way a tool does, and drain it as the window does.
        let waiter = tokio::spawn(crate::relay::submit(
            user,
            "engine.execute",
            json!({ "command": "project.inspect" }),
            std::time::Duration::from_secs(5),
        ));
        for _ in 0..200 {
            if crate::relay::queued(user) == 1 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        let response = call("relay_next", request_with_user(user)).await;
        assert_eq!(response.status(), StatusCode::OK);
        let handed = response.into_body();
        let bytes = axum::body::to_bytes(handed, MAX_BODY).await.expect("body");
        let payload: Value = serde_json::from_slice(&bytes).expect("json");
        let id = payload["data"]["id"].as_u64().expect("an id");
        assert_eq!(payload["data"]["method"], "engine.execute");

        // Answer it through the route, as the window does.
        let result = Request::builder()
            .method("POST")
            .uri("/api/filmcraft/result")
            .header("x-shiny-user-id", user)
            .header("content-type", "application/json")
            .body(Body::from(format!(
                r#"{{"id":{id},"ok":true,"result":{{"project":"demo"}}}}"#
            )))
            .expect("a valid request");
        let response = call("relay_result", result).await;
        assert_eq!(response.status(), StatusCode::OK);

        let answer = waiter.await.expect("the task ran").expect("the command succeeded");
        assert_eq!(answer, json!({ "project": "demo" }));
    }

    #[tokio::test]
    async fn an_unanswered_id_is_reported_not_an_error() {
        let user = "route-user-3";
        let result = Request::builder()
            .method("POST")
            .uri("/api/filmcraft/result")
            .header("x-shiny-user-id", user)
            .header("content-type", "application/json")
            .body(Body::from(r#"{"id":9999,"ok":true,"result":null}"#))
            .expect("a valid request");
        let response = call("relay_result", result).await;
        assert_eq!(response.status(), StatusCode::OK, "a late answer is still a 200");
    }

    #[tokio::test]
    async fn the_relay_requires_identity() {
        let anonymous = Request::builder()
            .uri("/api/filmcraft/status")
            .body(Body::empty())
            .expect("a valid request");
        let response = call("relay_status", anonymous).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn the_worklet_route_is_served_without_identity() {
        let response = call("audio_worklet", Request::builder().uri("/audio-worklet.js").body(Body::empty()).expect("a valid request")).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some("text/javascript"),
        );
        let bytes = axum::body::to_bytes(response.into_body(), MAX_BODY).await.expect("body");
        assert_eq!(bytes.as_ref(), AUDIO_WORKLET.as_bytes());
    }

    #[tokio::test]
    async fn an_unknown_tag_resolves_to_nothing() {
        assert!(handle("no_such_tag").is_none(), "an unknown tag must not resolve");
        let _ = Arc::new(());
    }
}