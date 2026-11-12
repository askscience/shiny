//! Oorbit REST routes. The game window (a framed Godot export) reaches these
//! through its page bridge (`web/index.html`), with the shell's session cookie.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::FromRequest;
use serde::Deserialize;
use serde_json::json;

use oorbit_sim::{advance, apply, Action, Planet};
use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::routes::{bridged_route, ok, user_id, RouteHandler};
use shiny_plugin_sdk::services::PluginCtx;

use crate::store::{self, catalog};
use oorbit_sim::view::planet_view;

pub fn handle(ctx: &Arc<PluginCtx>, tag: &str) -> Option<RouteHandler> {
    let ctx = ctx.clone();
    Some(match tag {
        "ping" => ping(),
        "catalog" => catalog_route(),
        "state" => state(ctx),
        "found" => found(ctx),
        "action" => action(ctx),
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

async fn json_body<T: serde::de::DeserializeOwned>(
    req: axum::extract::Request,
) -> Result<T, AppError> {
    let axum::Json(body) = axum::Json::<T>::from_request(req, &())
        .await
        .map_err(|e| AppError::BadRequest(format!("invalid JSON body: {e}")))?;
    Ok(body)
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

/// The game data (eras, discoveries, buildings, events, climates), for the
/// window's labels and previews. It changes only with the plugin.
fn catalog_route() -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| async move {
        user_id(&req)?;
        Ok(ok(serde_json::to_value(catalog()).unwrap_or_default()))
    })
}

/// The user's planet as of now (`planet: null` before it is founded).
fn state(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let now = now_ms();
            let _guard = store::lock();
            match store::load(&ctx, &uid)? {
                None => Ok(ok(json!({ "now_ms": now, "planet": null }))),
                Some(mut planet) => {
                    advance(&mut planet, catalog(), now);
                    store::save(&ctx, &uid, &planet)?;
                    Ok(ok(planet_view(&planet, catalog(), now)))
                }
            }
        }
    })
}

/// Found the user's planet in a climate. Refused when one exists.
fn found(ctx: Arc<PluginCtx>) -> RouteHandler {
    #[derive(Deserialize)]
    struct Body {
        climate: String,
    }
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let body: Body = json_body(req).await?;
            let now = now_ms();
            let _guard = store::lock();
            if store::load(&ctx, &uid)?.is_some() {
                return Err(AppError::BadRequest(
                    "this planet is already founded".into(),
                ));
            }
            let planet = Planet::new(store::new_seed(&uid, now), &body.climate, now, catalog())
                .map_err(AppError::BadRequest)?;
            store::save(&ctx, &uid, &planet)?;
            Ok(ok(planet_view(&planet, catalog(), now)))
        }
    })
}

/// One player action (`oorbit_sim::Action` as JSON).
fn action(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let action: Action = json_body(req).await?;
            let now = now_ms();
            let _guard = store::lock();
            let mut planet = store::load(&ctx, &uid)?
                .ok_or_else(|| AppError::BadRequest("found a planet first".into()))?;
            let result = apply(&mut planet, catalog(), now, &action);
            // Save even when the action is refused: the planet advanced.
            store::save(&ctx, &uid, &planet)?;
            result.map_err(|e| AppError::BadRequest(e.to_string()))?;
            Ok(ok(planet_view(&planet, catalog(), now)))
        }
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
