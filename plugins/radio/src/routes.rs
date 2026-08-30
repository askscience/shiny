//! Radio plugin REST routes — served through the plugin's `RouteSpec`s.
//! The "now playing" ICY metadata proxy the Radio window polls while a
//! station plays (browsers can't read Shoutcast/Icecast metadata directly).

use std::sync::Arc;

use axum::extract::FromRequestParts;
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::json;

use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::routes::{bridged_route, RouteHandler, UserId};
use shiny_plugin_sdk::services::PluginCtx;

const MAX_METADATA_READ: usize = 64 * 1024 + 4080;

pub fn handle(ctx: &Arc<PluginCtx>, tag: &str) -> Option<RouteHandler> {
    let ctx = ctx.clone();
    match tag {
        "nowplaying" => Some(nowplaying(ctx)),
        _ => None,
    }
}

fn user_id(req: &axum::extract::Request) -> Result<String, AppError> {
    req.extensions()
        .get::<UserId>()
        .map(|u| u.0.clone())
        .ok_or_else(|| AppError::Unauthorized("not authenticated".into()))
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

#[derive(Deserialize)]
struct NowPlayingQuery {
    url: String,
}

#[derive(Serialize)]
struct NowPlayingData {
    title: Option<String>,
    station_name: Option<String>,
}

fn nowplaying(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let _uid = user_id(&req)?;
            let (q, _) = take_query::<NowPlayingQuery>(req).await?;
            let url = q.url.trim().to_string();
            if !(url.starts_with("http://") || url.starts_with("https://")) {
                return Err(AppError::BadRequest("url must be http(s)".into()));
            }

            let data = tokio::time::timeout(
                std::time::Duration::from_secs(8),
                fetch_icy_metadata(&url),
            )
            .await
            .unwrap_or_else(|_| Ok(NowPlayingData { title: None, station_name: None }))?;

            Ok(axum::Json(json!({ "success": true, "data": data })).into_response())
        }
    })
}

async fn fetch_icy_metadata(url: &str) -> Result<NowPlayingData, AppError> {
    let client = reqwest::Client::builder()
        .user_agent("Shiny/0.1 (shiny-radio)")
        .timeout(std::time::Duration::from_secs(6))
        .build()
        .unwrap_or_default();

    let resp = client
        .get(url)
        .header("Icy-MetaData", "1")
        .send()
        .await?;

    let station_name = resp
        .headers()
        .get("icy-name")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let metaint: usize = resp
        .headers()
        .get("icy-metaint")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    let empty = || NowPlayingData { title: None, station_name: station_name.clone() };
    if metaint == 0 {
        return Ok(empty());
    }

    let to_read = (metaint + 4080).min(MAX_METADATA_READ);
    let mut buf: Vec<u8> = Vec::with_capacity(to_read.min(32 * 1024));
    let mut stream = resp.bytes_stream();

    use futures::StreamExt;
    while buf.len() < to_read {
        let Some(chunk) = stream.next().await else { break };
        let Ok(bytes) = chunk else { break };
        buf.extend_from_slice(&bytes);
        if let Some(title) = parse_stream_title(&buf, metaint) {
            return Ok(NowPlayingData { title: Some(title), station_name });
        }
    }
    Ok(empty())
}

fn parse_stream_title(buf: &[u8], metaint: usize) -> Option<String> {
    if buf.len() <= metaint {
        return None;
    }
    let len_byte = *buf.get(metaint)? as usize;
    let len = len_byte * 16;
    if len == 0 || buf.len() < metaint + 1 + len {
        return None;
    }
    let block = &buf[metaint + 1..metaint + 1 + len];
    let text = String::from_utf8_lossy(block);

    let key = "StreamTitle='";
    let start = text.find(key)? + key.len();
    let end = text[start..].find("';")? + start;
    let title = text[start..end].trim().to_string();
    if title.is_empty() || title == "-" {
        None
    } else {
        Some(title)
    }
}
