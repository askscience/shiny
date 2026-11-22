//! The HTTP routes the window calls. `plugin.rs` declares each route with a
//! tag; `handle` turns the tag into its handler. The server has already
//! checked who is calling: `user_id(&req)` is that user.

use std::sync::Arc;

use axum::extract::FromRequest;
use serde::Deserialize;
use serde_json::json;
use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::routes::{bridged_route, ok, user_id, RouteHandler};
use shiny_plugin_sdk::services::PluginCtx;

pub fn handle(ctx: &Arc<PluginCtx>, tag: &str) -> Option<RouteHandler> {
    let ctx = ctx.clone();
    Some(match tag {
        "item_list" => item_list(ctx),
        "item_create" => item_create(ctx),
        _ => return None,
    })
}

/// GET /api/@@NAME@@/items
fn item_list(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let user = user_id(&req)?;
            let items = crate::store::list(&ctx, &user, 200)?;
            Ok(ok(json!({ "items": items })))
        }
    })
}

/// POST /api/@@NAME@@/items  { "text": "…" }
fn item_create(ctx: Arc<PluginCtx>) -> RouteHandler {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Create {
        text: String,
    }

    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let user = user_id(&req)?;
            let axum::Json(body) = axum::Json::<Create>::from_request(req, &())
                .await
                .map_err(|e| AppError::BadRequest(format!("invalid JSON body: {e}")))?;
            let item = crate::store::add(&ctx, &user, &body.text)?;
            Ok(ok(json!({ "item": item })))
        }
    })
}
