//! Files plugin REST routes — served through the plugin's `RouteSpec`s.
//!
//! Every handler is wrapped with `bridged_route` so its `tokio::fs` work runs
//! on the plugin-owned runtime. All paths are home-relative and sandboxed by
//! `fs_util::resolve`; see that module for the escape checks.

use std::path::Path;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{FromRequest, FromRequestParts, Request};
use axum::http::header::{
    ACCEPT_RANGES, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, RANGE,
};
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value as Json};

use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::routes::{bridged_route, user_id_from_request, RouteHandler};
use shiny_plugin_sdk::services::PluginCtx;

use crate::fs_util;
use crate::preview;

/// Ceiling for a single file read into memory by `read`/`raw`/`download`.
const MAX_INLINE: usize = 256 * 1024 * 1024;
/// Largest byte window served per ranged request. Media players fetch the next
/// window themselves, so this bounds memory for huge videos.
const RANGE_CHUNK: usize = 4 * 1024 * 1024;
/// Ceiling for an uploaded file body.
const MAX_UPLOAD: usize = 128 * 1024 * 1024;

pub fn handle(ctx: &Arc<PluginCtx>, tag: &str) -> Option<RouteHandler> {
    let ctx = ctx.clone();
    Some(match tag {
        "files_list" => list(ctx),
        "files_home" => home(ctx),
        "files_read" => read(ctx),
        "files_text" => text(ctx),
        "files_render" => render(ctx),
        "files_raw" => raw(ctx),
        "files_thumb" => thumb(ctx),
        "files_video_info" => video_info(ctx),
        "files_video_frame" => video_frame(ctx),
        "files_download" => download(ctx),
        "files_upload" => upload(ctx),
        "files_write" => write(ctx),
        "files_mkdir" => mkdir(ctx),
        "files_rename" => rename(ctx),
        "files_delete" => delete(ctx),
        "files_restore" => restore(ctx),
        "files_empty_trash" => empty_trash(ctx),
        "files_search" => search(ctx),
        _ => return None,
    })
}

/* ── helpers ────────────────────────────────────────────────── */

fn user_id(req: &Request) -> Result<String, AppError> {
    user_id_from_request(req)
        .ok_or_else(|| AppError::Unauthorized("not authenticated".into()))
}

fn ok(data: Json) -> Response {
    axum::Json(json!({ "success": true, "data": data })).into_response()
}

