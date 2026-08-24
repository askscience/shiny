//! Radio helpers: ICY "now playing" metadata proxy.
//!
//! Browsers can't read Shoutcast/Icecast stream metadata from an <audio>
//! element, so the frontend polls this route while a station plays. We open
//! the stream server-side, ask for ICY metadata, and parse the first
//! `StreamTitle='…'` block — most Icecast/Shoutcast stations carry it.
//!
//! Never hangs the UI: hard 8s timeout, ~metaint+255 bytes read at most,
//! and streams without ICY support return an empty title.

use axum::extract::{Extension, Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::api::AppState;
use crate::errors::AppError;
use crate::models::Traveler;

const MAX_METADATA_READ: usize = 64 * 1024 + 4080; // largest sane metaint + max metadata block

#[derive(Deserialize)]
pub struct NowPlayingQuery {
    url: String,
}

#[derive(Serialize)]
pub struct NowPlayingResponse {
    success: bool,
    data: NowPlayingData,
}

#[derive(Serialize)]
pub struct NowPlayingData {
    /// Parsed `StreamTitle` ("Artist — Title" for most music stations).
    title: Option<String>,
    /// `icy-name` header — the stream's own station label.
    station_name: Option<String>,
}

/// GET /api/radio/nowplaying?url=<stream_url>
pub async fn now_playing(
    State(_state): State<AppState>,
    Extension(_traveler): Extension<Traveler>,
    Query(q): Query<NowPlayingQuery>,
) -> Result<Json<NowPlayingResponse>, AppError> {
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

    Ok(Json(NowPlayingResponse { success: true, data }))
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
            return Ok(NowPlayingData {
                title: Some(title),
                station_name,
            });
        }
    }

    Ok(empty())
}

/// Scan the received bytes for the metadata block that sits at `metaint`
/// byte offset (length byte × 16, then `key='value';` pairs).
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
