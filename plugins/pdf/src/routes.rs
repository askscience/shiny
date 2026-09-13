//! PDF plugin REST routes — served through the plugin's `RouteSpec`s.
//!
//! DB access goes through the SDK's **synchronous** `ctx.db()` (no async
//! worker threads), so prepare/bind/step/finalize all run on the plugin's
//! single runtime thread.

use std::sync::Arc;

use axum::extract::{FromRequest, FromRequestParts, Multipart};
use axum::http::header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value as Json};

use shiny_plugin_sdk::db::Value;
use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::routes::{bridged_route, path_params_from_request, user_id_from_request, RouteHandler};
use shiny_plugin_sdk::services::PluginCtx;

use crate::ops;

const MIME_PDF: &str = "application/pdf";
const MAX_UPLOAD: usize = 64 * 1024 * 1024;

pub fn handle(ctx: &Arc<PluginCtx>, tag: &str) -> Option<RouteHandler> {
    let ctx = ctx.clone();
    Some(match tag {
        "pdf_list" => pdf_list(ctx),
        "pdf_create" => pdf_create(ctx),
        "pdf_import" => pdf_import(ctx),
        "pdf_get" => pdf_get(ctx),
        "pdf_rename" => pdf_rename(ctx),
        "pdf_delete" => pdf_delete(ctx),
        "pdf_export" => pdf_export(ctx),
        "pdf_file" => pdf_file(ctx),
        "pdf_render" => pdf_render(ctx),
        "pdf_text" => pdf_text(ctx),
        "pdf_text_runs" => pdf_text_runs(ctx),
        "pdf_edit_text" => pdf_edit_text(ctx),
        "pdf_rotate" => pdf_rotate(ctx),
        "pdf_reorder" => pdf_reorder(ctx),
        "pdf_delete_pages" => pdf_delete_pages(ctx),
        "pdf_merge" => pdf_merge(ctx),
        "pdf_replace_text" => pdf_replace_text(ctx),
        "pdf_annotate" => pdf_annotate(ctx),
        "pdf_watermark" => pdf_watermark(ctx),
        _ => return None,
    })
}

/* ── helpers ────────────────────────────────────────────────── */

fn user_id(req: &axum::extract::Request) -> Result<String, AppError> {
    user_id_from_request(req)
        .ok_or_else(|| AppError::Unauthorized("not authenticated".into()))
}

fn ok(data: Json) -> Response {
    axum::Json(json!({ "success": true, "data": data })).into_response()
}

fn clean_title(t: &str) -> String {
    let t = t.trim();
    if t.is_empty() { "Untitled".into() } else { t.chars().take(120).collect() }
}

