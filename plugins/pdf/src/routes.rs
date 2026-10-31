//! PDF plugin REST routes — served through the plugin's `RouteSpec`s.
//!
//! DB access goes through the SDK's **synchronous** `ctx.db()` (no async
//! worker threads), so prepare/bind/step/finalize all run on the plugin's
//! single runtime thread.

use std::sync::Arc;

use axum::extract::{FromRequest, FromRequestParts, Multipart};
use axum::http::header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::response::Response;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value as Json};

use shiny_plugin_sdk::db::Value;
use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::util::{as_text, clean_title, filename_for_title};
use shiny_plugin_sdk::routes::{bridged_route, path_params_from_request, RouteHandler, ok, user_id};
use shiny_plugin_sdk::services::PluginCtx;

use crate::ops;
use crate::store;

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
        "pdf_undo" => pdf_history_step(ctx, true),
        "pdf_redo" => pdf_history_step(ctx, false),
        "pdf_insert_blank" => pdf_insert_blank(ctx),
        "pdf_duplicate_page" => pdf_duplicate_page(ctx),
        "pdf_extract" => pdf_extract(ctx),
        "pdf_preview" => pdf_preview(ctx),
        "pdf_set_source" => pdf_set_source(ctx),
        "pdf_comments" => pdf_comments(ctx),
        "pdf_comment_edit" => pdf_comment_edit(ctx),
        "pdf_ink" => pdf_ink(ctx),
        "pdf_header_footer" => pdf_header_footer(ctx),
        "pdf_marks" => pdf_marks(ctx),
        "pdf_marks_remove" => pdf_marks_remove(ctx),
        "pdf_replace_file" => pdf_replace_file(ctx),
        "pdf_render_html" => pdf_render_html(ctx),
        _ => return None,
    })
}

/* ── helpers ────────────────────────────────────────────────── */
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

fn load_bytes(ctx: &PluginCtx, uid: &str, id: &str) -> Result<Vec<u8>, AppError> {
    let rows = ctx.db().query(
        "SELECT bytes FROM pdf_documents WHERE id = ?1 AND user_id = ?2",
        &[Value::text(id), Value::text(uid)],
    )?;
    let row = rows.first().ok_or_else(|| AppError::NotFound("PDF not found".into()))?;
    Ok(as_blob(&row[0]))
}

/// Load the bytes and the cached page count together. Per-page routes used to
/// re-parse the whole PDF just to validate the page number.
fn load_bytes_and_count(
    ctx: &PluginCtx,
    uid: &str,
    id: &str,
) -> Result<(Vec<u8>, usize), AppError> {
    let rows = ctx.db().query(
        "SELECT bytes, page_count FROM pdf_documents WHERE id = ?1 AND user_id = ?2",
        &[Value::text(id), Value::text(uid)],
    )?;
    let row = rows.first().ok_or_else(|| AppError::NotFound("PDF not found".into()))?;
    Ok((as_blob(&row[0]), as_int(&row[1]) as usize))
}

/// Run a CPU-heavy PDF operation off the plugin's serial async worker. The
/// closure only receives owned bytes — never `ctx.db()` or a SQLite handle.
async fn blocking<T, F>(f: F) -> Result<T, AppError>
where
    F: FnOnce() -> Result<T, AppError> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| AppError::Internal(format!("PDF worker task failed: {e}")))?
}

fn commit_bytes_with_count(
    ctx: &PluginCtx,
    uid: &str,
    id: &str,
    bytes: Vec<u8>,
    count: usize,
    label: &str,
) -> Result<usize, AppError> {
    store::commit(ctx, uid, id, bytes, count, label)
}

