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

/// This server's identity for the session launchers.
///
/// The per-account ports are predictable, so another local account can sit on
/// one before the real server binds it; a launcher that only probed the port
/// would then show that process's page as the login screen or the user's
/// desktop. At startup — before binding — the server writes a random secret to
/// `$XDG_RUNTIME_DIR/shiny-instance-<port>` (mode 0600, in a 0700 directory),
/// and answers a challenge with `sha256(secret:challenge)`. Only the process
/// that wrote the file can answer; the secret itself is never sent, so a
/// listener cannot learn it from the real server and replay it later.
static INSTANCE_SECRET: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Mint the instance secret and write it for the launcher. Called once, before
/// the listener is bound. Without a runtime directory there is no launcher to
/// prove anything to, and nothing is written.
pub fn publish_instance(port: u16) {
    let secret = INSTANCE_SECRET.get_or_init(|| uuid::Uuid::new_v4().to_string());
    let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR").filter(|d| !d.is_empty()) else {
        return;
    };
    let path = std::path::PathBuf::from(dir).join(format!("shiny-instance-{port}"));
    // Replace, never follow: the name is predictable.
    let _ = std::fs::remove_file(&path);
    let written = {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .and_then(|mut file| file.write_all(secret.as_bytes()))
    };
    if let Err(e) = written {
        tracing::warn!("could not write the instance file {}: {e}", path.display());
    }
}

fn instance_proof(secret: &str, challenge: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(format!("{secret}:{challenge}").as_bytes()))
}

#[derive(serde::Deserialize)]
pub struct InstanceQuery {
    c: Option<String>,
}

/// `GET /api/health/instance?c=<challenge>` — proof that this is the server
/// that wrote the instance file (see [`INSTANCE_SECRET`]). Loopback only.
pub async fn instance(
    axum::extract::Query(query): axum::extract::Query<InstanceQuery>,
    request: Request,
) -> Response {
    if !local(&request) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let challenge = query.c.unwrap_or_default();
    let valid = (16..=128).contains(&challenge.len())
        && challenge
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-');
    match (valid, INSTANCE_SECRET.get()) {
        (true, Some(secret)) => instance_proof(secret, &challenge).into_response(),
        _ => StatusCode::BAD_REQUEST.into_response(),
    }
}

#[cfg(test)]
mod instance_tests {
    #[test]
    fn proof_matches_what_the_launcher_computes() {
        // printf '%s:%s' secret 0123456789abcdef | sha256sum
        assert_eq!(
            super::instance_proof("secret", "0123456789abcdef"),
            "c7eddc02a43ab6e0a4f99d6fe26d0750c29444cd27005cc721e8c721f0d9876c"
        );
    }
}
