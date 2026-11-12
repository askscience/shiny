//! Oorbit REST routes. The game window (a framed Godot export) reaches these
//! through its page bridge (`web/index.html`), with the shell's session cookie.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;
use shiny_plugin_sdk::routes::{bridged_route, ok, user_id, RouteHandler};
use shiny_plugin_sdk::services::PluginCtx;

pub fn handle(ctx: &Arc<PluginCtx>, tag: &str) -> Option<RouteHandler> {
    let _ = ctx;
    Some(match tag {
        "ping" => ping(),
        _ => return None,
    })
}

/// Wall-clock milliseconds since the Unix epoch: the game's clock. The
/// simulation runs on server time, never on the client's.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Health check for the window: proves the framed game can reach the
/// plugin's routes as the logged-in user.
fn ping() -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| async move {
        let uid = user_id(&req)?;
        Ok(ok(json!({
            "plugin": "oorbit",
            "version": env!("CARGO_PKG_VERSION"),
            "user": uid,
            "server_time_ms": now_ms(),
        })))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clock_is_after_the_project_started() {
        // 2026-01-01T00:00:00Z
        assert!(now_ms() > 1_767_225_600_000);
    }
}
