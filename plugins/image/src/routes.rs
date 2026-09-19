//! Image plugin REST routes — served through the plugin's `RouteSpec`s.
//!
//! A document lives in `images` (title, canvas size, flattened cache) and its
//! compositing stack in `image_layers`. The window uploads, lists, edits,
//! reorders and composites layers here; `POST /api/images/:id/apply` is the
//! shared operations entry point used by both the window and the agent tools.
//!
//! Real-time path: `POST /api/images/:id/apply?raw=1&commit=0` mutates the
//! target layer in memory and streams the composited raw RGBA straight back
//! with no codec work and no database write.

use std::collections::HashMap;
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

use crate::{layers, ops};

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
        "image_render" => image_render(ctx),
        "image_layer_list" => layer_list(ctx),
        "image_layer_create" => layer_create(ctx),
        "image_layer_update" => layer_update(ctx),
        "image_layer_delete" => layer_delete(ctx),
        "image_layer_image" => layer_image(ctx),
        "image_layer_thumb" => layer_thumb(ctx),
        "image_layer_duplicate" => layer_duplicate(ctx),
        "image_layer_merge" => layer_merge(ctx),
        "image_layer_reorder" => layer_reorder(ctx),
        "image_flatten" => image_flatten(ctx),
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

fn is_multipart(req: &axum::extract::Request) -> bool {
    req.headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.starts_with("multipart/"))
        .unwrap_or(false)
}

fn raw_response(raw: Vec<u8>, w: u32, h: u32) -> Result<Response, AppError> {
    Response::builder()
        .header(CONTENT_TYPE, "application/octet-stream")
        .header("x-image-width", w.to_string())
        .header("x-image-height", h.to_string())
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

fn layer_json(l: &layers::LayerRow) -> Json {
    json!({
        "layer_id": l.id,
        "name": l.name,
        "position": l.position,
        "visible": l.visible,
        "opacity": l.opacity,
        "blend_mode": l.blend.as_str(),
        "x": l.x,
        "y": l.y,
        "width": l.w,
        "height": l.h,
        "is_group": l.is_group,
        "group_id": l.group_id,
        "mask": l.mask.is_some(),
        "mask_enabled": l.mask_enabled,
    })
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

            // Seed the document's first layer so the stack is never empty.
            layers::insert_layer(&ctx.db(), &uid, &id, "Background", None, 0, 0, w, h, raw)?;

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
                "SELECT id, title, width, height, updated_at FROM images \
                 WHERE id = ?1 AND user_id = ?2",
                &[Value::text(&id), Value::text(&uid)],
            )?;
            let row = rows.first().ok_or_else(|| AppError::NotFound("Image not found".into()))?;
            let layer_count = layers::load_layers(&ctx.db(), &uid, &id)?.len();
            Ok(ok(json!({
                "image_id": as_text(&row[0]),
                "title": as_text(&row[1]),
                "width": as_int(&row[2]),
                "height": as_int(&row[3]),
                "updated_at": as_text(&row[4]),
                "layer_count": layer_count,
            })))
        }
    })
}

/* ── GET /api/images/:id/data (composited PNG) ──────────────── */

fn image_data(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = path_param(&req, "id")?;
            let (raw, w, h) = layers::rendered(&ctx.db(), &uid, &id)?;
            png_response(&raw, w, h)
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
            let id = path_param(&req, "id")?;
            let body: Rename = take_json(req).await?;
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
        operation: Option<Json>,
        layer_id: Option<String>,
    }

    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = path_param(&req, "id")?;
            let (q, req) = take_query::<Q>(req).await?;
            let body: Apply = take_json(req).await?;

            let operations = match (body.operations, body.operation) {
                (Some(ops), _) if !ops.is_empty() => ops,
                (_, Some(op)) => vec![op],
                _ => return Err(AppError::BadRequest("operations required".into())),
            };
            let want_raw = q.raw.unwrap_or(false);
            let commit = q.commit.unwrap_or(true);
            let db = ctx.db();

            // Make sure the document has a stack, then resolve the target.
            layers::ensure_base_layer(&db, &uid, &id)?;
            let target = layers::active_layer(&db, &uid, &id, body.layer_id.as_deref())?;
            let (new_raw, nw, nh) = ops::apply_raw(
                &target.bytes,
                target.w,
                target.h,
                &target.original,
                target.w,
                target.h,
                &operations,
            )?;

            let (comp_raw, cw, ch) = if commit {
                layers::set_layer_pixels(
                    &db, &uid, &id, &target.id, new_raw, target.original.clone(), nw, nh,
                    target.x, target.y,
                )?;
                layers::refresh_composite(&db, &uid, &id)?
            } else {
                // Preview: fold the would-be layer into the loaded stack and
                // composite in memory, without touching the database.
                let mut rows = layers::load_layers(&db, &uid, &id)?;
                if let Some(row) = rows.iter_mut().find(|l| l.id == target.id) {
                    row.bytes = new_raw;
                    row.w = nw;
                    row.h = nh;
                }
                let doc = layers::load_doc(&db, &uid, &id)?;
                let (w, h) = (doc.width.max(1), doc.height.max(1));
                (layers::compose(&rows, w, h), w, h)
            };

            if want_raw {
                return raw_response(comp_raw, cw, ch);
            }
            Ok(ok(json!({
                "image_id": id,
                "layer_id": target.id,
                "width": nw,
                "height": nh,
                "operations_applied": operations.len(),
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
            let (raw, w, h) = layers::rendered(&ctx.db(), &uid, &id)?;
            if q.raw.unwrap_or(false) {
                return raw_response(raw, w, h);
            }
            png_response(&raw, w, h)
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
            ctx.db().execute(
                "DELETE FROM image_layers WHERE image_id = ?1 AND user_id = ?2",
                &[Value::text(&id), Value::text(&uid)],
            )?;
            Ok(ok(json!({ "image_id": id, "deleted": true })))
        }
    })
}

