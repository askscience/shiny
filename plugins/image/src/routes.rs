//! Image plugin REST routes — served through the plugin's `RouteSpec`s.
//!
//! The Image window uploads, lists, edits and deletes images here. Images are
//! stored as **raw RGBA** BLOBs (see `session.rs`): the real-time edit path
//! (`POST /api/images/:id/apply?raw=1`) mutates in-memory pixels with no codec
//! work, and `GET /api/images/:id/data` encodes to PNG only when pixels are
//! actually served/downloaded.

use std::sync::Arc;

use axum::extract::{FromRequest, FromRequestParts, Multipart};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value as Json};

use shiny_plugin_sdk::db::Value;
use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::routes::{bridged_route, path_params_from_request, user_id_from_request, RouteHandler};
use shiny_plugin_sdk::services::PluginCtx;

use crate::{ops, session};

const MAX_DIM: u32 = 1600;
const MAX_UPLOAD: usize = 32 * 1024 * 1024;

pub fn handle(ctx: &Arc<PluginCtx>, tag: &str) -> Option<RouteHandler> {
    let ctx = ctx.clone();
    Some(match tag {
        "image_list" => image_list(ctx),
        "image_create" => image_create(ctx),
        "image_get" => image_get(ctx),
        "image_data" => image_data(ctx),
        "image_rename" => image_rename(ctx),
        "image_apply" => image_apply(ctx),
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

fn take_path(req: &axum::extract::Request) -> Result<String, AppError> {
    let mut params = path_params_from_request(req)
        .ok_or_else(|| AppError::BadRequest("no path parameter found".into()))?;
    if params.len() != 1 {
        return Err(AppError::BadRequest("expected exactly one path parameter".into()));
    }
    Ok(params.remove(0).1)
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

fn clean_title(t: &str) -> String {
    let t = t.trim();
    if t.is_empty() { "Untitled".into() } else { t.chars().take(120).collect() }
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
            let mut multipart = Multipart::from_request(req, &())
                .await
                .map_err(|e| AppError::BadRequest(format!("multipart error: {e}")))?;

            let mut bytes: Option<Vec<u8>> = None;
            let mut original_name: Option<String> = None;
            while let Some(field) = multipart
                .next_field()
                .await
                .map_err(|e| AppError::BadRequest(format!("multipart error: {e}")))?
            {
                if field.name() == Some("file") {
                    original_name = field.file_name().map(|f| f.to_string()).or(original_name);
                    let data = field
                        .bytes()
                        .await
                        .map_err(|e| AppError::BadRequest(format!("read error: {e}")))?;
                    bytes = Some(data.to_vec());
                }
            }
            let data = bytes.ok_or_else(|| AppError::BadRequest("missing 'file' field".into()))?;
            if data.len() > MAX_UPLOAD {
                return Err(AppError::BadRequest("image too large (max 32 MB)".into()));
            }

            let mut img = ops::decode(&data)?;
            ops::fit(&mut img, MAX_DIM);
            let raw = img.get_raw_pixels();
            let w = img.get_width() as i64;
            let h = img.get_height() as i64;

            let stem = original_name
                .as_deref()
                .map(|n| n.rsplit('.').nth(1).unwrap_or(n))
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
                    Value::Int(w),
                    Value::Int(h),
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
            let id = take_path(&req)?;
            let rows = ctx.db().query(
                "SELECT id, title, width, height, updated_at FROM images \
                 WHERE id = ?1 AND user_id = ?2",
                &[Value::text(&id), Value::text(&uid)],
            )?;
            let row = rows.first().ok_or_else(|| AppError::NotFound("Image not found".into()))?;
            Ok(ok(json!({
                "image_id": as_text(&row[0]),
                "title": as_text(&row[1]),
                "width": as_int(&row[2]),
                "height": as_int(&row[3]),
                "updated_at": as_text(&row[4]),
            })))
        }
    })
}

/* ── GET /api/images/:id/data (raw PNG) ─────────────────────── */

fn image_data(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_path(&req)?;
            let (raw, w, h) = session::with_session(&ctx.db(), &uid, &id, false, |s| {
                Ok((s.raw.clone(), s.w, s.h))
            })?;
            let png = ops::encode_png(&raw, w, h);
            Ok(Response::builder()
                .header(CONTENT_TYPE, "image/png")
                .header(CACHE_CONTROL, "no-store")
                .body(axum::body::Body::from(png))
                .map_err(|e| AppError::Internal(format!("failed to build image response: {e}")))?)
        }
    })
}

/* ── PUT /api/images/:id (rename) ───────────────────────────── */

fn image_rename(ctx: Arc<PluginCtx>) -> RouteHandler {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Rename {
        title: Option<String>,
    }

    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_path(&req)?;
            let axum::Json(body) = axum::Json::<Rename>::from_request(req, &())
                .await
                .map_err(|e| AppError::BadRequest(format!("invalid JSON body: {e}")))?;
            let title = clean_title(body.title.as_deref().unwrap_or("Untitled"));
            let changed = ctx.db().execute(
                "UPDATE images SET title = ?1, updated_at = datetime('now') \
                 WHERE id = ?2 AND user_id = ?3",
                &[Value::text(&title), Value::text(&id), Value::text(&uid)],
            )?;
            if changed == 0 {
                return Err(AppError::NotFound("Image not found".into()));
            }
            Ok(ok(json!({ "image_id": id, "title": title })))
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
    }

    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_path(&req)?;
            let (q, req) = take_query::<Q>(req).await?;
            let axum::Json(body) = axum::Json::<Apply>::from_request(req, &())
                .await
                .map_err(|e| AppError::BadRequest(format!("invalid JSON body: {e}")))?;

            let operations = body.operations.unwrap_or_default();
            if operations.is_empty() {
                return Err(AppError::BadRequest("operations required".into()));
            }
            let want_raw = q.raw.unwrap_or(false);
            let commit = q.commit.unwrap_or(true);

            let (raw, w, h) = session::with_session(&ctx.db(), &uid, &id, commit, |s| {
                let (nr, nw, nh) = ops::apply_raw(
                    &s.raw, s.w, s.h,
                    &s.original, s.orig_w, s.orig_h,
                    &operations,
                )?;
                s.raw = nr.clone();
                s.w = nw;
                s.h = nh;
                Ok((nr, nw, nh))
            })?;

            if want_raw {
                return Ok(Response::builder()
                    .header(CONTENT_TYPE, "application/octet-stream")
                    .header("x-image-width", w.to_string())
                    .header("x-image-height", h.to_string())
                    .header(CACHE_CONTROL, "no-store")
                    .body(axum::body::Body::from(raw))
                    .map_err(|e| AppError::Internal(format!("failed to build raw response: {e}")))?);
            }

            Ok(ok(json!({ "image_id": id, "width": w, "height": h, "updated_at": "now" })))
        }
    })
}

/* ── DELETE /api/images/:id ─────────────────────────────────── */

fn image_delete(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_path(&req)?;
            let changed = ctx.db().execute(
                "DELETE FROM images WHERE id = ?1 AND user_id = ?2",
                &[Value::text(&id), Value::text(&uid)],
            )?;
            if changed == 0 {
                return Err(AppError::NotFound("Image not found".into()));
            }
            session::clear(&id);
            Ok(ok(json!({ "image_id": id, "deleted": true })))
        }
    })
}
