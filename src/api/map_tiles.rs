//! `GET /api/map/tiles/:z/:x/:y` — OpenStreetMap tiles for the traveler map,
//! fetched by the server and cached on disk.
//!
//! The web UI cannot load `tile.openstreetmap.org` directly: its pages send no
//! `Referer` (`Referrer-Policy: no-referrer`), and OSM's tile usage policy
//! blocks such anonymous requests. The policy asks apps to identify
//! themselves with a User-Agent, keep tiles cached for at least a week and
//! use at most two download connections, which is what this proxy does.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

use axum::extract::Path;
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::Extension;
use crate::errors::AppError;
use tokio::sync::Semaphore;

use crate::models::Traveler;

const TILE_HOST: &str = "https://tile.openstreetmap.org";
const MAX_ZOOM: u32 = 19;
/// Cached tiles are served without asking OSM again for this long.
const FRESH_FOR: Duration = Duration::from_secs(14 * 24 * 3600);

fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(concat!("Shiny/", env!("CARGO_PKG_VERSION"), " (traveler map tiles)"))
            .timeout(Duration::from_secs(20))
            .build()
            .unwrap_or_default()
    })
}

/// OSM allows at most two parallel downloads per client.
fn downloads() -> &'static Semaphore {
    static SLOTS: OnceLock<Semaphore> = OnceLock::new();
    SLOTS.get_or_init(|| Semaphore::new(2))
}

/// `$MAP_TILES_CACHE_DIR`, else `$XDG_CACHE_HOME/shiny/map-tiles`, else
/// `~/.cache/shiny/map-tiles`.
fn cache_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("MAP_TILES_CACHE_DIR").filter(|v| !v.is_empty()) {
        return PathBuf::from(dir);
    }
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(|| PathBuf::from(".cache"))
        .join("shiny")
        .join("map-tiles")
}

/// Parse and bound-check tile coordinates (`y` may carry a `.png` suffix).
fn parse_tile(z: &str, x: &str, y: &str) -> Option<(u32, u32, u32)> {
    let z: u32 = z.parse().ok()?;
    let x: u32 = x.parse().ok()?;
    let y: u32 = y.strip_suffix(".png").unwrap_or(y).parse().ok()?;
    if z > MAX_ZOOM {
        return None;
    }
    let n = 1u32 << z;
    (x < n && y < n).then_some((z, x, y))
}

fn png(bytes: Vec<u8>) -> Response {
    (
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "private, max-age=604800"),
        ],
        bytes,
    )
        .into_response()
}

pub async fn tile(
    Extension(_traveler): Extension<Traveler>,
    Path((z, x, y)): Path<(String, String, String)>,
) -> Result<Response, AppError> {
    let (z, x, y) =
        parse_tile(&z, &x, &y).ok_or_else(|| AppError::BadRequest("invalid tile".into()))?;
    let path = cache_dir().join(z.to_string()).join(x.to_string()).join(format!("{y}.png"));

    let cached = tokio::fs::read(&path).await.ok();
    let fresh = tokio::fs::metadata(&path)
        .await
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|age| age < FRESH_FOR);
    if let (Some(bytes), true) = (&cached, fresh) {
        return Ok(png(bytes.clone()));
    }

    match fetch(z, x, y).await {
        Ok(bytes) => {
            if let Some(parent) = path.parent() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }
            let tmp = path.with_extension("png.part");
            if tokio::fs::write(&tmp, &bytes).await.is_ok() {
                let _ = tokio::fs::rename(&tmp, &path).await;
            }
            Ok(png(bytes))
        }
        // Offline or refused: an old tile beats a hole in the map.
        Err(err) => match cached {
            Some(bytes) => Ok(png(bytes)),
            None => Err(err),
        },
    }
}

async fn fetch(z: u32, x: u32, y: u32) -> Result<Vec<u8>, AppError> {
    let _slot = downloads()
        .acquire()
        .await
        .map_err(|_| AppError::Internal("tile downloads closed".into()))?;
    let resp = client()
        .get(format!("{TILE_HOST}/{z}/{x}/{y}.png"))
        .send()
        .await?;
    if !resp.status().is_success() {
        tracing::warn!("map tile {z}/{x}/{y}: OSM answered {}", resp.status());
        return Err(AppError::NotFound(format!("tile {z}/{x}/{y} unavailable")));
    }
    Ok(resp.bytes().await?.to_vec())
}

#[cfg(test)]
mod tests {
    use super::parse_tile;

    #[test]
    fn parses_and_bounds_tiles() {
        assert_eq!(parse_tile("5", "17", "11.png"), Some((5, 17, 11)));
        assert_eq!(parse_tile("0", "0", "0"), Some((0, 0, 0)));
        assert_eq!(parse_tile("2", "4", "0"), None);
        assert_eq!(parse_tile("20", "0", "0"), None);
        assert_eq!(parse_tile("a", "0", "0"), None);
        assert_eq!(parse_tile("3", "../1", "0"), None);
    }
}