/// The standard reply after an edit: new page count, real timestamp and the
/// undo/redo state, so the window never has to guess.
fn edited(ctx: &PluginCtx, uid: &str, id: &str, count: usize, extra: Json) -> Result<Json, AppError> {
    let mut v = json!({
        "pdf_id": id,
        "page_count": count,
        "updated_at": store::updated_at(ctx, uid, id),
        "history": store::history(ctx, uid, id)?,
    });
    if let (Some(o), Json::Object(e)) = (v.as_object_mut(), extra) {
        o.extend(e);
    }
    Ok(v)
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
                "SELECT id, title, page_count, updated_at, source_format, pdf_stale FROM pdf_documents \
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
                    "editable": !as_text(&r[4]).is_empty(),
                    "pdf_stale": as_int(&r[5]) != 0,
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
        /// The documented field. `text` is accepted for older callers.
        content: Option<String>,
        text: Option<String>,
        css: Option<String>,
        format: Option<String>,
        plain: Option<bool>,
        page_numbers: Option<bool>,
    }

    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let axum::Json(body) = axum::Json::<Create>::from_request(req, &())
                .await
                .map_err(|e| AppError::BadRequest(format!("invalid JSON body: {e}")))?;
            let title = clean_title(body.title.as_deref().unwrap_or("Untitled"));
            let text = body.content.or(body.text).unwrap_or_default();
            let _css = body.css;
            let format = body.format.unwrap_or_else(|| {
                if body.plain.unwrap_or(false) { "plain".to_string() } else { "auto".to_string() }
            });
            let page_numbers = body.page_numbers.unwrap_or(false);
            let opts = ops::CreateOptions { title: Some(title.clone()), page_numbers };
            let source = text.clone();
            let source_format = format.clone();
            // An editor document's PDF is printed by the shell: start from a
            // blank page and mark it out of date until the first export.
            let stale = format == "html";
            let (bytes, count) = blocking(move || {
                let bytes = if stale { ops::create_with("", "plain", &opts)? } else { ops::create_with(&text, &format, &opts)? };
                let count = ops::page_count(&bytes)?;
                Ok::<_, AppError>((bytes, count))
            })
            .await?;
            let id = uuid::Uuid::new_v4().to_string();

            ctx.db().execute(
                "INSERT INTO pdf_documents (id, user_id, title, bytes, page_count, source, source_format, page_numbers, pdf_stale, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, datetime('now'), datetime('now'))",
                &[
                    Value::text(&id), Value::text(&uid), Value::text(&title), Value::blob(bytes), Value::Int(count as i64),
                    Value::text(&source), Value::text(&source_format), Value::Int(page_numbers as i64), Value::Int(stale as i64),
                ],
            )?;

            let updated_at = store::updated_at(&ctx, &uid, &id);
            Ok(ok(json!({ "pdf_id": id, "title": title, "page_count": count, "updated_at": updated_at })))
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
                    bytes = Some(shiny_plugin_sdk::field_bytes_capped(field, MAX_UPLOAD).await?);
                }
            }
            let data = bytes.ok_or_else(|| AppError::BadRequest("missing 'file' field".into()))?;

            let (data, count) = blocking(move || {
                let count = ops::validate(&data)?;
                Ok::<_, AppError>((data, count))
            })
            .await?;
            let stem = original_name
                .as_deref()
                .map(|n| n.rsplit(['/', '\\']).next().unwrap_or(n))
                .map(|n| match n.rsplit_once('.') {
                    Some((stem, ext)) if ext.eq_ignore_ascii_case("pdf") && !stem.is_empty() => stem,
                    _ => n,
                })
                .unwrap_or("Imported PDF");
            let title = clean_title(q.name.as_deref().unwrap_or(stem));
            let id = uuid::Uuid::new_v4().to_string();

            ctx.db().execute(
                "INSERT INTO pdf_documents (id, user_id, title, bytes, page_count, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'), datetime('now'))",
                &[Value::text(&id), Value::text(&uid), Value::text(&title), Value::blob(data), Value::Int(count as i64)],
            )?;

            let updated_at = store::updated_at(&ctx, &uid, &id);
            Ok(ok(json!({ "pdf_id": id, "title": title, "page_count": count, "updated_at": updated_at })))
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
                "SELECT title, page_count, updated_at, source, source_format, page_numbers, pdf_stale FROM pdf_documents WHERE id = ?1 AND user_id = ?2",
                &[Value::text(&id), Value::text(&uid)],
            )?;
            let row = rows.first().ok_or_else(|| AppError::NotFound("PDF not found".into()))?;
            let source = match &row[3] {
                Value::Text(t) => Some(t.clone()),
                _ => None,
            };
            Ok(ok(json!({
                "pdf_id": id,
                "title": as_text(&row[0]),
                "page_count": as_int(&row[1]),
                "updated_at": as_text(&row[2]),
                "source": source,
                "source_format": as_text(&row[4]),
                "page_numbers": as_int(&row[5]) != 0,
                "pdf_stale": as_int(&row[6]) != 0,
                // What the document editor opens: its own HTML, or the
                // assistant's Markdown turned into editable HTML.
                "editor_html": match (&row[3], as_text(&row[4]).as_str()) {
                    (Value::Text(src), "html") => Some(src.clone()),
                    (Value::Text(src), fmt) => Some(crate::compose::to_editor_html(src, fmt)),
                    _ => None,
                },
                "history": store::history(&ctx, &uid, &id)?,
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
            store::forget(&ctx, &uid, &id)?;
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
            let filename = filename_for_title(&title, "pdf", "document");
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

/* ── GET /api/pdfs/:id/text/:page ───────────────────────────── */

fn pdf_text(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let (id, page) = take_id_page(&req)?;
            let (bytes, count) = load_bytes_and_count(&ctx, &uid, &id)?;
            if page >= count {
                return Err(AppError::NotFound("page out of range".into()));
            }
            let text = blocking(move || ops::extract_text(&bytes, page)).await?;
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
            let (bytes, count) = load_bytes_and_count(&ctx, &uid, &id)?;
            if page >= count {
                return Err(AppError::NotFound("page out of range".into()));
            }
            let runs = blocking(move || ops::page_text_runs(&bytes, page)).await?;
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
            let (bytes, page_count) = load_bytes_and_count(&ctx, &uid, &id)?;
            if body.page >= page_count {
                return Err(AppError::NotFound("page out of range".into()));
            }
            let (out, applied, count) = blocking(move || {
                let (out, applied) = ops::edit_text_runs(&bytes, body.page, &body.edits)?;
                let count = if applied > 0 { ops::page_count(&out)? } else { 0 };
                Ok::<_, AppError>((out, applied, count))
            })
            .await?;
            if applied == 0 {
                return Err(AppError::NotFound(
                    "no text run matched the supplied text and position".into(),
                ));
            }
            let count = commit_bytes_with_count(&ctx, &uid, &id, out, count, "Edit text")?;
            Ok(ok(edited(&ctx, &uid, &id, count, json!({ "applied": applied }))?))
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
            let (new_bytes, count) = blocking(move || {
                let new_bytes = if all || list.is_empty() {
                    ops::rotate(&bytes, None, degrees)?
                } else {
                    ops::rotate(&bytes, Some(&list), degrees)?
                };
                let count = ops::page_count(&new_bytes)?;
                Ok::<_, AppError>((new_bytes, count))
            })
            .await?;
            let count = commit_bytes_with_count(&ctx, &uid, &id, new_bytes, count, "Rotate")?;
            Ok(ok(edited(&ctx, &uid, &id, count, json!({}))?))
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
            let (new_bytes, count) = blocking(move || {
                let new_bytes = ops::select_pages(&bytes, &order)?;
                let count = ops::page_count(&new_bytes)?;
                Ok::<_, AppError>((new_bytes, count))
            })
            .await?;
            let count = commit_bytes_with_count(&ctx, &uid, &id, new_bytes, count, "Reorder pages")?;
            Ok(ok(edited(&ctx, &uid, &id, count, json!({}))?))
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
            let (new_bytes, count) = blocking(move || {
                let new_bytes = ops::delete_pages(&bytes, &pages)?;
                let count = ops::page_count(&new_bytes)?;
                Ok::<_, AppError>((new_bytes, count))
            })
            .await?;
            let count = commit_bytes_with_count(&ctx, &uid, &id, new_bytes, count, "Delete pages")?;
            Ok(ok(edited(&ctx, &uid, &id, count, json!({}))?))
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
            let (new_bytes, count) = blocking(move || {
                let new_bytes = ops::merge(&bytes, &other)?;
                let count = ops::page_count(&new_bytes)?;
                Ok::<_, AppError>((new_bytes, count))
            })
            .await?;
            let count = commit_bytes_with_count(&ctx, &uid, &id, new_bytes, count, "Merge")?;
            Ok(ok(edited(&ctx, &uid, &id, count, json!({}))?))
        }
    })
}

/* ── POST /api/pdfs/:id/replace-text ────────────────────────── */

fn pdf_replace_text(ctx: Arc<PluginCtx>) -> RouteHandler {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct ReplaceText {
        page: Option<usize>,
        /// Replace on every page (the default when `page` is absent).
        all_pages: Option<bool>,
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
            let old = body.old.filter(|o| !o.trim().is_empty())
                .ok_or_else(|| AppError::BadRequest("old required".into()))?;
            let new = body.new.unwrap_or_default();
            let all = body.all_pages.unwrap_or(body.page.is_none());
            let page = body.page.unwrap_or(0);
            let (bytes, page_count) = load_bytes_and_count(&ctx, &uid, &id)?;
            if !all && page >= page_count {
                return Err(AppError::NotFound("page out of range".into()));
            }
            let needle = old.clone();
            // Same content-stream engine the agent's pdf_replace_text uses.
            let (new_bytes, report, count) = blocking(move || {
                let (new_bytes, report) = if all {
                    crate::stream_edit::replace_text_pages(&bytes, None, &old, &new)?
                } else {
                    crate::stream_edit::replace_text(&bytes, page, &old, &new)?
                };
                let count = if report.replaced > 0 { ops::page_count(&new_bytes)? } else { 0 };
                Ok::<_, AppError>((new_bytes, report, count))
            })
            .await?;
            if report.replaced == 0 {
                // Nothing changed: say so instead of saving an identical copy.
                return Err(AppError::NotFound(format!("\"{needle}\" was not found")));
            }
            let count = commit_bytes_with_count(&ctx, &uid, &id, new_bytes, count, "Replace text")?;
            Ok(ok(edited(&ctx, &uid, &id, count, json!({
                "page": page, "replaced": report.replaced, "pages_touched": report.pages_touched,
            }))?))
        }
    })
}