fn filename_for_title(t: &str) -> String {
    let clean: String = t
        .chars()
        .map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.') { c } else { '-' })
        .collect();
    let clean = clean.trim().trim_matches('.').to_string();
    let clean = if clean.is_empty() { "document".to_string() } else { clean };
    format!("{clean}.pdf")
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

fn as_blob(v: &Value) -> Vec<u8> {
    match v {
        Value::Blob(b) => b.clone(),
        _ => Vec::new(),
    }
}

/// All captured path-param values, in route declaration order.
fn take_paths(req: &axum::extract::Request) -> Result<Vec<String>, AppError> {
    let params = path_params_from_request(req)
        .ok_or_else(|| AppError::BadRequest("no path parameter found".into()))?;
    Ok(params.into_iter().map(|(_, v)| v).collect())
}

fn take_id(req: &axum::extract::Request) -> Result<String, AppError> {
    let p = take_paths(req)?;
    if p.len() != 1 {
        return Err(AppError::BadRequest("expected one path parameter".into()));
    }
    Ok(p.into_iter().next().unwrap())
}

fn take_id_page(req: &axum::extract::Request) -> Result<(String, usize), AppError> {
    let p = take_paths(req)?;
    if p.len() != 2 {
        return Err(AppError::BadRequest("expected :id and :page".into()));
    }
    let page: usize = p[1]
        .parse()
        .map_err(|_| AppError::BadRequest("invalid page number".into()))?;
    Ok((p[0].clone(), page))
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

async fn take_json<T: DeserializeOwned>(
    req: axum::extract::Request,
) -> Result<T, AppError> {
    axum::Json::<T>::from_request(req, &())
        .await
        .map(|j| j.0)
        .map_err(|e| AppError::BadRequest(format!("invalid body: {e}")))
}

fn load_bytes(ctx: &PluginCtx, uid: &str, id: &str) -> Result<Vec<u8>, AppError> {    let rows = ctx.db().query(
        "SELECT bytes FROM pdf_documents WHERE id = ?1 AND user_id = ?2",
        &[Value::text(id), Value::text(uid)],
    )?;
    let row = rows.first().ok_or_else(|| AppError::NotFound("PDF not found".into()))?;
    Ok(as_blob(&row[0]))
}

/// Replace a document's bytes and refresh its cached page count.
fn commit_bytes(ctx: &PluginCtx, uid: &str, id: &str, bytes: Vec<u8>) -> Result<usize, AppError> {
    let count = ops::page_count(&bytes)?;
    let changed = ctx.db().execute(
        "UPDATE pdf_documents SET bytes = ?1, page_count = ?2, updated_at = datetime('now') \
         WHERE id = ?3 AND user_id = ?4",
        &[Value::blob(bytes), Value::Int(count as i64), Value::text(id), Value::text(uid)],
    )?;
    if changed == 0 {
        return Err(AppError::NotFound("PDF not found".into()));
    }
    Ok(count)
}

fn normalize_degrees(degrees: i32) -> Result<i32, AppError> {
    if degrees % 90 != 0 {
        return Err(AppError::BadRequest("degrees must be a multiple of 90".into()));
    }
    Ok(degrees)
}

/* ── GET /api/pdfs ──────────────────────────────────────────── */

fn pdf_list(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let rows = ctx.db().query(
                "SELECT id, title, page_count, updated_at FROM pdf_documents \
                 WHERE user_id = ?1 ORDER BY updated_at DESC LIMIT 200",
                &[Value::text(&uid)],
            )?;
            let docs: Vec<Json> = rows
                .iter()
                .map(|r| json!({
                    "pdf_id": as_text(&r[0]),
                    "title": as_text(&r[1]),
                    "page_count": as_int(&r[2]),
                    "updated_at": as_text(&r[3]),
                }))
                .collect();
            Ok(ok(json!(docs)))
        }
    })
}

/* ── POST /api/pdfs (create) ────────────────────────────────── */

fn pdf_create(ctx: Arc<PluginCtx>) -> RouteHandler {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Create {
        title: Option<String>,
        text: Option<String>,
        css: Option<String>,
        format: Option<String>,
        plain: Option<bool>,
    }

    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let axum::Json(body) = axum::Json::<Create>::from_request(req, &())
                .await
                .map_err(|e| AppError::BadRequest(format!("invalid JSON body: {e}")))?;
            let title = clean_title(body.title.as_deref().unwrap_or("Untitled"));
            let text = body.text.unwrap_or_default();
            let css = body.css.unwrap_or_default();
            let format = body.format.unwrap_or_else(|| {
                if body.plain.unwrap_or(false) { "plain".to_string() } else { "html".to_string() }
            });
            let bytes = ops::create(&text, &css, &format)?;
            let count = ops::page_count(&bytes)?;
            let id = uuid::Uuid::new_v4().to_string();

            ctx.db().execute(
                "INSERT INTO pdf_documents (id, user_id, title, bytes, page_count, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'), datetime('now'))",
                &[Value::text(&id), Value::text(&uid), Value::text(&title), Value::blob(bytes), Value::Int(count as i64)],
            )?;

            Ok(ok(json!({ "pdf_id": id, "title": title, "page_count": count, "updated_at": "now" })))
        }
    })
}

/* ── POST /api/pdfs/import ──────────────────────────────────── */

