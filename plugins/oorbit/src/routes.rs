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
        "layout" => layout_route(),
        "state" => state(ctx),
        "found" => found(ctx),
        "action" => action(ctx),
        "reset" => reset(ctx),
        "dev_skip" => dev_skip(ctx),
        "presence" => presence(ctx),
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

/// The planet's cells (centres, shared corners, rings and neighbours), the
/// same layout the simulation uses. It changes only with the plugin.
fn layout_route() -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| async move {
        user_id(&req)?;
        static LAYOUT: std::sync::OnceLock<serde_json::Value> = std::sync::OnceLock::new();
        let value = LAYOUT.get_or_init(|| {
            oorbit_sim::cells::layout_json(catalog().tuning.tiles)
                .and_then(|text| serde_json::from_str(text).ok())
                .unwrap_or_default()
        });
        Ok(ok(value.clone()))
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
                    let now = planet.time_at(now);
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
            let mut planet = Planet::new(store::new_seed(&uid, now), &body.climate, now, catalog())
                .map_err(AppError::BadRequest)?;
            planet.clock = oorbit_sim::clock::Clock::starting(now);
            planet.present(now);
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
            // Acting is playing: the planet runs at play speed a while.
            planet.time_at(now);
            planet.present(now);
            let now = planet.time_at(now);
            let result = apply(&mut planet, catalog(), now, &action);
            // Save even when the action is refused: the planet advanced.
            store::save(&ctx, &uid, &planet)?;
            result.map_err(|e| AppError::BadRequest(e.to_string()))?;
            Ok(ok(planet_view(&planet, catalog(), now)))
        }
    })
}

/// The window reports that someone is playing (it sends this every few
/// seconds while the player is active and the window is on screen): the
/// planet runs at play speed for a little while more.
fn presence(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let wall = now_ms();
            let _guard = store::lock();
            let mut planet = store::load(&ctx, &uid)?
                .ok_or_else(|| AppError::BadRequest("found a planet first".into()))?;
            planet.time_at(wall);
            planet.present(wall);
            let now = planet.time_at(wall);
            advance(&mut planet, catalog(), now);
            store::save(&ctx, &uid, &planet)?;
            Ok(ok(planet_view(&planet, catalog(), now)))
        }
    })
}

/// Testing only (temporary): move this planet's time forward by `hours` of
/// planet time, as if that much had passed.
fn dev_skip(ctx: Arc<PluginCtx>) -> RouteHandler {
    #[derive(Deserialize)]
    struct Body {
        hours: f64,
    }
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let body: Body = json_body(req).await?;
            if !(0.0..=24.0 * 30.0).contains(&body.hours) {
                return Err(AppError::BadRequest("hours must be 0 to 720".into()));
            }
            let _guard = store::lock();
            let mut planet = store::load(&ctx, &uid)?
                .ok_or_else(|| AppError::BadRequest("found a planet first".into()))?;
            planet.time_at(now_ms());
            planet.clock.skip((body.hours * 3_600_000.0) as i64);
            let now = planet.time_at(now_ms());
            advance(&mut planet, catalog(), now);
            store::save(&ctx, &uid, &planet)?;
            Ok(ok(planet_view(&planet, catalog(), now)))
        }
    })
}

/// Give up the user's planet, to found a new one.
fn reset(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let _guard = store::lock();
            store::delete(&ctx, &uid)?;
            Ok(ok(json!({ "now_ms": now_ms(), "planet": null })))
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
