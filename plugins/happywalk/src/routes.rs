//! The plugin's routes, called by the game's page as the logged-in user.

use std::sync::Arc;

use axum::extract::FromRequest;
use serde_json::json;
use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::routes::{bridged_route, ok, user_id, RouteHandler};
use shiny_plugin_sdk::services::PluginCtx;

pub fn handle(ctx: &Arc<PluginCtx>, tag: &str) -> Option<RouteHandler> {
    let ctx = ctx.clone();
    Some(match tag {
        "ping" => ping(),
        "brain" => crate::brain::route(ctx),
        _ => return None,
    })
}

pub async fn json_body<T: serde::de::DeserializeOwned>(
    req: axum::extract::Request,
) -> Result<T, AppError> {
    let axum::Json(body) = axum::Json::<T>::from_request(req, &())
        .await
        .map_err(|e| AppError::BadRequest(format!("invalid JSON body: {e}")))?;
    Ok(body)
}

/// Proves the framed game reaches the plugin as the logged-in user.
fn ping() -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| async move {
        let uid = user_id(&req)?;
        Ok(ok(json!({
            "plugin": "happywalk",
            "version": env!("CARGO_PKG_VERSION"),
            "user": uid,
        })))
    })
}