fn pdf_import(ctx: Arc<PluginCtx>) -> RouteHandler {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct ImportQuery {
        name: Option<String>,
    }

    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let (q, req) = take_query::<ImportQuery>(req).await?;
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
                return Err(AppError::BadRequest("PDF too large (max 64 MB)".into()));
            }

            let count = ops::validate(&data)?;
            let stem = original_name
                .as_deref()
                .map(|n| n.rsplit('.').nth(1).unwrap_or(n))
                .unwrap_or("Imported PDF");
            let title = clean_title(q.name.as_deref().unwrap_or(stem));
            let id = uuid::Uuid::new_v4().to_string();

            ctx.db().execute(
                "INSERT INTO pdf_documents (id, user_id, title, bytes, page_count, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'), datetime('now'))",
                &[Value::text(&id), Value::text(&uid), Value::text(&title), Value::blob(data), Value::Int(count as i64)],
            )?;

            Ok(ok(json!({ "pdf_id": id, "title": title, "page_count": count, "updated_at": "now" })))
        }
    })
}

/* ── GET /api/pdfs/:id ──────────────────────────────────────── */

fn pdf_get(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_id(&req)?;
            let rows = ctx.db().query(
                "SELECT title, page_count, updated_at FROM pdf_documents WHERE id = ?1 AND user_id = ?2",
                &[Value::text(&id), Value::text(&uid)],
            )?;
            let row = rows.first().ok_or_else(|| AppError::NotFound("PDF not found".into()))?;
            Ok(ok(json!({
                "pdf_id": id,
                "title": as_text(&row[0]),
                "page_count": as_int(&row[1]),
                "updated_at": as_text(&row[2]),
            })))
        }
    })
}

/* ── PUT /api/pdfs/:id (rename) ─────────────────────────────── */

fn pdf_rename(ctx: Arc<PluginCtx>) -> RouteHandler {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Rename {
        title: Option<String>,
    }

    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_id(&req)?;
            let axum::Json(body) = axum::Json::<Rename>::from_request(req, &())
                .await
                .map_err(|e| AppError::BadRequest(format!("invalid JSON body: {e}")))?;
            let title = clean_title(body.title.as_deref().unwrap_or("Untitled"));
            let changed = ctx.db().execute(
                "UPDATE pdf_documents SET title = ?1, updated_at = datetime('now') \
                 WHERE id = ?2 AND user_id = ?3",
                &[Value::text(&title), Value::text(&id), Value::text(&uid)],
            )?;
            if changed == 0 {
                return Err(AppError::NotFound("PDF not found".into()));
            }
            Ok(ok(json!({ "pdf_id": id, "title": title })))
        }
    })
}

/* ── DELETE /api/pdfs/:id ───────────────────────────────────── */

fn pdf_delete(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_id(&req)?;
            let changed = ctx.db().execute(
                "DELETE FROM pdf_documents WHERE id = ?1 AND user_id = ?2",
                &[Value::text(&id), Value::text(&uid)],
            )?;
            if changed == 0 {
                return Err(AppError::NotFound("PDF not found".into()));
            }
            Ok(ok(json!({ "pdf_id": id, "deleted": true })))
        }
    })
}

/* ── GET /api/pdfs/:id/export ───────────────────────────────── */

fn pdf_export(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_id(&req)?;
            let rows = ctx.db().query(
                "SELECT title, bytes FROM pdf_documents WHERE id = ?1 AND user_id = ?2",
                &[Value::text(&id), Value::text(&uid)],
            )?;
            let row = rows.first().ok_or_else(|| AppError::NotFound("PDF not found".into()))?;
            let title = as_text(&row[0]);
            let bytes = as_blob(&row[1]);
            let filename = filename_for_title(&title);
            Ok(Response::builder()
                .header(CONTENT_TYPE, MIME_PDF)
                .header(CONTENT_DISPOSITION, format!("attachment; filename=\"{}\"", filename.replace('"', "")))
                .body(axum::body::Body::from(bytes))
                .map_err(|e| AppError::Internal(format!("Export failed: {e}")))?)
        }
    })
}

/* ── GET /api/pdfs/:id/file (raw bytes for the viewer) ──────── */