async fn take_query<T: DeserializeOwned + Send + 'static>(
    req: Request,
) -> Result<(T, Request), AppError> {
    let (mut parts, body) = req.into_parts();
    let query = axum::extract::Query::<T>::from_request_parts(&mut parts, &())
        .await
        .map_err(|e| AppError::BadRequest(format!("invalid query: {e}")))?;
    Ok((query.0, Request::from_parts(parts, body)))
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct PathQuery {
    path: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ReadQuery {
    path: Option<String>,
    max: Option<usize>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ThumbQuery {
    path: Option<String>,
    size: Option<u32>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct FrameQuery {
    path: Option<String>,
    /// Timestamp in seconds.
    t: Option<f64>,
    size: Option<u32>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct UploadQuery {
    path: Option<String>,
    name: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct SearchQuery {
    q: Option<String>,
    path: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct WriteBody {
    path: Option<String>,
    content: Option<String>,
    append: Option<bool>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RenameBody {
    from: Option<String>,
    to: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct PathBody {
    path: Option<String>,
    name: Option<String>,
}

/// Ensure the path exists and is a directory.
async fn require_dir(path: &Path) -> Result<(), AppError> {
    let meta = tokio::fs::metadata(path)
        .await
        .map_err(|_| AppError::NotFound("folder not found".into()))?;
    if !meta.is_dir() {
        return Err(AppError::BadRequest("not a folder".into()));
    }
    Ok(())
}

/// Reject a leaf name containing separators or NUL.
fn clean_name(name: &str) -> Result<String, AppError> {
    let name = name.trim();
    if name.is_empty() || name == "." || name == ".." {
        return Err(AppError::BadRequest("invalid name".into()));
    }
    if name.contains('/') || name.contains('\\') || name.contains('\0') {
        return Err(AppError::BadRequest("name may not contain separators".into()));
    }
    Ok(name.to_string())
}

fn sanitize_filename(name: &str) -> String {
    name.chars()
        .filter(|c| !matches!(c, '"' | '\\' | '\r' | '\n' | '\0'))
        .collect()
}

fn parse_range(header: &str, len: usize) -> Option<(usize, usize)> {
    let spec = header.strip_prefix("bytes=")?;
    let (start_s, end_s) = spec.split_once('-')?;
    let (start, end) = if start_s.is_empty() {
        // suffix range: -N
        let n: usize = end_s.parse().ok()?;
        (len.saturating_sub(n), len.saturating_sub(1))
    } else {
        let start: usize = start_s.parse().ok()?;
        let end: usize = if end_s.is_empty() {
            len.saturating_sub(1)
        } else {
            end_s.parse().ok()?
        };
        (start, end.min(len.saturating_sub(1)))
    };
    if start > end || start >= len {
        return None;
    }
    Some((start, end))
}

/* ── handlers ───────────────────────────────────────────────── */

fn list(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let _ = &ctx;
        async move {
            let uid = user_id(&req)?;
            let home = fs_util::ensure_home(&uid).await?;
            let (q, _req) = take_query::<PathQuery>(req).await?;
            let rel = q.path.unwrap_or_default();
            let dir = fs_util::resolve(&home, &rel).await?;
            require_dir(&dir).await?;
            let entries = fs_util::list_dir(&home, &dir).await?;
            Ok(ok(json!({
                "path": fs_util::rel_display(&home, &dir),
                "home": fs_util::home_display(&uid),
                "entries": entries,
            })))
        }
    })
}

fn home(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let _ = &ctx;
        async move {
            let uid = user_id(&req)?;
            let home = fs_util::ensure_home(&uid).await?;
            let entries = fs_util::list_dir(&home, &home).await?;
            Ok(ok(json!({
                "home": fs_util::home_display(&uid),
                "classic": fs_util::CLASSIC_DIRS,
                "path": "",
                "entries": entries,
            })))
        }
    })
}

fn read(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let _ = &ctx;
        async move {
            let uid = user_id(&req)?;
            let home = fs_util::ensure_home(&uid).await?;
            let (q, _req) = take_query::<ReadQuery>(req).await?;
            let rel = q.path.unwrap_or_default();
            let path = fs_util::resolve(&home, &rel).await?;
            let meta = tokio::fs::metadata(&path)
                .await
                .map_err(|_| AppError::NotFound("file not found".into()))?;
            if meta.is_dir() {
                return Err(AppError::BadRequest("cannot read a folder".into()));
            }
            let limit = q.max.unwrap_or(1024 * 1024).clamp(1024, 8 * 1024 * 1024);
            let bytes = tokio::fs::read(&path).await?;
            let size = bytes.len();
            if bytes.iter().take(8192).any(|&b| b == 0) {
                return Err(AppError::BadRequest("binary file — download instead".into()));
            }
            let truncated = size > limit;
            let slice = &bytes[..size.min(limit)];
            let content = String::from_utf8_lossy(slice).into_owned();
            Ok(ok(json!({
                "path": fs_util::rel_display(&home, &path),
                "content": content,
                "size": size,
                "truncated": truncated,
            })))
        }
    })
}

fn text(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let _ = &ctx;
        async move {
            let uid = user_id(&req)?;
            let home = fs_util::ensure_home(&uid).await?;
            let (q, _req) = take_query::<PathQuery>(req).await?;
            let rel = q.path.unwrap_or_default();
            let path = fs_util::resolve(&home, &rel).await?;
            let bytes = tokio::fs::read(&path).await?;
            let ext = preview::ext_of(&path);
            let content = match ext.as_str() {
                "odt" => shiny_plugin_sdk::odt::odt_to_plain_text(&bytes)?,
                "ods" => {
                    let cells = shiny_plugin_sdk::ods::ods_to_cells(&bytes)?;
                    cells
                        .iter()
                        .map(|(k, v)| format!("{k}\t{v}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                }
                "odp" => {
                    let slides = shiny_plugin_sdk::odp::odp_to_slides(&bytes)?;
                    slides
                        .iter()
                        .enumerate()
                        .map(|(i, s)| {
                            let mut block = format!("Slide {} — {}", i + 1, s.title);
                            if !s.subtitle.is_empty() {
                                block.push_str(&format!("\n{}", s.subtitle));
                            }
                            for b in &s.bullets {
                                block.push_str(&format!("\n• {b}"));
                            }
                            if !s.body.is_empty() {
                                block.push_str(&format!("\n{}", s.body));
                            }
                            block
                        })
                        .collect::<Vec<_>>()
                        .join("\n\n")
                }
                _ => {
                    let limit = 1024 * 1024usize;
                    let slice = &bytes[..bytes.len().min(limit)];
                    String::from_utf8_lossy(slice).into_owned()
                }
            };
            Ok(ok(json!({
                "path": fs_util::rel_display(&home, &path),
                "content": content,
            })))
        }
    })
}

/// Rich preview for Office documents: `.odt` → HTML, `.ods` → a cell grid,
/// `.odp` → slide data. Powers the quick-look so documents don't read as raw
/// bytes. Uses the SDK's OpenDocument codecs.
fn render(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let _ = &ctx;
        async move {
            let uid = user_id(&req)?;
            let home = fs_util::ensure_home(&uid).await?;
            let (q, _req) = take_query::<PathQuery>(req).await?;
            let rel = q.path.unwrap_or_default();
            let path = fs_util::resolve(&home, &rel).await?;
            let bytes = tokio::fs::read(&path).await?;
            let display = fs_util::rel_display(&home, &path);
            let data = match preview::ext_of(&path).as_str() {
                "odt" => json!({
                    "format": "doc",
                    "html": shiny_plugin_sdk::odt::odt_to_html(&bytes)?,
                }),
                "ods" => json!({
                    "format": "sheet",
                    "grid": ods_grid(&bytes)?,
                }),
                "odp" => json!({
                    "format": "slides",
                    "slides": odp_slides(&bytes)?,
                }),
                _ => return Err(AppError::BadRequest("no rich preview for this type".into())),
            };
            let mut out = data;
            if let Some(obj) = out.as_object_mut() {
                obj.insert("path".into(), json!(display));
            }
            Ok(ok(out))
        }
    })
}

/// Parse an A1-style cell reference into `(row, col)` zero-based indices.
fn cell_ref(key: &str) -> Option<(usize, usize)> {
    let mut col = 0usize;
    let mut row = 0usize;
    let mut seen_digit = false;
    for c in key.chars() {
        if c.is_ascii_alphabetic() {
            if seen_digit {
                return None;
            }
            col = col * 26 + (c.to_ascii_uppercase() as usize - 'A' as usize + 1);
        } else if c.is_ascii_digit() {
            seen_digit = true;
            row = row * 10 + (c as usize - '0' as usize);
        } else {
            return None;
        }
    }
    if col == 0 || row == 0 {
        return None;
    }
    Some((row - 1, col - 1))
}

/// Turn an `.ods` cell map into a rectangular grid (capped to keep the payload
/// small for very large sheets).
fn ods_grid(bytes: &[u8]) -> Result<Json, AppError> {
    const MAX_ROWS: usize = 300;
    const MAX_COLS: usize = 32;
    let cells = shiny_plugin_sdk::ods::ods_to_cells(bytes)?;
    let mut rows: Vec<Vec<String>> = Vec::new();
    for (key, value) in &cells {
        if let Some((r, c)) = cell_ref(key) {
            if r >= MAX_ROWS || c >= MAX_COLS {
                continue;
            }
            while rows.len() <= r {
                rows.push(Vec::new());
            }
            if rows[r].len() <= c {
                rows[r].resize(c + 1, String::new());
            }
            rows[r][c] = value.clone();
        }
    }
    Ok(json!(rows))
}

/// Turn an `.odp` into the slide fields the preview renders.
fn odp_slides(bytes: &[u8]) -> Result<Json, AppError> {
    let slides = shiny_plugin_sdk::odp::odp_to_slides(bytes)?;
    let out: Vec<Json> = slides
        .iter()
        .map(|s| {
            json!({
                "layout": s.layout,
                "title": s.title,
                "subtitle": s.subtitle,
                "bullets": s.bullets,
                "columns": s.columns,
                "body": s.body,
                "attribution": s.attribution,
            })
        })
        .collect();
    Ok(json!(out))
}

/// Inline bytes with range support — powers `<img>`, `<video>`, `<audio>` and
/// the PDF.js fetch. Auth comes from the same-origin session cookie.
fn raw(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let _ = &ctx;
        async move {
            let uid = user_id(&req)?;
            let home = fs_util::ensure_home(&uid).await?;
            let (q, req) = take_query::<PathQuery>(req).await?;
            let rel = q.path.unwrap_or_default();
            let path = fs_util::resolve(&home, &rel).await?;
            let meta = tokio::fs::metadata(&path)
                .await
                .map_err(|_| AppError::NotFound("file not found".into()))?;
            if !meta.is_file() {
                return Err(AppError::BadRequest("not a file".into()));
            }
            let len = meta.len();
            let ct = preview::mime_for(&path);
            let filename = sanitize_filename(
                path.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "file".into())
                    .as_str(),
            );
            let range = req
                .headers()
                .get(RANGE)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);

            // A ranged request is served as one bounded window. The plugin may
            // not hand the host a live file stream: a body built with
            // `Body::from_stream` is polled on the *host* runtime, and a
            // `tokio::fs::File` opened on the plugin runtime then panics with
            // "no reactor running" and aborts the process (PLUGINS.md §15). So
            // the window is read on the plugin runtime and returned as bytes.
            //
            // The window is capped at `RANGE_CHUNK`, so an open-ended
            // `bytes=start-` over a multi-GB video costs a few MB of RAM, not
            // the whole remainder; HTML media simply asks for the next window
            // as it plays. That is what makes progressive playback and seeking
            // work over the network port.
            if let Some((start, end)) = range.as_deref().and_then(|r| parse_range(r, len as usize)) {
                use tokio::io::{AsyncReadExt, AsyncSeekExt};
                let end = end.min(start.saturating_add(RANGE_CHUNK - 1));
                let mut file = tokio::fs::File::open(&path).await?;
                file.seek(std::io::SeekFrom::Start(start as u64)).await?;
                let mut buf = vec![0u8; end - start + 1];
                file.read_exact(&mut buf).await?;
                return Ok(Response::builder()
                    .status(axum::http::StatusCode::PARTIAL_CONTENT)
                    .header(CONTENT_TYPE, ct)
                    .header(CONTENT_LENGTH, buf.len().to_string())
                    .header(CONTENT_RANGE, format!("bytes {start}-{end}/{len}"))
                    .header(ACCEPT_RANGES, "bytes")
                    .body(Body::from(buf))
                    .map_err(|e| AppError::Internal(format!("response: {e}")))?);
            }

            // No `Range`: images, PDF.js and small files are read whole (capped
            // below). Media elements always range, so this is never the video
            // path in practice.
            if len as usize > MAX_INLINE {
                return Err(AppError::BadRequest("file too large to inline".into()));
            }
            let bytes = tokio::fs::read(&path).await?;
            Ok(Response::builder()
                .header(CONTENT_TYPE, ct)
                .header(CONTENT_LENGTH, bytes.len().to_string())
                .header(ACCEPT_RANGES, "bytes")
                .header(CONTENT_DISPOSITION, format!("inline; filename=\"{filename}\""))
                .body(Body::from(bytes))
                .map_err(|e| AppError::Internal(format!("response: {e}")))?)
        }
    })
}