/* ── POST /api/pdfs/:id/annotate ────────────────────────────── */

/// Add a comment (PdfCraft annotation). `rects` lets a multi-line text
/// selection become one highlight with one quad per line.
fn pdf_annotate(ctx: Arc<PluginCtx>) -> RouteHandler {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Annotate {
        page: Option<usize>,
        kind: Option<String>,
        rect: Option<[f32; 4]>,
        rects: Option<Vec<[f32; 4]>>,
        text: Option<String>,
        /// 0..1 RGB, or `text_color` as `#rrggbb`.
        color: Option<[f32; 3]>,
        text_color: Option<String>,
        size: Option<f32>,
    }

    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_id(&req)?;
            let body: Annotate = take_json(req).await?;
            let page = body.page.unwrap_or(0);
            let kind = body.kind.ok_or_else(|| AppError::BadRequest("kind required".into()))?;
            let rects: Vec<[f32; 4]> = match (body.rects, body.rect) {
                (Some(r), _) if !r.is_empty() => r.into_iter().take(500).collect(),
                (_, Some(r)) => vec![r],
                _ => return Err(AppError::BadRequest("rect required".into())),
            };
            let text = body.text.unwrap_or_default();
            let size = body.size.unwrap_or(12.0);
            if !(4.0..=144.0).contains(&size) {
                return Err(AppError::BadRequest("size must be between 4 and 144 points".into()));
            }
            let color = match body.text_color.as_deref() {
                Some(c) => Some(ops::parse_text_color(c).ok_or_else(|| AppError::BadRequest("text_color must be #rrggbb".into()))?),
                None => body.color,
            };
            let (bytes, page_count) = load_bytes_and_count(&ctx, &uid, &id)?;
            if page >= page_count {
                return Err(AppError::NotFound("page out of range".into()));
            }
            let kind_c = kind.clone();
            let (new_bytes, count) = blocking(move || {
                let b = ops::annotate(&bytes, page, &kind_c, &rects, &text, color, size, "")?;
                let c = ops::page_count(&b)?;
                Ok::<_, AppError>((b, c))
            })
            .await?;
            let count = commit_bytes_with_count(&ctx, &uid, &id, new_bytes, count, "Annotate")?;
            Ok(ok(edited(&ctx, &uid, &id, count, json!({ "page": page, "kind": kind }))?))
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
            let text = body.text.filter(|t| !t.trim().is_empty())
                .ok_or_else(|| AppError::BadRequest("text required".into()))?;
            let bytes = load_bytes(&ctx, &uid, &id)?;
            let pages = if body.all.unwrap_or(false) { None } else { body.pages.clone() };
            let (new_bytes, count) = blocking(move || {
                let new_bytes = ops::add_watermark(&bytes, pages.as_deref(), &text)?;
                let count = ops::page_count(&new_bytes)?;
                Ok::<_, AppError>((new_bytes, count))
            })
            .await?;
            let count = commit_bytes_with_count(&ctx, &uid, &id, new_bytes, count, "Watermark")?;
            Ok(ok(edited(&ctx, &uid, &id, count, json!({}))?))
        }
    })
}