/// The viewer's data source: the stored PDF exactly as the user imported or
/// last edited it, so the browser can lay out and paint the real document
/// instead of showing a server-rendered bitmap.
///
/// Distinct from `pdf_export`: no `Content-Disposition`, so the response is
/// meant to be consumed in-page rather than downloaded, and `no-store` so a
/// reload always reflects the latest bytes after an edit.
fn pdf_file(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_id(&req)?;
            let bytes = load_bytes(&ctx, &uid, &id)?;
            Ok(Response::builder()
                .header(CONTENT_TYPE, MIME_PDF)
                .header(CACHE_CONTROL, "no-store")
                .body(axum::body::Body::from(bytes))
                .map_err(|e| AppError::Internal(format!("PDF serve failed: {e}")))?)
        }
    })
}

/* ── GET /api/pdfs/:id/pages/:page (render PNG) ─────────────── */

fn pdf_render(ctx: Arc<PluginCtx>) -> RouteHandler {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct RenderQuery {
        dpi: Option<u32>,
    }

    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let (id, page) = take_id_page(&req)?;
            let (q, _req) = take_query::<RenderQuery>(req).await?;
            let dpi = q.dpi.unwrap_or(150).clamp(24, 400);
            let bytes = load_bytes(&ctx, &uid, &id)?;
            if page >= ops::page_count(&bytes)? {
                return Err(AppError::NotFound("page out of range".into()));
            }
            let png = ops::render_png(&bytes, page, dpi)?;
            Ok(Response::builder()
                .header(CONTENT_TYPE, "image/png")
                .header(CACHE_CONTROL, "no-store")
                .body(axum::body::Body::from(png))
                .map_err(|e| AppError::Internal(format!("render failed: {e}")))?)
        }
    })
}

/* ── GET /api/pdfs/:id/text/:page ───────────────────────────── */

fn pdf_text(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let (id, page) = take_id_page(&req)?;
            let bytes = load_bytes(&ctx, &uid, &id)?;
            if page >= ops::page_count(&bytes)? {
                return Err(AppError::NotFound("page out of range".into()));
            }
            let text = ops::extract_text(&bytes, page)?;
            Ok(ok(json!({ "page": page, "text": text })))
        }
    })
}

/* ── GET /api/pdfs/:id/runs/:page (editable text runs) ──────── */

/// Every text run on a page with its geometry and style. The viewer overlays
/// these so the user can click a line and edit it in place.
fn pdf_text_runs(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let (id, page) = take_id_page(&req)?;
            let bytes = load_bytes(&ctx, &uid, &id)?;
            if page >= ops::page_count(&bytes)? {
                return Err(AppError::NotFound("page out of range".into()));
            }
            let runs = ops::page_text_runs(&bytes, page)?;
            Ok(ok(json!({ "page": page, "runs": runs })))
        }
    })
}

/* ── POST /api/pdfs/:id/edit-text ───────────────────────────── */

/// Apply in-place text edits from the viewer, in one transaction.
fn pdf_edit_text(ctx: Arc<PluginCtx>) -> RouteHandler {
    #[derive(Deserialize)]
    struct EditBody {
        page: usize,
        edits: Vec<ops::TextEdit>,
    }

    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_id(&req)?;
            let body: EditBody = take_json(req).await?;
            let bytes = load_bytes(&ctx, &uid, &id)?;
            if body.page >= ops::page_count(&bytes)? {
                return Err(AppError::NotFound("page out of range".into()));
            }
            let (out, applied) = ops::edit_text_runs(&bytes, body.page, &body.edits)?;
            if applied == 0 {
                return Err(AppError::NotFound(
                    "no text run matched the supplied text and position".into(),
                ));
            }
            let count = commit_bytes(&ctx, &uid, &id, out)?;
            Ok(ok(json!({ "applied": applied, "page_count": count })))
        }
    })
}

/* ── POST /api/pdfs/:id/rotate ──────────────────────────────── */

fn pdf_rotate(ctx: Arc<PluginCtx>) -> RouteHandler {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Rotate {
        pages: Option<Vec<usize>>,
        all: Option<bool>,
        degrees: Option<i32>,
    }

    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_id(&req)?;
            let axum::Json(body) = axum::Json::<Rotate>::from_request(req, &())
                .await
                .map_err(|e| AppError::BadRequest(format!("invalid JSON body: {e}")))?;
            let degrees = normalize_degrees(body.degrees.unwrap_or(90))?;
            let bytes = load_bytes(&ctx, &uid, &id)?;
            let all = body.all.unwrap_or(false);
            let list = body.pages.unwrap_or_default();
            let new_bytes = if all || list.is_empty() {
                ops::rotate(&bytes, None, degrees)?
            } else {
                ops::rotate(&bytes, Some(&list), degrees)?
            };
            let count = commit_bytes(&ctx, &uid, &id, new_bytes)?;
            Ok(ok(json!({ "pdf_id": id, "page_count": count, "updated_at": "now" })))
        }
    })
}