/// Downscaled, cached PNG thumbnail for a raster image.
fn thumb(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let _ = &ctx;
        async move {
            let uid = user_id(&req)?;
            let home = fs_util::ensure_home(&uid).await?;
            let (q, _req) = take_query::<ThumbQuery>(req).await?;
            let size = q.size.unwrap_or(256).clamp(32, 1024);
            let rel = q.path.unwrap_or_default();
            let path = fs_util::resolve(&home, &rel).await?;
            let is_video = preview::is_video(&path);
            if !preview::is_thumbnailable_image(&path) && !is_video {
                return Err(AppError::BadRequest("not a thumbnailable file".into()));
            }
            let meta = tokio::fs::metadata(&path)
                .await
                .map_err(|_| AppError::NotFound("file not found".into()))?;
            let cache_dir = home.join(fs_util::THUMBS_DIR);
            tokio::fs::create_dir_all(&cache_dir).await?;
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let key = format!(
                "{:016x}-{}-{mtime}-{size}",
                fnv1a(rel.as_bytes()),
                meta.len()
            );
            let cache_file = cache_dir.join(format!("{key}.png"));
            if let Ok(cached) = tokio::fs::read(&cache_file).await {
                return Ok(png_response(cached));
            }

            let size_inner = size;
            let source = path.clone();
            // Images are decoded with photon-rs; videos get a real frame from
            // ffmpeg — one PNG, never a re-encode of the video itself.
            let png = tokio::task::spawn_blocking(move || {
                if is_video {
                    preview::video_thumbnail_png(&source, size_inner)
                } else {
                    let bytes = std::fs::read(&source)
                        .map_err(|e| AppError::Internal(format!("read: {e}")))?;
                    preview::thumbnail_png(&bytes, size_inner)
                }
            })
            .await
            .map_err(|e| AppError::Internal(format!("thumbnail task: {e}")))??;
            let _ = tokio::fs::write(&cache_file, &png).await;
            Ok(png_response(png))
        }
    })
}