/* ── POST /api/pdfs/:id/undo · /redo ────────────────────────── */

fn pdf_history_step(ctx: Arc<PluginCtx>, undo: bool) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_id(&req)?;
            let (count, label) = if undo {
                store::undo(&ctx, &uid, &id)?
            } else {
                store::redo(&ctx, &uid, &id)?
            };
            Ok(ok(edited(&ctx, &uid, &id, count, json!({ "label": label }))?))
        }
    })
}

/* ── POST /api/pdfs/:id/insert-blank ────────────────────────── */

fn pdf_insert_blank(ctx: Arc<PluginCtx>) -> RouteHandler {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Insert {
        /// Insert after this page (0-based); absent = at the start.
        after: Option<usize>,
        count: Option<usize>,
    }

    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_id(&req)?;
            let body: Insert = take_json(req).await?;
            let bytes = load_bytes(&ctx, &uid, &id)?;
            let (new_bytes, count) = blocking(move || {
                let b = ops::insert_blank(&bytes, body.after, body.count.unwrap_or(1))?;
                let c = ops::page_count(&b)?;
                Ok::<_, AppError>((b, c))
            })
            .await?;
            let count = commit_bytes_with_count(&ctx, &uid, &id, new_bytes, count, "Insert page")?;
            Ok(ok(edited(&ctx, &uid, &id, count, json!({}))?))
        }
    })
}

