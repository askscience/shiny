//! Image plugin REST routes — served through the plugin's `RouteSpec`s.
//!
//! A document is a single picture (`images.bytes`, raw RGBA once touched) plus
//! the untouched upload (`images.original`). `POST /api/images/:id/apply` is
//! the shared operations entry point used by both the window and the agent
//! tools; `GET /api/images/:id/looks` renders the look-preview strip.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{FromRequest, FromRequestParts};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::response::{IntoResponse, Response};
use photon_rs::transform::SamplingFilter;
use photon_rs::PhotonImage;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Map, Value as Json};

use shiny_plugin_sdk::db::Value;
use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::routes::{bridged_route, path_params_from_request, user_id_from_request, RouteHandler};
use shiny_plugin_sdk::services::PluginCtx;

use crate::{doc, ops};

const MAX_DIM: u32 = 1600;
const MAX_UPLOAD: usize = 32 * 1024 * 1024;

pub fn handle(ctx: &Arc<PluginCtx>, tag: &str) -> Option<RouteHandler> {
    let ctx = ctx.clone();
    Some(match tag {
        "image_list" => image_list(ctx),
        "image_create" => image_create(ctx),
        "image_get" => image_get(ctx),
        "image_data" => image_data(ctx),
        "image_apply" => image_apply(ctx),
        "image_render" => image_render(ctx),
        "image_looks" => image_looks(ctx),
        "image_delete" => image_delete(ctx),
        _ => return None,
    })
}

fn user_id(req: &axum::extract::Request) -> Result<String, AppError> {
    user_id_from_request(req)
        .ok_or_else(|| AppError::Unauthorized("not authenticated".into()))
}

fn ok(data: Json) -> Response {
    axum::Json(json!({ "success": true, "data": data })).into_response()
}

fn as_text(v: &Value) -> String {
    match v {
        Value::Text(s) => s.clone(),
        Value::Int(n) => n.to_string(),
        _ => String::new(),
    }
}

fn as_int(v: &Value) -> i64 {
    match v {
        Value::Int(n) => *n,
        _ => 0,
    }
}

