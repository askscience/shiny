//! What is running: versions and fingerprints the page compares to notice an
//! update, and the request that swaps the server for its new binary.
//!
//! Nothing here reveals configuration: a version string and content digests
//! are what a package manager publishes anyway.

use crate::api::{remote, AppState};
use axum::{
    extract::{ConnectInfo, Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use std::net::SocketAddr;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Fingerprint of the server binary that is running, taken once at startup:
/// after a package upgrade the file on disk differs from it, and after the
/// swap the new process reports a new one.
pub fn build_id() -> &'static str {
    static BUILD: OnceLock<String> = OnceLock::new();
    BUILD.get_or_init(|| {
        let exe = std::env::current_exe().unwrap_or_default();
        crate::plugins::digest::file_digest(&exe)
    })
}

/// Fingerprint of the core web tree, cached for a moment: the page asks every
/// few seconds and a stat walk of a thousand files need not run each time.
fn web_digest(web_dir: &str) -> String {
    static CACHE: parking_lot::Mutex<Option<(Instant, String)>> = parking_lot::Mutex::new(None);
    let mut cache = CACHE.lock();
    if let Some((at, digest)) = cache.as_ref() {
        if at.elapsed() < Duration::from_secs(3) {
            return digest.clone();
        }
    }
    let digest = crate::plugins::digest::web_tree_digest(std::path::Path::new(web_dir));
    *cache = Some((Instant::now(), digest.clone()));
    digest
}

/// GET /api/system/version — the running version, the binary's fingerprint,
/// the web tree's fingerprint and one per loaded plugin.
pub async fn version(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(json!({
        "success": true,
        "data": {
            "version": env!("CARGO_PKG_VERSION"),
            "build": build_id(),
            "web": web_digest(&state.config.web_dir),
            "plugins": state.plugins.loaded_digests(),
            "binaryReplaced": crate::application::reexec::binary_replaced(),
        },
        "error": null,
    }))
}

/// POST /api/system/reexec — swap the server for the binary now on disk,
/// keeping the listening socket, once no assistant turn is running.
///
/// Local callers only: a remote client must not restart the machine's server.
pub async fn reexec(State(state): State<AppState>, request: Request) -> Response {
    let local = !remote::is_remote(request.headers())
        && request
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .is_some_and(|peer| peer.0.ip().is_loopback());
    if !local {
        return StatusCode::FORBIDDEN.into_response();
    }
    let scheduled = state.reexec.request();
    Json(json!({
        "success": true,
        "data": { "status": if scheduled { "scheduled" } else { "pending" }, "build": build_id() },
        "error": null,
    }))
    .into_response()
}