fn png_response(bytes: Vec<u8>) -> Response {
    Response::builder()
        .header(CONTENT_TYPE, "image/png")
        .header(CONTENT_LENGTH, bytes.len().to_string())
        .body(Body::from(bytes))
        .unwrap_or_else(|_| AppError::Internal("response".into()).into_response())
}

/// `ffprobe` metadata for a video (duration, size, codecs). Powers the custom
/// player's timeline; the window never has to rely on the webview to decode a
/// header just to learn how long the clip is.
fn video_info(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let _ = &ctx;
        async move {
            let uid = user_id(&req)?;
            let home = fs_util::ensure_home(&uid).await?;
            let (q, _req) = take_query::<PathQuery>(req).await?;
            let path = fs_util::resolve(&home, &q.path.unwrap_or_default()).await?;
            if !preview::is_video(&path) {
                return Err(AppError::BadRequest("not a video".into()));
            }
            let info = tokio::task::spawn_blocking(move || preview::video_info_json(&path))
                .await
                .map_err(|e| AppError::Internal(format!("ffprobe task: {e}")))?;
            Ok(ok(info.unwrap_or_else(|| json!({ "duration": 0.0 }))))
        }
    })
}

/// One ffmpeg-extracted PNG frame at `t` seconds — the player poster and the
/// scrub preview. Always a still frame; the video stream itself is untouched.
fn video_frame(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let _ = &ctx;
        async move {
            let uid = user_id(&req)?;
            let home = fs_util::ensure_home(&uid).await?;
            let (q, _req) = take_query::<FrameQuery>(req).await?;
            let path = fs_util::resolve(&home, &q.path.unwrap_or_default()).await?;
            if !preview::is_video(&path) {
                return Err(AppError::BadRequest("not a video".into()));
            }
            let size = q.size.unwrap_or(640).clamp(64, 1024);
            let at = q.t.unwrap_or(0.0);
            let png = tokio::task::spawn_blocking(move || preview::video_frame_png(&path, at, size))
                .await
                .map_err(|e| AppError::Internal(format!("frame task: {e}")))??;
            Ok(png_response(png))
        }
    })
}