/* ── GET /api/images/:id/layers ─────────────────────────────── */

fn layer_list(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = path_param(&req, "id")?;
            layers::load_doc(&ctx.db(), &uid, &id)?;
            let rows = layers::ensure_base_layer(&ctx.db(), &uid, &id)?;
            let out: Vec<Json> = rows.iter().map(layer_json).collect();
            Ok(ok(json!({ "layers": out, "count": out.len() })))
        }
    })
}

/* ── POST /api/images/:id/layers ────────────────────────────── */

#[derive(Deserialize, Default)]
#[serde(default)]
struct NewLayer {
    name: Option<String>,
    group_id: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    /// Create a folder instead of a pixel layer.
    is_group: Option<bool>,
}

fn layer_create(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = path_param(&req, "id")?;
            let db = ctx.db();
            let doc = layers::load_doc(&db, &uid, &id)?;

            if is_multipart(&req) {
                let mut multipart = Multipart::from_request(req, &())
                    .await
                    .map_err(|e| AppError::BadRequest(format!("multipart error: {e}")))?;
                let mut file: Option<Vec<u8>> = None;
                let mut name: Option<String> = None;
                let mut group_id: Option<String> = None;
                while let Some(field) = multipart
                    .next_field()
                    .await
                    .map_err(|e| AppError::BadRequest(format!("multipart error: {e}")))?
                {
                    match field.name() {
                        Some("file") => {
                            name = field.file_name().map(|f| f.to_string()).or(name);
                            let data = field
                                .bytes()
                                .await
                                .map_err(|e| AppError::BadRequest(format!("read error: {e}")))?;
                            file = Some(data.to_vec());
                        }
                        Some("name") => {
                            name = Some(
                                field
                                    .text()
                                    .await
                                    .map_err(|e| AppError::BadRequest(format!("read error: {e}")))?,
                            );
                        }
                        Some("group_id") => {
                            group_id = Some(
                                field
                                    .text()
                                    .await
                                    .map_err(|e| AppError::BadRequest(format!("read error: {e}")))?,
                            );
                        }
                        _ => {}
                    }
                }
                let data = file.ok_or_else(|| AppError::BadRequest("missing 'file' field".into()))?;
                if data.len() > MAX_UPLOAD {
                    return Err(AppError::BadRequest("image too large (max 32 MB)".into()));
                }
                let mut img = ops::decode(&data)?;
                ops::fit(&mut img, MAX_DIM);
                let w = img.get_width();
                let h = img.get_height();
                let raw = img.get_raw_pixels();
                // First layer sizes the canvas; later ones land at the origin.
                if doc.width == 0 || doc.height == 0 {
                    db.execute(
                        "UPDATE images SET width = ?1, height = ?2 WHERE id = ?3 AND user_id = ?4",
                        &[
                            Value::Int(w as i64),
                            Value::Int(h as i64),
                            Value::text(&id),
                            Value::text(&uid),
                        ],
                    )?;
                }
                let name = clean_title(
                    &name
                        .as_deref()
                        .map(|n| n.split('.').next().unwrap_or(n).to_string())
                        .unwrap_or_else(|| format!("Layer {}", doc.title)),
                );
                let layer_id = layers::insert_layer(
                    &db,
                    &uid,
                    &id,
                    &name,
                    group_id.as_deref().filter(|s| !s.is_empty()),
                    0,
                    0,
                    w,
                    h,
                    raw,
                )?;
                layers::refresh_composite(&db, &uid, &id)?;
                return Ok(ok(json!({ "layer_id": layer_id, "name": name, "width": w, "height": h })));
            }

            let body: NewLayer = take_json(req).await?;
            let name = clean_title(
                body.name
                    .as_deref()
                    .unwrap_or(&format!("Layer {}", layers::load_layers(&db, &uid, &id)?.len() + 1)),
            );
            if body.is_group.unwrap_or(false) {
                let group_id = layers::create_group(
                    &db,
                    &uid,
                    &id,
                    &name,
                    body.group_id.as_deref().filter(|s| !s.is_empty()),
                )?;
                return Ok(ok(json!({ "layer_id": group_id, "name": name, "is_group": true })));
            }
            let (w, h) = (
                body.width.unwrap_or(doc.width.max(1)),
                body.height.unwrap_or(doc.height.max(1)),
            );
            let raw = vec![0u8; (w as usize) * (h as usize) * 4];
            let layer_id = layers::insert_layer(
                &db,
                &uid,
                &id,
                &name,
                body.group_id.as_deref().filter(|s| !s.is_empty()),
                0,
                0,
                w,
                h,
                raw,
            )?;
            layers::refresh_composite(&db, &uid, &id)?;
            Ok(ok(json!({ "layer_id": layer_id, "name": name, "width": w, "height": h })))
        }
    })
}