/* ── POST /api/pdfs/:id/reorder ─────────────────────────────── */

fn pdf_reorder(ctx: Arc<PluginCtx>) -> RouteHandler {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Reorder {
        order: Option<Vec<usize>>,
    }

    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_id(&req)?;
            let axum::Json(body) = axum::Json::<Reorder>::from_request(req, &())
                .await
                .map_err(|e| AppError::BadRequest(format!("invalid JSON body: {e}")))?;
            let order = body.order.ok_or_else(|| AppError::BadRequest("order required".into()))?;
            if order.is_empty() {
                return Err(AppError::BadRequest("order must not be empty".into()));
            }
            let bytes = load_bytes(&ctx, &uid, &id)?;
            let new_bytes = ops::select_pages(&bytes, &order)?;
            let count = commit_bytes(&ctx, &uid, &id, new_bytes)?;
            Ok(ok(json!({ "pdf_id": id, "page_count": count, "updated_at": "now" })))
        }
    })
}

/* ── POST /api/pdfs/:id/delete-pages ────────────────────────── */

fn pdf_delete_pages(ctx: Arc<PluginCtx>) -> RouteHandler {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct DeletePages {
        pages: Option<Vec<usize>>,
    }

    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_id(&req)?;
            let axum::Json(body) = axum::Json::<DeletePages>::from_request(req, &())
                .await
                .map_err(|e| AppError::BadRequest(format!("invalid JSON body: {e}")))?;
            let pages = body.pages.ok_or_else(|| AppError::BadRequest("pages required".into()))?;
            if pages.is_empty() {
                return Err(AppError::BadRequest("pages must not be empty".into()));
            }
            let bytes = load_bytes(&ctx, &uid, &id)?;
            let new_bytes = ops::delete_pages(&bytes, &pages)?;
            let count = commit_bytes(&ctx, &uid, &id, new_bytes)?;
            Ok(ok(json!({ "pdf_id": id, "page_count": count, "updated_at": "now" })))
        }
    })
}

/* ── POST /api/pdfs/:id/merge ───────────────────────────────── */

fn pdf_merge(ctx: Arc<PluginCtx>) -> RouteHandler {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Merge {
        other_id: Option<String>,
    }

    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_id(&req)?;
            let axum::Json(body) = axum::Json::<Merge>::from_request(req, &())
                .await
                .map_err(|e| AppError::BadRequest(format!("invalid JSON body: {e}")))?;
            let other_id = body.other_id.ok_or_else(|| AppError::BadRequest("other_id required".into()))?;
            if other_id == id {
                return Err(AppError::BadRequest("cannot merge a PDF with itself".into()));
            }
            let bytes = load_bytes(&ctx, &uid, &id)?;
            let other = load_bytes(&ctx, &uid, &other_id)?;
            let new_bytes = ops::merge(&bytes, &other)?;
            let count = commit_bytes(&ctx, &uid, &id, new_bytes)?;
            Ok(ok(json!({ "pdf_id": id, "page_count": count, "updated_at": "now" })))
        }
    })
}

/* ── POST /api/pdfs/:id/replace-text ────────────────────────── */