/// Stable 64-bit FNV-1a (used only for cache keys).
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn download(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let _ = &ctx;
        async move {
            let uid = user_id(&req)?;
            let home = fs_util::ensure_home(&uid).await?;
            let (q, _req) = take_query::<PathQuery>(req).await?;
            let rel = q.path.unwrap_or_default();
            let path = fs_util::resolve(&home, &rel).await?;
            let meta = tokio::fs::metadata(&path)
                .await
                .map_err(|_| AppError::NotFound("file not found".into()))?;
            if !meta.is_file() {
                return Err(AppError::BadRequest("cannot download a folder".into()));
            }
            if meta.len() as usize > MAX_INLINE {
                return Err(AppError::BadRequest("file too large to download".into()));
            }
            let bytes = tokio::fs::read(&path).await?;
            let filename = sanitize_filename(
                path.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "file".into())
                    .as_str(),
            );
            Ok(Response::builder()
                .header(CONTENT_TYPE, preview::mime_for(&path))
                .header(CONTENT_LENGTH, bytes.len().to_string())
                .header(
                    CONTENT_DISPOSITION,
                    format!("attachment; filename=\"{filename}\""),
                )
                .body(Body::from(bytes))
                .map_err(|e| AppError::Internal(format!("response: {e}")))?)
        }
    })
}