fn as_opt_text(v: &Value) -> Option<String> {
    if matches!(v, Value::Null) {
        return None;
    }
    let s = as_text(v);
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

fn path_params(req: &axum::extract::Request) -> HashMap<String, String> {
    path_params_from_request(req)
        .map(|p| p.into_iter().collect())
        .unwrap_or_default()
}

fn path_param(req: &axum::extract::Request, name: &str) -> Result<String, AppError> {
    if let Some(v) = path_params(req).get(name) {
        if !v.is_empty() {
            return Ok(v.clone());
        }
    }
    // Fall back to the first positional param (older `:id` routes).
    path_params_from_request(req)
        .and_then(|p| p.into_iter().next().map(|(_, v)| v))
        .filter(|v| !v.is_empty())
        .ok_or_else(|| AppError::BadRequest(format!("missing path parameter '{name}'")))
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

async fn take_json<T: DeserializeOwned>(req: axum::extract::Request) -> Result<T, AppError> {
    let axum::Json(body) = axum::Json::<T>::from_request(req, &())
        .await
        .map_err(|e| AppError::BadRequest(format!("invalid JSON body: {e}")))?;
    Ok(body)
}

fn clean_title(t: &str) -> String {
    let t = t.trim();
    if t.is_empty() { "Untitled".into() } else { t.chars().take(120).collect() }
}

/// Query for upload requests: the file's name (for the document title).
#[derive(Deserialize, Default)]
#[serde(default)]
struct UploadQuery {
    name: Option<String>,
}

/// Read a request body sent as raw bytes.
///
/// Consuming the body with `to_bytes` directly bypasses axum's 2MB
/// `DefaultBodyLimit` (which only applies to extractors that opt in), so a
/// large photo uploads without tripping over the limit.
async fn read_raw_upload(
    req: axum::extract::Request,
) -> Result<(Vec<u8>, Option<String>), AppError> {
    let (q, req) = take_query::<UploadQuery>(req).await?;
    let bytes = axum::body::to_bytes(req.into_body(), MAX_UPLOAD + 1)
        .await
        .map_err(|e| AppError::BadRequest(format!("read error: {e}")))?;
    if bytes.len() > MAX_UPLOAD {
        return Err(AppError::BadRequest("image too large (max 32 MB)".into()));
    }
    Ok((bytes.to_vec(), q.name))
}

/// Raw RGBA response. `look` and the effective dial `adjust` (both part of an
/// apply response) travel in `x-image-look` / `x-image-adjust` headers —
/// empty string / `{}` mean no look, neutral dials.
fn raw_response(
    raw: Vec<u8>,
    w: u32,
    h: u32,
    look: Option<&str>,
    adjust: &Map<String, Json>,
) -> Result<Response, AppError> {
    Response::builder()
        .header(CONTENT_TYPE, "application/octet-stream")
        .header("x-image-width", w.to_string())
        .header("x-image-height", h.to_string())
        .header("x-image-look", look.unwrap_or(""))
        .header("x-image-adjust", Json::Object(adjust.clone()).to_string())
        .header(CACHE_CONTROL, "no-store")
        .body(axum::body::Body::from(raw))
        .map_err(|e| AppError::Internal(format!("failed to build raw response: {e}")))
}

fn png_response(raw: &[u8], w: u32, h: u32) -> Result<Response, AppError> {
    let png = ops::encode_png(raw, w, h);
    Response::builder()
        .header(CONTENT_TYPE, "image/png")
        .header(CACHE_CONTROL, "no-store")
        .body(axum::body::Body::from(png))
        .map_err(|e| AppError::Internal(format!("failed to build image response: {e}")))
}

/// A square center-cropped thumbnail, used as the base for the look strip.
fn cover_thumb(raw: &[u8], w: u32, h: u32, size: u32) -> Result<PhotonImage, AppError> {
    let (w, h) = (w.max(1), h.max(1));
    crate::ops::checked_layer_len(w, h)?;
    if raw.len() < (w as usize) * (h as usize) * 4 {
        return Err(AppError::BadRequest("image data is incomplete".into()));
    }
    let img = PhotonImage::new(raw.to_vec(), w, h);
    let shortest = w.min(h) as f64;
    let scale = (size as f64 / shortest).max(1.0);
    let nw = ((w as f64) * scale).round().max(size as f64) as u32;
    let nh = ((h as f64) * scale).round().max(size as f64) as u32;
    let resized = if nw != w || nh != h {
        photon_rs::transform::resize(&img, nw, nh, SamplingFilter::Lanczos3)
    } else {
        img
    };
    let x0 = (resized.get_width().saturating_sub(size)) / 2;
    let y0 = (resized.get_height().saturating_sub(size)) / 2;
    Ok(photon_rs::transform::crop(&resized, x0, y0, x0 + size, y0 + size))
}

/* ── GET /api/images ────────────────────────────────────────── */

fn image_list(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let rows = ctx.db().query(
                "SELECT id, title, width, height, updated_at FROM images \
                 WHERE user_id = ?1 ORDER BY updated_at DESC LIMIT 200",
                &[Value::text(&uid)],
            )?;
            let images: Vec<Json> = rows
                .iter()
                .map(|r| json!({
                    "image_id": as_text(&r[0]),
                    "title": as_text(&r[1]),
                    "width": as_int(&r[2]),
                    "height": as_int(&r[3]),
                    "updated_at": as_text(&r[4]),
                }))
                .collect();
            Ok(ok(json!({ "images": images, "count": images.len() })))
        }
    })
}

/* ── POST /api/images (upload) ──────────────────────────────── */

fn image_create(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let (data, original_name) = read_raw_upload(req).await?;

            let mut img = ops::decode(&data)?;
            ops::fit(&mut img, MAX_DIM);
            let raw = img.get_raw_pixels();
            let w = img.get_width();
            let h = img.get_height();

            let stem = original_name
                .as_deref()
                .map(|n| n.split('.').next().filter(|s| !s.is_empty()).unwrap_or(n))
                .unwrap_or("Untitled");
            let title = clean_title(stem);
            let id = uuid::Uuid::new_v4().to_string();

            ctx.db().execute(
                "INSERT INTO images \
                 (id, user_id, title, width, height, bytes, original, format, orig_width, orig_height, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'rgba', ?4, ?5, datetime('now'), datetime('now'))",
                &[
                    Value::text(&id),
                    Value::text(&uid),
                    Value::text(&title),
                    Value::Int(w as i64),
                    Value::Int(h as i64),
                    Value::blob(raw.clone()),
                    Value::blob(raw.clone()),
                ],
            )?;

            Ok(ok(json!({ "image_id": id, "title": title, "width": w, "height": h, "updated_at": "now" })))
        }
    })
}

/* ── GET /api/images/:id ────────────────────────────────────── */

fn image_get(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = path_param(&req, "id")?;
            let rows = ctx.db().query(
                "SELECT id, title, width, height, updated_at, look, adjust FROM images \
                 WHERE id = ?1 AND user_id = ?2",
                &[Value::text(&id), Value::text(&uid)],
            )?;
            let row = rows.first().ok_or_else(|| AppError::NotFound("Image not found".into()))?;
            let looks: Vec<Json> = crate::looks::LOOKS
                .iter()
                .map(|l| json!({ "key": l.key, "name": l.name }))
                .collect();
            let adjust: Json =
                serde_json::from_str(&as_text(&row[6])).unwrap_or_else(|_| json!({}));
            Ok(ok(json!({
                "image_id": as_text(&row[0]),
                "title": as_text(&row[1]),
                "width": as_int(&row[2]),
                "height": as_int(&row[3]),
                "updated_at": as_text(&row[4]),
                "look": as_opt_text(&row[5]),
                "adjust": adjust,
                "looks": looks,
            })))
        }
    })
}

