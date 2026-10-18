//! Minimal local health probes: optional AI, voice and host daemons are not
//! readiness dependencies. Never reveal configuration or credentials here.
use crate::api::{remote, AppState};
use axum::{
    extract::{ConnectInfo, Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use std::{net::SocketAddr, time::Duration};

fn local(request: &Request) -> bool {
    !remote::is_remote(request.headers())
        && request
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .is_some_and(|peer| peer.0.ip().is_loopback())
}

pub async fn live(request: Request) -> Response {
    if !local(&request) {
        return StatusCode::FORBIDDEN.into_response();
    }
    Json(json!({"success": true, "data": {"status": "live"}, "error": null})).into_response()
}

pub async fn ready(State(state): State<AppState>, request: Request) -> Response {
    if !local(&request) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let ready = matches!(
        tokio::time::timeout(
            Duration::from_secs(1),
            sqlx::query_scalar::<_, i64>("SELECT 1").fetch_one(&state.pool)
        )
        .await,
        Ok(Ok(1))
    );
    let status = if ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (
        status,
        Json(json!({"success": ready,
            "data": {"status": if ready {"ready"} else {"unavailable"}},
            "error": if ready {None} else {Some("Database unavailable")}
        })),
    )
        .into_response()
}
