//! YouTube plugin REST routes — served through the plugin's `RouteSpec`s.

use std::sync::Arc;

use axum::extract::FromRequestParts;
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};

use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::routes::{bridged_route, RouteHandler, user_id_from_request};
use shiny_plugin_sdk::services::PluginCtx;

pub fn handle(ctx: &Arc<PluginCtx>, tag: &str) -> Option<RouteHandler> {
    let ctx = ctx.clone();
    match tag {
        "yt_search" => Some(yt_search(ctx)),
        _ => None,
    }
}

fn user_id(req: &axum::extract::Request) -> Result<String, AppError> {
    user_id_from_request(req)
        .ok_or_else(|| AppError::Unauthorized("not authenticated".into()))
}

fn ok(data: Value) -> Response {
    axum::Json(json!({ "success": true, "data": data })).into_response()
}

async fn take_query<T: DeserializeOwned + Send + 'static>(
    req: axum::extract::Request,
) -> Result<(T, axum::extract::Request), AppError> {
    let (mut parts, body) = req.into_parts();
    let query = axum::extract::Query::<T>::from_request_parts(&mut parts, &())
        .await
        .map_err(|e| AppError::BadRequest(format!("invalid query: {e}")))?;
    Ok((query.0, axum::extract::Request::from_parts(parts, body)))
}

fn yt_search(ctx: Arc<PluginCtx>) -> RouteHandler {
    #[derive(Deserialize)]
    struct SearchQuery {
        q: String,
    }

    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let _uid = user_id(&req)?;
            let (q, _) = take_query::<SearchQuery>(req).await?;
            let query = q.q.trim().to_string();
            if query.is_empty() {
                return Err(AppError::BadRequest("q required".into()));
            }
            let results = crate::youtube_client::search(&query, 12).await?;
            Ok(ok(json!(results)))
        }
    })
}