/* ── PUT /api/images/:id/layers/:layer_id ───────────────────── */

#[derive(Deserialize, Default)]
#[serde(default)]
struct LayerUpdate {
    name: Option<String>,
    visible: Option<bool>,
    opacity: Option<f32>,
    blend_mode: Option<String>,
    x: Option<i32>,
    y: Option<i32>,
    /// `null` moves to the root; a string moves into that folder.
    group_id: Option<Option<String>>,
}

fn layer_update(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = path_param(&req, "id")?;
            let layer_id = path_param(&req, "layer_id")?;
            let body: LayerUpdate = take_json(req).await?;
            layers::update_layer(
                &ctx.db(),
                &uid,
                &id,
                &layer_id,
                &layers::LayerPatch {
                    name: body.name,
                    visible: body.visible,
                    opacity: body.opacity,
                    blend_mode: body.blend_mode,
                    x: body.x,
                    y: body.y,
                    group_id: body.group_id,
                },
            )?;
            let (_, w, h) = layers::refresh_composite(&ctx.db(), &uid, &id)?;
            Ok(ok(json!({ "layer_id": layer_id, "width": w, "height": h })))
        }
    })
}

/* ── DELETE /api/images/:id/layers/:layer_id ────────────────── */

fn layer_delete(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = path_param(&req, "id")?;
            let layer_id = path_param(&req, "layer_id")?;
            layers::delete_layer(&ctx.db(), &uid, &id, &layer_id)?;
            layers::refresh_composite(&ctx.db(), &uid, &id)?;
            Ok(ok(json!({ "layer_id": layer_id, "deleted": true })))
        }
    })
}

/* ── POST /api/images/:id/layers/:layer_id/image (replace) ──── */

fn layer_image(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = path_param(&req, "id")?;
            let layer_id = path_param(&req, "layer_id")?;
            let mut multipart = Multipart::from_request(req, &())
                .await
                .map_err(|e| AppError::BadRequest(format!("multipart error: {e}")))?;
            let mut file: Option<Vec<u8>> = None;
            while let Some(field) = multipart
                .next_field()
                .await
                .map_err(|e| AppError::BadRequest(format!("multipart error: {e}")))?
            {
                if field.name() == Some("file") {
                    let data = field
                        .bytes()
                        .await
                        .map_err(|e| AppError::BadRequest(format!("read error: {e}")))?;
                    file = Some(data.to_vec());
                }
            }
            let data = file.ok_or_else(|| AppError::BadRequest("missing 'file' field".into()))?;
            let mut img = ops::decode(&data)?;
            ops::fit(&mut img, MAX_DIM);
            let raw = img.get_raw_pixels();
            let w = img.get_width();
            let h = img.get_height();
            let db = ctx.db();
            let existing = layers::active_layer(&db, &uid, &id, Some(&layer_id))?;
            layers::set_layer_pixels(
                &db, &uid, &id, &layer_id, raw.clone(), raw, w, h, existing.x, existing.y,
            )?;
            layers::refresh_composite(&db, &uid, &id)?;
            Ok(ok(json!({ "layer_id": layer_id, "width": w, "height": h })))
        }
    })
}

