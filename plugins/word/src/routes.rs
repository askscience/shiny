//! Word plugin REST routes — served through the plugin's `RouteSpec`s.
//!
//! DB access goes through the SDK's **synchronous** `ctx.db()` (no async
//! worker threads), so prepare/bind/step/finalize all run on the plugin's
//! single runtime thread.

use std::sync::Arc;

use axum::extract::{FromRequest, FromRequestParts, Multipart};
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value as Json};

use shiny_plugin_sdk::db::Value;
use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::odt;
use shiny_plugin_sdk::routes::{bridged_route, RouteHandler, UserId};
use shiny_plugin_sdk::services::PluginCtx;

const MIME_ODT: &str = "application/vnd.oasis.opendocument.text";

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
        _ => return None,
    })
}

/* ── helpers ────────────────────────────────────────────────── */

fn user_id(req: &axum::extract::Request) -> Result<String, AppError> {
    req.extensions()
        .get::<UserId>()
        .map(|u| u.0.clone())
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
    format!("{clean}.odt")
}

fn as_text(v: &Value) -> String {
    match v {
        Value::Text(s) => s.clone(),
        Value::Int(n) => n.to_string(),
        _ => String::new(),
    }
}

fn as_blob(v: &Value) -> Vec<u8> {
    match v {
        Value::Blob(b) => b.clone(),
        _ => Vec::new(),
    }
}

async fn take_path<T: DeserializeOwned + Send + 'static>(
    req: axum::extract::Request,
) -> Result<(T, axum::extract::Request), AppError> {
    let (mut parts, body) = req.into_parts();
    let path = axum::extract::Path::<T>::from_request_parts(&mut parts, &())
        .await
        .map_err(|e| AppError::BadRequest(format!("invalid path: {e}")))?;
    Ok((path.0, axum::extract::Request::from_parts(parts, body)))
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
            let odt_bytes = odt::html_to_odt(&title, &html)?;
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
            let (id, _) = take_path::<String>(req).await?;
            let rows = ctx.db().query(
                "SELECT title, odt, updated_at FROM documents WHERE id = ?1 AND user_id = ?2",
                &[Value::text(&id), Value::text(&uid)],
            )?;
            let row = rows.first().ok_or_else(|| AppError::NotFound("Document not found".into()))?;
            let title = as_text(&row[0]);
            let html = odt::odt_to_html(&as_blob(&row[1]))?;
            let updated_at = as_text(&row[2]);
            Ok(ok(json!({ "id": id, "title": title, "html": html, "updated_at": updated_at })))
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
            let (id, req) = take_path::<String>(req).await?;
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
            let odt_bytes = odt::html_to_odt(&title, &html)?;

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
            let (id, _) = take_path::<String>(req).await?;
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
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let (id, _) = take_path::<String>(req).await?;
            let rows = ctx.db().query(
                "SELECT title, odt FROM documents WHERE id = ?1 AND user_id = ?2",
                &[Value::text(&id), Value::text(&uid)],
            )?;
            let row = rows.first().ok_or_else(|| AppError::NotFound("Document not found".into()))?;
            let title = as_text(&row[0]);
            let bytes = as_blob(&row[1]);
            let filename = filename_for_title(&title);
            Ok(Response::builder()
                .header(CONTENT_TYPE, MIME_ODT)
                .header(CONTENT_DISPOSITION, format!("attachment; filename=\"{}\"", filename.replace('"', "")))
                .body(axum::body::Body::from(bytes))
                .map_err(|e| AppError::Internal(format!("Export failed: {e}")))?)
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

            let stem = original_name
                .as_deref()
                .map(|n| n.rsplit('.').nth(1).unwrap_or(n))
                .unwrap_or("Imported document");
            let title = clean_title(q.name.as_deref().unwrap_or(stem));

            let _ = odt::odt_to_html(&data)?;
            let id = uuid::Uuid::new_v4().to_string();
            ctx.db().execute(
                "INSERT INTO documents (id, user_id, title, odt, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, datetime('now'), datetime('now'))",
                &[Value::text(&id), Value::text(&uid), Value::text(&title), Value::blob(data)],
            )?;

            Ok(ok(json!({ "title": title })))
        }
    })
}