/* ── POST /api/pdfs/:id/duplicate-page ──────────────────────── */

fn pdf_duplicate_page(ctx: Arc<PluginCtx>) -> RouteHandler {
    #[derive(Deserialize)]
    struct Dup {
        page: usize,
    }

    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_id(&req)?;
            let body: Dup = take_json(req).await?;
            let bytes = load_bytes(&ctx, &uid, &id)?;
            let (new_bytes, count) = blocking(move || {
                let b = ops::duplicate_page(&bytes, body.page)?;
                let c = ops::page_count(&b)?;
                Ok::<_, AppError>((b, c))
            })
            .await?;
            let count = commit_bytes_with_count(&ctx, &uid, &id, new_bytes, count, "Duplicate page")?;
            Ok(ok(edited(&ctx, &uid, &id, count, json!({}))?))
        }
    })
}

/* ── POST /api/pdfs/:id/extract (pages → new PDF) ───────────── */

fn pdf_extract(ctx: Arc<PluginCtx>) -> RouteHandler {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Extract {
        pages: Vec<usize>,
        title: Option<String>,
    }

    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_id(&req)?;
            let body: Extract = take_json(req).await?;
            if body.pages.is_empty() {
                return Err(AppError::BadRequest("pages must not be empty".into()));
            }
            let rows = ctx.db().query(
                "SELECT title, bytes FROM pdf_documents WHERE id = ?1 AND user_id = ?2",
                &[Value::text(&id), Value::text(&uid)],
            )?;
            let row = rows.first().ok_or_else(|| AppError::NotFound("PDF not found".into()))?;
            let src_title = as_text(&row[0]);
            let bytes = as_blob(&row[1]);
            let pages = body.pages.clone();
            let (out, count) = blocking(move || {
                let b = ops::extract_pages(&bytes, &pages)?;
                let c = ops::page_count(&b)?;
                Ok::<_, AppError>((b, c))
            })
            .await?;
            let title = clean_title(body.title.as_deref().unwrap_or(&format!("{src_title} (extract)")));
            let new_id = uuid::Uuid::new_v4().to_string();
            ctx.db().execute(
                "INSERT INTO pdf_documents (id, user_id, title, bytes, page_count, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'), datetime('now'))",
                &[Value::text(&new_id), Value::text(&uid), Value::text(&title), Value::blob(out), Value::Int(count as i64)],
            )?;
            let updated_at = store::updated_at(&ctx, &uid, &new_id);
            Ok(ok(json!({ "pdf_id": new_id, "title": title, "page_count": count, "updated_at": updated_at })))
        }
    })
}

