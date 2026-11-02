//! Word plugin REST routes — served through the plugin's `RouteSpec`s.
//!
//! Content is stored as `.odt`; the window exchanges HTML. Import reads
//! `.docx`/`.odt`/`.rtf`/`.md`/`.html`/`.txt`, export composes the document
//! with its [`Design`] and writes `.docx`/`.pdf`/`.odt`/`.md`/`.html`
//! (see `engine.rs`).
//!
//! DB access goes through the SDK's **synchronous** `ctx.db()` (no async
//! worker threads), so prepare/bind/step/finalize all run on the plugin's
//! single runtime thread.

use std::sync::Arc;

use axum::extract::{FromRequest, FromRequestParts};
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::response::Response;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value as Json};

use shiny_plugin_sdk::db::Value;
use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::util::{as_text, clean_title, filename_for_title};
use shiny_plugin_sdk::routes::{bridged_route, RouteHandler, path_params_from_request, ok, user_id};
use shiny_plugin_sdk::services::PluginCtx;

use std::sync::{Mutex, OnceLock};

use crate::engine::{self, Design, Format, Preview};
/// Largest imported document accepted into memory.
const MAX_UPLOAD: usize = 64 * 1024 * 1024;

pub fn handle(ctx: &Arc<PluginCtx>, tag: &str) -> Option<RouteHandler> {
    let ctx = ctx.clone();
    Some(match tag {
        "doc_list" => doc_list(ctx),
        "doc_create" => doc_create(ctx),
        "doc_get" => doc_get(ctx),
        "doc_save" => doc_save(ctx),
        "doc_delete" => doc_delete(ctx),
        "doc_export" => doc_export(ctx),
        "doc_import" => doc_import(ctx),
        "doc_design" => doc_design(ctx),
        "doc_pages" => doc_pages(ctx),
        "doc_opened" => doc_opened(ctx),
        "doc_page" => doc_page(ctx),
        "doc_font" => doc_font(),
        _ => return None,
    })
}

/* ── helpers ────────────────────────────────────────────────── */
fn as_blob(v: &Value) -> Vec<u8> {
    match v {
        Value::Blob(b) => b.clone(),
        _ => Vec::new(),
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

/* ── handlers ───────────────────────────────────────────────── */

fn doc_list(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let rows = ctx.db().query(
                "SELECT id, title, updated_at FROM documents \
                 WHERE user_id = ?1 ORDER BY updated_at DESC LIMIT 200",
                &[Value::text(&uid)],
            )?;
            let docs: Vec<Json> = rows
                .iter()
                .map(|r| json!({ "id": as_text(&r[0]), "title": as_text(&r[1]), "updated_at": as_text(&r[2]) }))
                .collect();
            Ok(ok(json!(docs)))
        }
    })
}

fn doc_create(ctx: Arc<PluginCtx>) -> RouteHandler {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Create {
        title: Option<String>,
        html: Option<String>,
    }

    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let axum::Json(body) = axum::Json::<Create>::from_request(req, &())
                .await
                .map_err(|e| AppError::BadRequest(format!("invalid JSON body: {e}")))?;
            let title = clean_title(body.title.as_deref().unwrap_or("Untitled"));
            let html = body.html.unwrap_or_default();
            let odt_bytes = engine::html_to_odt(&html)?;
            let id = uuid::Uuid::new_v4().to_string();

            ctx.db().execute(
                "INSERT INTO documents (id, user_id, title, odt, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, datetime('now'), datetime('now'))",
                &[Value::text(&id), Value::text(&uid), Value::text(&title), Value::blob(odt_bytes)],
            )?;

            Ok(ok(json!({ "id": id, "title": title, "html": html, "updated_at": "now" })))
        }
    })
}

fn doc_get(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_path(&req)?;
            let rows = ctx.db().query(
                "SELECT title, odt, updated_at, design FROM documents WHERE id = ?1 AND user_id = ?2",
                &[Value::text(&id), Value::text(&uid)],
            )?;
            let row = rows.first().ok_or_else(|| AppError::NotFound("Document not found".into()))?;
            let title = as_text(&row[0]);
            let html = engine::odt_to_html(&as_blob(&row[1]))?;
            let updated_at = as_text(&row[2]);
            let design = Design::from_json(&as_text(&row[3]));
            Ok(ok(json!({ "id": id, "title": title, "html": html, "updated_at": updated_at, "design": design })))
        }
    })
}