fn upload(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let _ = &ctx;
        async move {
            let uid = user_id(&req)?;
            let home = fs_util::ensure_home(&uid).await?;
            let (q, req) = take_query::<UploadQuery>(req).await?;
            let name = clean_name(&q.name.unwrap_or_default())?;
            let dir_rel = q.path.unwrap_or_default();
            let dir = fs_util::resolve(&home, &dir_rel).await?;
            require_dir(&dir).await?;

            // Raw octet-stream body with an explicit limit — the plugin's axum
            // copy can't see core's DefaultBodyLimit extension across the
            // dlopen boundary, so the limit is passed directly here.
            let bytes = axum::body::to_bytes(req.into_body(), MAX_UPLOAD)
                .await
                .map_err(|e| AppError::BadRequest(format!("upload too large or invalid: {e}")))?;
            let rel = if dir_rel.trim().is_empty() {
                name.clone()
            } else {
                format!("{}/{}", dir_rel.trim().trim_end_matches('/'), name)
            };
            let dest = fs_util::resolve(&home, &rel).await?;
            tokio::fs::write(&dest, &bytes).await?;
            Ok(ok(json!({
                "path": fs_util::rel_display(&home, &dest),
                "name": name,
                "size": bytes.len(),
            })))
        }
    })
}

fn write(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let _ = &ctx;
        async move {
            let uid = user_id(&req)?;
            let home = fs_util::ensure_home(&uid).await?;
            let axum::Json(body) = axum::Json::<WriteBody>::from_request(req, &())
                .await
                .map_err(|e| AppError::BadRequest(format!("invalid JSON body: {e}")))?;
            let rel = body.path.unwrap_or_default();
            let content = body.content.unwrap_or_default();
            let dest = fs_util::resolve(&home, &rel).await?;
            if let Some(parent) = dest.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            if body.append.unwrap_or(false) {
                use tokio::io::AsyncWriteExt;
                let mut f = tokio::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&dest)
                    .await?;
                f.write_all(content.as_bytes()).await?;
            } else {
                tokio::fs::write(&dest, content.as_bytes()).await?;
            }
            Ok(ok(json!({
                "path": fs_util::rel_display(&home, &dest),
                "size": content.len(),
            })))
        }
    })
}

fn mkdir(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let _ = &ctx;
        async move {
            let uid = user_id(&req)?;
            let home = fs_util::ensure_home(&uid).await?;
            let axum::Json(body) = axum::Json::<PathBody>::from_request(req, &())
                .await
                .map_err(|e| AppError::BadRequest(format!("invalid JSON body: {e}")))?;
            let rel = body.path.unwrap_or_default();
            if rel.trim().is_empty() {
                return Err(AppError::BadRequest("path required".into()));
            }
            let dest = fs_util::resolve(&home, &rel).await?;
            tokio::fs::create_dir_all(&dest).await?;
            Ok(ok(json!({ "path": fs_util::rel_display(&home, &dest) })))
        }
    })
}