/* ── POST /api/pdfs/preview (render without saving) ─────────── */

/// The composer's live preview: the same creation engine, returning the PDF
/// bytes inline instead of storing a document.
fn pdf_preview(_ctx: Arc<PluginCtx>) -> RouteHandler {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Preview {
        title: Option<String>,
        content: Option<String>,
        format: Option<String>,
        page_numbers: Option<bool>,
    }

    bridged_route(move |req: axum::extract::Request| async move {
        let _uid = user_id(&req)?;
        let body: Preview = take_json(req).await?;
        let content = body.content.unwrap_or_default();
        if content.len() > 2 * 1024 * 1024 {
            return Err(AppError::BadRequest("content too large".into()));
        }
        let format = body.format.unwrap_or_else(|| "markdown".into());
        let opts = ops::CreateOptions { title: body.title, page_numbers: body.page_numbers.unwrap_or(false) };
        let bytes = blocking(move || ops::create_with(&content, &format, &opts)).await?;
        Ok(Response::builder()
            .header(CONTENT_TYPE, MIME_PDF)
            .header(CACHE_CONTROL, "no-store")
            .body(axum::body::Body::from(bytes))
            .map_err(|e| AppError::Internal(format!("preview failed: {e}")))?)
    })
}

/* ── PUT /api/pdfs/:id/source (regenerate from text) ────────── */

/// Re-create a document from its (edited) source text. An ordinary edit:
/// the previous bytes stay one undo away.
fn pdf_set_source(ctx: Arc<PluginCtx>) -> RouteHandler {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Source {
        title: Option<String>,
        content: Option<String>,
        format: Option<String>,
        page_numbers: Option<bool>,
    }

    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_id(&req)?;
            let body: Source = take_json(req).await?;
            let rows = ctx.db().query(
                "SELECT title FROM pdf_documents WHERE id = ?1 AND user_id = ?2",
                &[Value::text(&id), Value::text(&uid)],
            )?;
            let row = rows.first().ok_or_else(|| AppError::NotFound("PDF not found".into()))?;
            let title = clean_title(body.title.as_deref().unwrap_or(&as_text(&row[0])));
            let content = body.content.unwrap_or_default();
            let format = body.format.unwrap_or_else(|| "markdown".into());
            let page_numbers = body.page_numbers.unwrap_or(false);
            if format == "html" {
                // The editor's document: only the shell can print it, so the
                // text is saved and the PDF marked out of date until export.
                if content.len() > 32 * 1024 * 1024 {
                    return Err(AppError::BadRequest("document too large".into()));
                }
                ctx.db().execute(
                    "UPDATE pdf_documents SET title = ?1, source = ?2, source_format = 'html', pdf_stale = 1, updated_at = datetime('now') \
                     WHERE id = ?3 AND user_id = ?4",
                    &[Value::text(&title), Value::text(&content), Value::text(&id), Value::text(&uid)],
                )?;
                return Ok(ok(json!({ "pdf_id": id, "title": title, "pdf_stale": true, "updated_at": store::updated_at(&ctx, &uid, &id) })));
            }
            let opts = ops::CreateOptions { title: Some(title.clone()), page_numbers };
            let (src, fmt) = (content.clone(), format.clone());
            let (bytes, count) = blocking(move || {
                let b = ops::create_with(&content, &format, &opts)?;
                let c = ops::page_count(&b)?;
                Ok::<_, AppError>((b, c))
            })
            .await?;
            let count = commit_bytes_with_count(&ctx, &uid, &id, bytes, count, "Edit content")?;
            ctx.db().execute(
                "UPDATE pdf_documents SET title = ?1, source = ?2, source_format = ?3, page_numbers = ?4 WHERE id = ?5 AND user_id = ?6",
                &[Value::text(&title), Value::text(&src), Value::text(&fmt), Value::Int(page_numbers as i64), Value::text(&id), Value::text(&uid)],
            )?;
            Ok(ok(edited(&ctx, &uid, &id, count, json!({ "title": title }))?))
        }
    })
}