fn doc_save(ctx: Arc<PluginCtx>) -> RouteHandler {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Save {
        title: Option<String>,
        html: Option<String>,
    }

    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_path(&req)?;
            let axum::Json(body) = axum::Json::<Save>::from_request(req, &())
                .await
                .map_err(|e| AppError::BadRequest(format!("invalid JSON body: {e}")))?;
            let html = body.html.unwrap_or_default();

            let current = ctx.db().query(
                "SELECT title FROM documents WHERE id = ?1 AND user_id = ?2",
                &[Value::text(&id), Value::text(&uid)],
            )?;
            let title = match body.title {
                Some(t) => clean_title(&t),
                None => current
                    .first()
                    .map(|r| as_text(&r[0]))
                    .filter(|t| !t.is_empty())
                    .unwrap_or_else(|| "Untitled".into()),
            };
            let odt_bytes = engine::html_to_odt(&html)?;

            let changed = ctx.db().execute(
                "UPDATE documents SET title = ?1, odt = ?2, updated_at = datetime('now') \
                 WHERE id = ?3 AND user_id = ?4",
                &[Value::text(&title), Value::blob(odt_bytes), Value::text(&id), Value::text(&uid)],
            )?;
            if changed == 0 {
                return Err(AppError::NotFound("Document not found".into()));
            }
            Ok(ok(json!({ "success": true })))
        }
    })
}

fn doc_delete(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_path(&req)?;
            let changed = ctx.db().execute(
                "DELETE FROM documents WHERE id = ?1 AND user_id = ?2",
                &[Value::text(&id), Value::text(&uid)],
            )?;
            if changed == 0 {
                return Err(AppError::NotFound("Document not found".into()));
            }
            Ok(ok(json!({ "success": true })))
        }
    })
}

fn doc_export(ctx: Arc<PluginCtx>) -> RouteHandler {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct ExportQuery {
        /// `odt` (default), `docx`, `pdf`, `md`, `html`, `rtf`, `txt`.
        format: Option<String>,
    }

    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_path(&req)?;
            let (q, _req) = take_query::<ExportQuery>(req).await?;
            let format = match q.format.as_deref().filter(|f| !f.trim().is_empty()) {
                None => Format::Odt,
                Some(f) => Format::parse(f).ok_or_else(|| AppError::BadRequest(format!("Unknown export format: {f}")))?,
            };
            let rows = ctx.db().query(
                "SELECT title, odt, design FROM documents WHERE id = ?1 AND user_id = ?2",
                &[Value::text(&id), Value::text(&uid)],
            )?;
            let row = rows.first().ok_or_else(|| AppError::NotFound("Document not found".into()))?;
            let title = as_text(&row[0]);
            let design = Design::from_json(&as_text(&row[2]));
            let bytes = export_document(&title, &as_blob(&row[1]), &design, format)?;
            let filename = filename_for_title(&title, format.ext(), "document");
            Response::builder()
                .header(CONTENT_TYPE, format.mime())
                .header(CONTENT_DISPOSITION, format!("attachment; filename=\"{}\"", filename.replace('"', "")))
                .body(axum::body::Body::from(bytes))
                .map_err(|e| AppError::Internal(format!("Export failed: {e}")))
        }
    })
}

/// The stored content composed with its design, written as `format`. A plain
/// design exported as `.odt` is the stored file itself.
pub fn export_document(title: &str, odt: &[u8], design: &Design, format: Format) -> Result<Vec<u8>, AppError> {
    if format == Format::Odt && design.is_plain() {
        return Ok(odt.to_vec());
    }
    let content = engine::odt_to_doc(odt)?;
    engine::export(&engine::compose(&content, title, design), format)
}

/// The window opened this document: it becomes the agent's default target.
fn doc_opened(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_path(&req)?;
            let changed = ctx.db().execute(
                "UPDATE documents SET opened_at = datetime('now') WHERE id = ?1 AND user_id = ?2",
                &[Value::text(&id), Value::text(&uid)],
            )?;
            if changed == 0 {
                return Err(AppError::NotFound("Document not found".into()));
            }
            Ok(ok(json!({ "success": true })))
        }
    })
}

/* ── Preview ────────────────────────────────────────────────── */

/// Composed and laid-out documents, keyed by id, valid for one
/// (updated_at, design) pair: listing the pages and fetching each one lay
/// the document out once.
type PreviewCache = Mutex<Vec<(String, String, Arc<Preview>)>>;
static PREVIEWS: OnceLock<PreviewCache> = OnceLock::new();
const PREVIEW_CACHE: usize = 4;