fn rename(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let _ = &ctx;
        async move {
            let uid = user_id(&req)?;
            let home = fs_util::ensure_home(&uid).await?;
            let axum::Json(body) = axum::Json::<RenameBody>::from_request(req, &())
                .await
                .map_err(|e| AppError::BadRequest(format!("invalid JSON body: {e}")))?;
            let from_rel = body.from.unwrap_or_default();
            let to_rel = body.to.unwrap_or_default();
            if from_rel.trim().is_empty() || to_rel.trim().is_empty() {
                return Err(AppError::BadRequest("from and to are required".into()));
            }
            let from = fs_util::resolve(&home, &from_rel).await?;
            let to = fs_util::resolve(&home, &to_rel).await?;
            if !tokio::fs::symlink_metadata(&from).await.is_ok() {
                return Err(AppError::NotFound("source not found".into()));
            }
            if let Some(parent) = to.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            tokio::fs::rename(&from, &to).await?;
            Ok(ok(json!({
                "from": fs_util::rel_display(&home, &from),
                "to": fs_util::rel_display(&home, &to),
            })))
        }
    })
}

fn delete(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let _ = &ctx;
        async move {
            let uid = user_id(&req)?;
            let home = fs_util::ensure_home(&uid).await?;
            let axum::Json(body) = axum::Json::<PathBody>::from_request(req, &())
                .await
                .map_err(|e| AppError::BadRequest(format!("invalid JSON body: {e}")))?;
            let rel = body.path.unwrap_or_default();
            if rel.trim().is_empty() {
                return Err(AppError::BadRequest("cannot delete the home folder".into()));
            }
            let path = fs_util::resolve(&home, &rel).await?;
            if !tokio::fs::symlink_metadata(&path).await.is_ok() {
                return Err(AppError::NotFound("file not found".into()));
            }
            // Deleting from inside Trash is permanent; elsewhere it moves to Trash.
            if rel == fs_util::TRASH_DIR
                || rel.starts_with(&format!("{}/", fs_util::TRASH_DIR))
            {
                fs_util::delete_permanent(&path).await?;
                return Ok(ok(json!({ "deleted": fs_util::rel_display(&home, &path) })));
            }
            let trashed = fs_util::move_to_trash(&home, &path).await?;
            Ok(ok(json!({ "trashed": trashed })))
        }
    })
}

fn restore(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let _ = &ctx;
        async move {
            let uid = user_id(&req)?;
            let home = fs_util::ensure_home(&uid).await?;
            let axum::Json(body) = axum::Json::<PathBody>::from_request(req, &())
                .await
                .map_err(|e| AppError::BadRequest(format!("invalid JSON body: {e}")))?;
            let name = clean_name(&body.name.or(body.path).unwrap_or_default())?;
            let restored = fs_util::restore_from_trash(&home, &name).await?;
            Ok(ok(json!({ "restored": restored })))
        }
    })
}

fn empty_trash(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let _ = &ctx;
        async move {
            let uid = user_id(&req)?;
            let home = fs_util::ensure_home(&uid).await?;
            fs_util::empty_trash(&home).await?;
            Ok(ok(json!({ "emptied": true })))
        }
    })
}

fn search(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let _ = &ctx;
        async move {
            let uid = user_id(&req)?;
            let home = fs_util::ensure_home(&uid).await?;
            let (q, _req) = take_query::<SearchQuery>(req).await?;
            let needle = q.q.unwrap_or_default();
            if needle.trim().len() < 2 {
                return Ok(ok(json!({ "query": needle, "results": [] })));
            }
            let start = fs_util::resolve(&home, &q.path.unwrap_or_default()).await?;
            let results = fs_util::search(&home, &start, needle.trim(), 200).await?;
            Ok(ok(json!({ "query": needle, "results": results })))
        }
    })
}