/* ── Shared: run a bytes → bytes edit and commit it ─────────── */

async fn apply_edit<F>(ctx: &PluginCtx, uid: &str, id: &str, label: &str, extra: Json, f: F) -> Result<Json, AppError>
where
    F: FnOnce(Vec<u8>) -> Result<Vec<u8>, AppError> + Send + 'static,
{
    let bytes = load_bytes(ctx, uid, id)?;
    let (new_bytes, count) = blocking(move || {
        let b = f(bytes)?;
        let c = ops::page_count(&b)?;
        Ok::<_, AppError>((b, c))
    })
    .await?;
    let count = commit_bytes_with_count(ctx, uid, id, new_bytes, count, label)?;
    edited(ctx, uid, id, count, extra)
}

/* ── GET /api/pdfs/:id/comments ─────────────────────────────── */

fn pdf_comments(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_id(&req)?;
            let bytes = load_bytes(&ctx, &uid, &id)?;
            let (list, marks) = blocking(move || Ok::<_, AppError>((crate::craft::comments(&bytes)?, crate::craft::marks(&bytes)?))).await?;
            Ok(ok(json!({ "comments": list, "marks": marks })))
        }
    })
}

/* ── POST /api/pdfs/:id/comments/edit ───────────────────────── */

/// One endpoint for what a comment can go through: `delete`, `text` (change
/// its note), `move` (by `dx`, `dy` points) and `reply`.
fn pdf_comment_edit(ctx: Arc<PluginCtx>) -> RouteHandler {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct CommentEdit {
        page: usize,
        index: usize,
        action: String,
        text: Option<String>,
        dx: Option<f64>,
        dy: Option<f64>,
    }

    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_id(&req)?;
            let b: CommentEdit = take_json(req).await?;
            let (page, index) = (b.page, b.index);
            let text = b.text.unwrap_or_default();
            let (label, action) = match b.action.as_str() {
                "delete" => ("Delete comment", 0),
                "text" => ("Edit comment", 1),
                "move" => ("Move comment", 2),
                "reply" => ("Reply", 3),
                other => return Err(AppError::BadRequest(format!("unknown action: {other}"))),
            };
            let (dx, dy) = (b.dx.unwrap_or(0.0), b.dy.unwrap_or(0.0));
            let res = apply_edit(&ctx, &uid, &id, label, json!({}), move |bytes| match action {
                0 => crate::craft::delete_comment(&bytes, page, index),
                1 => crate::craft::set_comment_text(&bytes, page, index, &text),
                2 => crate::craft::move_comment(&bytes, page, index, dx, dy),
                _ => crate::craft::reply(&bytes, page, index, &text, ""),
            })
            .await?;
            Ok(ok(res))
        }
    })
}

/* ── POST /api/pdfs/:id/ink ─────────────────────────────────── */

fn pdf_ink(ctx: Arc<PluginCtx>) -> RouteHandler {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Ink {
        page: usize,
        strokes: Vec<Vec<[f64; 2]>>,
        color: Option<String>,
    }

    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_id(&req)?;
            let b: Ink = take_json(req).await?;
            let color = b.color.as_deref().and_then(ops::parse_text_color);
            let page = b.page;
            let strokes: Vec<Vec<[f64; 2]>> = b.strokes.into_iter().take(200).map(|s| s.into_iter().take(5000).collect()).collect();
            let res = apply_edit(&ctx, &uid, &id, "Draw", json!({ "page": page }), move |bytes| ops::ink(&bytes, page, strokes, color, "")).await?;
            Ok(ok(res))
        }
    })
}

/* ── POST /api/pdfs/:id/header-footer ───────────────────────── */

fn pdf_header_footer(ctx: Arc<PluginCtx>) -> RouteHandler {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Hf {
        /// Header left/centre/right, footer left/centre/right.
        slots: Vec<String>,
        font_size: Option<f64>,
    }

    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_id(&req)?;
            let b: Hf = take_json(req).await?;
            let mut slots: [String; 6] = Default::default();
            for (i, s) in b.slots.into_iter().take(6).enumerate() {
                slots[i] = s;
            }
            let size = b.font_size.unwrap_or(9.0);
            let res = apply_edit(&ctx, &uid, &id, "Header & footer", json!({}), move |bytes| crate::craft::header_footer(&bytes, slots, size)).await?;
            Ok(ok(res))
        }
    })
}