/* ── GET /api/images/:id/layers/:layer_id/thumb ─────────────── */

fn layer_thumb(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = path_param(&req, "id")?;
            let layer_id = path_param(&req, "layer_id")?;
            let layer = layers::get_layer(&ctx.db(), &uid, &id, &layer_id)?;
            let png = if layer.is_group {
                ops::thumbnail_png(&[0, 0, 0, 0], 1, 1, 44)
            } else {
                ops::thumbnail_png(&layer.bytes, layer.w, layer.h, 44)
            };
            Response::builder()
                .header(CONTENT_TYPE, "image/png")
                .header(CACHE_CONTROL, "no-store")
                .body(axum::body::Body::from(png))
                .map_err(|e| AppError::Internal(format!("failed to build thumb response: {e}")))
        }
    })
}

/* ── POST /api/images/:id/layers/:layer_id/duplicate ────────── */

fn layer_duplicate(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = path_param(&req, "id")?;
            let layer_id = path_param(&req, "layer_id")?;
            let new_id = layers::duplicate_layer(&ctx.db(), &uid, &id, &layer_id)?;
            layers::refresh_composite(&ctx.db(), &uid, &id)?;
            Ok(ok(json!({ "layer_id": new_id, "duplicated_from": layer_id })))
        }
    })
}

/* ── POST /api/images/:id/layers/:layer_id/merge ────────────── */

fn layer_merge(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = path_param(&req, "id")?;
            let layer_id = path_param(&req, "layer_id")?;
            layers::merge_down(&ctx.db(), &uid, &id, &layer_id)?;
            let (_, w, h) = layers::refresh_composite(&ctx.db(), &uid, &id)?;
            Ok(ok(json!({ "merged": layer_id, "width": w, "height": h })))
        }
    })
}

/* ── POST /api/images/:id/layers/reorder ────────────────────── */

#[derive(Deserialize, Default)]
#[serde(default)]
struct Reorder {
    /// New bottom-to-top order of a sibling list.
    ids: Option<Vec<String>>,
    /// Or move one layer before another (both siblings).
    layer_id: Option<String>,
    before_id: Option<String>,
    after_id: Option<String>,
}

fn layer_reorder(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = path_param(&req, "id")?;
            // `reorder` is a literal segment, but axum matches `:layer_id`
            // first for POST /layers/reorder only if no dedicated route — we
            // register this path explicitly, so `id` is the only param.
            let body: Reorder = take_json(req).await?;
            let db = ctx.db();
            let rows = layers::ensure_base_layer(&db, &uid, &id)?;

            let after = body.after_id.is_some();
            let anchor = body.before_id.clone().or_else(|| body.after_id.clone());

            if let Some(ids) = body.ids {
                layers::set_positions(&db, &uid, &id, &ids)?;
            } else if let (Some(layer_id), Some(anchor)) = (body.layer_id.as_ref(), anchor) {
                let parent = rows
                    .iter()
                    .find(|l| l.id == *layer_id)
                    .and_then(|l| l.group_id.clone());
                let siblings: Vec<String> = rows
                    .iter()
                    .filter(|l| l.group_id == parent)
                    .map(|l| l.id.clone())
                    .collect();
                let mut order: Vec<String> = siblings.into_iter().filter(|x| x != layer_id).collect();
                let at = order.iter().position(|x| *x == anchor).unwrap_or(order.len());
                let at = if after { (at + 1).min(order.len()) } else { at };
                order.insert(at, layer_id.clone());
                layers::set_positions(&db, &uid, &id, &order)?;
            } else {
                return Err(AppError::BadRequest("pass `ids` or `layer_id` + `before_id`".into()));
            }
            let (_, w, h) = layers::refresh_composite(&db, &uid, &id)?;
            Ok(ok(json!({ "width": w, "height": h })))
        }
    })
}

/* ── POST /api/images/:id/flatten ───────────────────────────── */

fn image_flatten(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = path_param(&req, "id")?;
            layers::ensure_base_layer(&ctx.db(), &uid, &id)?;
            layers::flatten(&ctx.db(), &uid, &id)?;
            let (_, w, h) = layers::refresh_composite(&ctx.db(), &uid, &id)?;
            Ok(ok(json!({ "image_id": id, "flattened": true, "width": w, "height": h })))
        }
    })
}