fn preview_for(ctx: &PluginCtx, uid: &str, id: &str) -> Result<Arc<Preview>, AppError> {
    let rows = ctx.db().query(
        "SELECT title, odt, design, updated_at FROM documents WHERE id = ?1 AND user_id = ?2",
        &[Value::text(id), Value::text(uid)],
    )?;
    let row = rows.first().ok_or_else(|| AppError::NotFound("Document not found".into()))?;
    let title = as_text(&row[0]);
    let design_json = as_text(&row[2]);
    let key = format!("{}|{}|{}", as_text(&row[3]), title, design_json);
    let cache = PREVIEWS.get_or_init(|| Mutex::new(Vec::new()));
    if let Some(hit) = cache.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|(i, k, _)| i == id && *k == key) {
        return Ok(hit.2.clone());
    }
    let content = engine::odt_to_doc(&as_blob(&row[1]))?;
    let preview = Arc::new(Preview::new(&content, &title, &Design::from_json(&design_json)));
    let mut c = cache.lock().unwrap_or_else(|e| e.into_inner());
    c.retain(|(i, _, _)| i != id);
    c.insert(0, (id.to_string(), key, preview.clone()));
    c.truncate(PREVIEW_CACHE);
    Ok(preview)
}

fn doc_pages(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_path(&req)?;
            let preview = preview_for(&ctx, &uid, &id)?;
            let pages: Vec<Json> = preview.pages().into_iter().map(|(w, h)| json!({ "w": w, "h": h })).collect();
            Ok(ok(json!({ "count": pages.len(), "pages": pages })))
        }
    })
}

fn doc_page(ctx: Arc<PluginCtx>) -> RouteHandler {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct PageQuery {
        scale: Option<f32>,
    }

    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let params = path_params_from_request(&req).ok_or_else(|| AppError::BadRequest("no path parameter found".into()))?;
            let get = |k: &str| params.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
            let id = get("id").ok_or_else(|| AppError::BadRequest("missing document id".into()))?;
            let n: usize = get("n")
                .and_then(|v| v.parse().ok())
                .filter(|n| *n >= 1)
                .ok_or_else(|| AppError::BadRequest("page must be a number from 1".into()))?;
            let (q, _req) = take_query::<PageQuery>(req).await?;
            let preview = preview_for(&ctx, &uid, &id)?;
            let png = preview.page_png(n - 1, q.scale.unwrap_or(1.5))?;
            Response::builder()
                .header(CONTENT_TYPE, "image/png")
                .header("Cache-Control", "private, no-store")
                .body(axum::body::Body::from(png))
                .map_err(|e| AppError::Internal(format!("Preview failed: {e}")))
        }
    })
}

/// The fonts WordCraft bundles (OFL), for the window's page: public, same
/// origin, immutable.
fn doc_font() -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| async move {
        let name = take_path(&req)?;
        let bytes = engine::font_file(&name).ok_or_else(|| AppError::NotFound("No such font".into()))?;
        Response::builder()
            .header(CONTENT_TYPE, "font/ttf")
            .header("Cache-Control", "public, max-age=31536000, immutable")
            .body(axum::body::Body::from(bytes))
            .map_err(|e| AppError::Internal(format!("Font failed: {e}")))
    })
}

fn doc_design(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_path(&req)?;
            let axum::Json(design) = axum::Json::<Design>::from_request(req, &())
                .await
                .map_err(|e| AppError::BadRequest(format!("invalid JSON body: {e}")))?;
            design.validate()?;
            let changed = ctx.db().execute(
                "UPDATE documents SET design = ?1, updated_at = datetime('now') WHERE id = ?2 AND user_id = ?3",
                &[Value::text(design.to_json()), Value::text(&id), Value::text(&uid)],
            )?;
            if changed == 0 {
                return Err(AppError::NotFound("Document not found".into()));
            }
            Ok(ok(json!({ "success": true, "design": design })))
        }
    })
}

fn doc_import(ctx: Arc<PluginCtx>) -> RouteHandler {
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
            let (data, original_name) = shiny_plugin_sdk::read_upload_file(req, MAX_UPLOAD).await?;

            let file_name = original_name.clone().unwrap_or_default();
            let stem = original_name
                .as_deref()
                .map(|n| n.rsplit_once('.').map(|(s, _)| s).unwrap_or(n))
                .filter(|s| !s.trim().is_empty())
                .unwrap_or("Imported document");
            let title = clean_title(q.name.as_deref().unwrap_or(stem));

            // Any readable format becomes .odt content; an .odt is kept as is.
            let doc = engine::import(&file_name, &data)?;
            let is_odt = Format::parse(file_name.rsplit_once('.').map(|(_, e)| e).unwrap_or("")) == Some(Format::Odt);
            let odt_bytes = if is_odt { data } else { engine::doc_to_odt(&doc)? };
            let id = uuid::Uuid::new_v4().to_string();
            ctx.db().execute(
                "INSERT INTO documents (id, user_id, title, odt, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, datetime('now'), datetime('now'))",
                &[Value::text(&id), Value::text(&uid), Value::text(&title), Value::blob(odt_bytes)],
            )?;

            Ok(ok(json!({ "id": id, "title": title })))
        }
    })
}