/* ── GET /api/pdfs/:id/marks · POST /marks/remove ───────────── */

fn pdf_marks(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_id(&req)?;
            let bytes = load_bytes(&ctx, &uid, &id)?;
            let marks = blocking(move || crate::craft::marks(&bytes)).await?;
            Ok(ok(json!({ "marks": marks })))
        }
    })
}

fn pdf_marks_remove(ctx: Arc<PluginCtx>) -> RouteHandler {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Remove {
        kind: String,
    }

    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_id(&req)?;
            let b: Remove = take_json(req).await?;
            let kind = b.kind.clone();
            let res = apply_edit(&ctx, &uid, &id, "Remove marks", json!({ "kind": b.kind }), move |bytes| crate::craft::remove_marks(&bytes, &kind)).await?;
            Ok(ok(res))
        }
    })
}

/* ── POST /api/pdfs/:id/replace-file (exported document) ───── */

/// Replace a document's bytes with an uploaded PDF — the document editor's
/// export (the shell prints the pages; the window uploads them here). An
/// ordinary, undoable edit; it also marks the document's PDF as current.
fn pdf_replace_file(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_id(&req)?;
            let mut multipart = Multipart::from_request(req, &())
                .await
                .map_err(|e| AppError::BadRequest(format!("multipart error: {e}")))?;
            let mut bytes: Option<Vec<u8>> = None;
            while let Some(field) = multipart
                .next_field()
                .await
                .map_err(|e| AppError::BadRequest(format!("multipart error: {e}")))?
            {
                if field.name() == Some("file") {
                    bytes = Some(shiny_plugin_sdk::field_bytes_capped(field, MAX_UPLOAD).await?);
                }
            }
            let data = bytes.ok_or_else(|| AppError::BadRequest("missing 'file' field".into()))?;
            let (data, count) = blocking(move || {
                let count = ops::validate(&data)?;
                Ok::<_, AppError>((data, count))
            })
            .await?;
            let count = commit_bytes_with_count(&ctx, &uid, &id, data, count, "Export")?;
            ctx.db().execute(
                "UPDATE pdf_documents SET pdf_stale = 0 WHERE id = ?1 AND user_id = ?2",
                &[Value::text(&id), Value::text(&uid)],
            )?;
            Ok(ok(edited(&ctx, &uid, &id, count, json!({ "pdf_stale": false }))?))
        }
    })
}

/* ── POST /api/pdfs/:id/render-html (export without the shell) ─ */

/// The editor's fallback when the shell cannot print: lay the stored HTML out
/// with the plugin's own composer. Same content, the composer's typography.
fn pdf_render_html(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_id(&req)?;
            let rows = ctx.db().query(
                "SELECT title, source, source_format FROM pdf_documents WHERE id = ?1 AND user_id = ?2",
                &[Value::text(&id), Value::text(&uid)],
            )?;
            let row = rows.first().ok_or_else(|| AppError::NotFound("PDF not found".into()))?;
            let title = as_text(&row[0]);
            let source = match &row[1] {
                Value::Text(s) => s.clone(),
                _ => return Err(AppError::BadRequest("this PDF has no editable source".into())),
            };
            let format = as_text(&row[2]);
            let numbers = source.starts_with("<!--pdfdoc") && source.split("-->").next().map_or(false, |m| m.contains("numbers=1"));
            let opts = ops::CreateOptions { title: Some(title), page_numbers: numbers };
            let (bytes, count) = blocking(move || {
                let b = ops::create_with(&source, if format == "html" { "html" } else { &format }, &opts)?;
                let c = ops::page_count(&b)?;
                Ok::<_, AppError>((b, c))
            })
            .await?;
            let count = commit_bytes_with_count(&ctx, &uid, &id, bytes, count, "Export")?;
            ctx.db().execute(
                "UPDATE pdf_documents SET pdf_stale = 0 WHERE id = ?1 AND user_id = ?2",
                &[Value::text(&id), Value::text(&uid)],
            )?;
            Ok(ok(edited(&ctx, &uid, &id, count, json!({ "pdf_stale": false, "approximate": true }))?))
        }
    })
}