fn pdf_replace_text(ctx: Arc<PluginCtx>) -> RouteHandler {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct ReplaceText {
        page: Option<usize>,
        old: Option<String>,
        new: Option<String>,
    }

    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_id(&req)?;
            let axum::Json(body) = axum::Json::<ReplaceText>::from_request(req, &())
                .await
                .map_err(|e| AppError::BadRequest(format!("invalid JSON body: {e}")))?;
            let page = body.page.unwrap_or(0);
            let old = body.old.ok_or_else(|| AppError::BadRequest("old required".into()))?;
            let new = body.new.unwrap_or_default();
            let bytes = load_bytes(&ctx, &uid, &id)?;
            // Same content-stream engine the agent's pdf_replace_text uses.
            let (new_bytes, report) = crate::stream_edit::replace_text(&bytes, page, &old, &new)?;
            let touched = report.replaced;
            let count = commit_bytes(&ctx, &uid, &id, new_bytes)?;
            Ok(ok(json!({ "pdf_id": id, "page": page, "replaced": touched, "page_count": count, "updated_at": "now" })))
        }
    })
}

/* ── POST /api/pdfs/:id/annotate ────────────────────────────── */

fn pdf_annotate(ctx: Arc<PluginCtx>) -> RouteHandler {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Annotate {
        page: Option<usize>,
        kind: Option<String>,
        rect: Option<[f32; 4]>,
        text: Option<String>,
        color: Option<[f32; 3]>,
        /// Formatting for added text (`free_text` only).
        #[serde(default)]
        size: Option<f32>,
        #[serde(default)]
        bold: Option<bool>,
        #[serde(default)]
        italic: Option<bool>,
        /// `#rrggbb` for added text.
        #[serde(default)]
        text_color: Option<String>,
    }

    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_id(&req)?;
            let axum::Json(body) = axum::Json::<Annotate>::from_request(req, &())
                .await
                .map_err(|e| AppError::BadRequest(format!("invalid JSON body: {e}")))?;
            let page = body.page.unwrap_or(0);
            let kind = body.kind.ok_or_else(|| AppError::BadRequest("kind required".into()))?;
            let rect = body.rect.ok_or_else(|| AppError::BadRequest("rect required".into()))?;
            let text = body.text.unwrap_or_default();
            // Added text is the one kind that needs a body; an empty one would
            // append a content stream that draws nothing and still grow the file.
            if kind.eq_ignore_ascii_case("free_text") && text.trim().is_empty() {
                return Err(AppError::BadRequest("text must not be empty".into()));
            }
            // Reject rather than clamp: silently substituting a different size
            // than the caller asked for is how a "success" becomes a lie.
            let size = body.size.unwrap_or(11.0);
            if !(4.0..=144.0).contains(&size) {
                return Err(AppError::BadRequest(
                    "size must be between 4 and 144 points".into(),
                ));
            }
            if let Some(c) = body.text_color.as_deref() {
                if ops::parse_text_color(c).is_none() {
                    return Err(AppError::BadRequest(
                        "text_color must be #rrggbb".into(),
                    ));
                }
            }
            let format = ops::TextFormat {
                size,
                bold: body.bold.unwrap_or(false),
                italic: body.italic.unwrap_or(false),
                color: body.text_color,
            };
            let bytes = load_bytes(&ctx, &uid, &id)?;
            let new_bytes = ops::annotate(&bytes, page, &kind, rect, &text, body.color, &format)?;
            let count = commit_bytes(&ctx, &uid, &id, new_bytes)?;
            Ok(ok(json!({ "pdf_id": id, "page": page, "kind": kind, "page_count": count, "updated_at": "now" })))
        }
    })
}

/* ── POST /api/pdfs/:id/watermark ───────────────────────────── */

fn pdf_watermark(ctx: Arc<PluginCtx>) -> RouteHandler {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Watermark {
        text: Option<String>,
        pages: Option<Vec<usize>>,
        all: Option<bool>,
    }

    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_id(&req)?;
            let axum::Json(body) = axum::Json::<Watermark>::from_request(req, &())
                .await
                .map_err(|e| AppError::BadRequest(format!("invalid JSON body: {e}")))?;
            let text = body.text.ok_or_else(|| AppError::BadRequest("text required".into()))?;
            let bytes = load_bytes(&ctx, &uid, &id)?;
            let pages = if body.all.unwrap_or(false) { None } else { body.pages.clone() };
            let new_bytes = ops::add_watermark(&bytes, pages.as_deref(), &text)?;
            let count = commit_bytes(&ctx, &uid, &id, new_bytes)?;
            Ok(ok(json!({ "pdf_id": id, "page_count": count, "updated_at": "now" })))
        }
    })
}