/* ── GET /api/images/:id/data (PNG) ─────────────────────────── */

fn image_data(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = path_param(&req, "id")?;
            let d = doc::load(&ctx.db(), &uid, &id)?;
            let raw = doc::visible(&d)?;
            png_response(&raw, d.w, d.h)
        }
    })
}

/* ── POST /api/images/:id/apply ─────────────────────────────── */

fn image_apply(ctx: Arc<PluginCtx>) -> RouteHandler {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Q {
        raw: Option<bool>,
        commit: Option<bool>,
    }

    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Apply {
        operations: Option<Vec<Json>>,
        operation: Option<Json>,
        /// Partial dial update (`{"brightness": 30}`) — merged into the live
        /// settings instead of baking an edit.
        settings: Option<Map<String, Json>>,
    }

    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = path_param(&req, "id")?;
            let (q, req) = take_query::<Q>(req).await?;
            let body: Apply = take_json(req).await?;

            let want_raw = q.raw.unwrap_or(false);
            let commit = q.commit.unwrap_or(true);
            let db = ctx.db();

            let res = if let Some(settings) = body.settings {
                doc::apply_settings(&db, &uid, &id, &settings, commit)?
            } else {
                let operations = match (body.operations, body.operation) {
                    (Some(ops), _) if !ops.is_empty() => ops,
                    (_, Some(op)) => vec![op],
                    _ => return Err(AppError::BadRequest("operations required".into())),
                };
                doc::apply_operations(&db, &uid, &id, &operations, commit)?
            };

            if want_raw {
                let adjust = res.adjust.as_object().cloned().unwrap_or_default();
                return raw_response(res.raw, res.w, res.h, res.look.as_deref(), &adjust);
            }
            Ok(ok(json!({
                "image_id": id,
                "width": res.w,
                "height": res.h,
                "look": res.look,
                "adjust": res.adjust,
                "updated_at": "now",
            })))
        }
    })
}

/* ── GET /api/images/:id/render ─────────────────────────────── */

fn image_render(ctx: Arc<PluginCtx>) -> RouteHandler {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Q {
        raw: Option<bool>,
    }

    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = path_param(&req, "id")?;
            let (q, _req) = take_query::<Q>(req).await?;
            let d = doc::load(&ctx.db(), &uid, &id)?;
            let raw = doc::visible(&d)?;
            if q.raw.unwrap_or(false) {
                return raw_response(raw, d.w, d.h, d.look.as_deref(), &d.adjust);
            }
            png_response(&raw, d.w, d.h)
        }
    })
}

/* ── GET /api/images/:id/looks (preview strip) ──────────────── */

/// One horizontal strip PNG of every look applied to a small square crop of
/// the open image — the window slices it into per-look thumbnails.
fn image_looks(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = path_param(&req, "id")?;
            let d = doc::load(&ctx.db(), &uid, &id)?;
            let tile = crate::looks::TILE as usize;
            // Tiles render each look over the current settings (no look), so a
            // swap stays meaningful and dial changes update the previews.
            let base_raw = doc::settings_base(&d)?;
            let base = cover_thumb(&base_raw, d.w, d.h, crate::looks::TILE)?;
            let n = crate::looks::LOOKS.len();
            let mut strip = vec![0u8; tile * n * tile * 4];
            let row_bytes = tile * 4;
            for (i, look) in crate::looks::LOOKS.iter().enumerate() {
                let mut preview = base.clone();
                (look.apply)(&mut preview);
                let px = preview.get_raw_pixels();
                for y in 0..tile {
                    let src = y * row_bytes;
                    let dst = (y * tile * n + i * tile) * 4;
                    strip[dst..dst + row_bytes].copy_from_slice(&px[src..src + row_bytes]);
                }
            }
            png_response(&strip, (tile * n) as u32, tile as u32)
        }
    })
}

/* ── DELETE /api/images/:id ─────────────────────────────────── */

fn image_delete(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = path_param(&req, "id")?;
            let changed = ctx.db().execute(
                "DELETE FROM images WHERE id = ?1 AND user_id = ?2",
                &[Value::text(&id), Value::text(&uid)],
            )?;
            if changed == 0 {
                return Err(AppError::NotFound("Image not found".into()));
            }
            Ok(ok(json!({ "image_id": id, "deleted": true })))
        }
    })
}
