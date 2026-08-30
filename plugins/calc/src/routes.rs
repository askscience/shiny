//! Calc plugin REST routes — served through the plugin's `RouteSpec`s.
//! DB access via the SDK's synchronous `ctx.db()`.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::extract::{FromRequest, FromRequestParts, Multipart};
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Map, Value as Json};

use shiny_plugin_sdk::db::Value;
use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::ods;
use shiny_plugin_sdk::routes::{bridged_route, RouteHandler, UserId};
use shiny_plugin_sdk::services::PluginCtx;

const MIME_ODS: &str = "application/vnd.oasis.opendocument.spreadsheet";
const MAX_CELLS: usize = 5000;
const MAX_CELL_VALUE_LEN: usize = 10_000;

pub fn handle(ctx: &Arc<PluginCtx>, tag: &str) -> Option<RouteHandler> {
    let ctx = ctx.clone();
    Some(match tag {
        "sheet_list" => sheet_list(ctx),
        "sheet_create" => sheet_create(ctx),
        "sheet_get" => sheet_get(ctx),
        "sheet_save" => sheet_save(ctx),
        "sheet_delete" => sheet_delete(ctx),
        "sheet_export" => sheet_export(ctx),
        "sheet_import" => sheet_import(ctx),
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
    let clean = if clean.is_empty() { "spreadsheet".to_string() } else { clean };
    format!("{clean}.ods")
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

fn parse_cells(json: &str) -> BTreeMap<String, String> {
    if json.trim().is_empty() {
        return BTreeMap::new();
    }
    serde_json::from_str::<BTreeMap<String, String>>(json)
        .unwrap_or_default()
        .into_iter()
        .take(MAX_CELLS)
        .collect()
}

fn is_valid_cell_ref(cell_ref: &str) -> bool {
    let bytes = cell_ref.as_bytes();
    let mut i = 0;
    let mut letters = 0;
    while i < bytes.len() && bytes[i].is_ascii_uppercase() {
        letters += 1;
        i += 1;
        if letters > 2 {
            return false;
        }
    }
    if letters == 0 || i >= bytes.len() {
        return false;
    }
    let digits = &cell_ref[i..];
    digits.len() <= 4 && digits.chars().all(|c| c.is_ascii_digit()) && digits != "0"
}

fn merge_cells(stored_json: &str, incoming: &BTreeMap<String, String>) -> Result<String, AppError> {
    let mut cells = parse_cells(stored_json);
    for (raw_ref, value) in incoming {
        let cell_ref = raw_ref.trim().to_uppercase();
        if !is_valid_cell_ref(&cell_ref) {
            return Err(AppError::BadRequest(format!(
                "Invalid cell reference \"{raw_ref}\" — expected something like A1 or BC42"
            )));
        }
        if value.chars().count() > MAX_CELL_VALUE_LEN {
            return Err(AppError::BadRequest(format!(
                "Value in {cell_ref} is too long (max {MAX_CELL_VALUE_LEN} chars)"
            )));
        }
        if value.trim().is_empty() {
            cells.remove(&cell_ref);
        } else {
            cells.insert(cell_ref, value.clone());
        }
    }
    if cells.len() > MAX_CELLS {
        return Err(AppError::BadRequest(format!("Too many cells (max {MAX_CELLS})")));
    }
    Ok(serde_json::to_string(&cells)?)
}

fn clamp_dim(v: i64, min: i64, max: i64) -> i64 {
    v.clamp(min, max)
}

/* ── handlers ───────────────────────────────────────────────── */

fn sheet_list(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let rows = ctx.db().query(
                "SELECT id, title, rows, cols, updated_at FROM spreadsheets \
                 WHERE user_id = ?1 ORDER BY updated_at DESC LIMIT 200",
                &[Value::text(&uid)],
            )?;
            let sheets: Vec<Json> = rows
                .iter()
                .map(|r| json!({ "id": as_text(&r[0]), "title": as_text(&r[1]), "rows": as_int(&r[2]), "cols": as_int(&r[3]), "updated_at": as_text(&r[4]) }))
                .collect();
            Ok(ok(json!(sheets)))
        }
    })
}

fn sheet_create(ctx: Arc<PluginCtx>) -> RouteHandler {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Create {
        title: Option<String>,
        rows: Option<i64>,
        cols: Option<i64>,
    }

    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let axum::Json(body) = axum::Json::<Create>::from_request(req, &())
                .await
                .map_err(|e| AppError::BadRequest(format!("invalid JSON body: {e}")))?;
            let title = clean_title(body.title.as_deref().unwrap_or("Untitled"));
            let rows = clamp_dim(body.rows.unwrap_or(100), 1, 500);
            let cols = clamp_dim(body.cols.unwrap_or(26), 1, 52);
            let id = uuid::Uuid::new_v4().to_string();

            ctx.db().execute(
                "INSERT INTO spreadsheets (id, user_id, title, cells, rows, cols, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, '{}', ?4, ?5, datetime('now'), datetime('now'))",
                &[Value::text(&id), Value::text(&uid), Value::text(&title), Value::Int(rows), Value::Int(cols)],
            )?;

            Ok(ok(json!({ "id": id, "title": title, "rows": rows, "cols": cols, "cells": {}, "updated_at": "now" })))
        }
    })
}

fn sheet_get(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let (id, _) = take_path::<String>(req).await?;
            let rows = ctx.db().query(
                "SELECT title, cells, rows, cols, updated_at FROM spreadsheets \
                 WHERE id = ?1 AND user_id = ?2",
                &[Value::text(&id), Value::text(&uid)],
            )?;
            let row = rows.first().ok_or_else(|| AppError::NotFound("Spreadsheet not found".into()))?;
            let title = as_text(&row[0]);
            let cells: Map<String, Json> = parse_cells(&as_text(&row[1]))
                .into_iter()
                .map(|(k, v)| (k, Json::String(v)))
                .collect();
            let rows_n = as_int(&row[2]);
            let cols_n = as_int(&row[3]);
            let updated_at = as_text(&row[4]);
            Ok(ok(json!({ "id": id, "title": title, "rows": rows_n, "cols": cols_n, "cells": cells, "updated_at": updated_at })))
        }
    })
}

fn sheet_save(ctx: Arc<PluginCtx>) -> RouteHandler {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Save {
        title: Option<String>,
        cells: Option<Map<String, Json>>,
    }

    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let (id, req) = take_path::<String>(req).await?;
            let axum::Json(body) = axum::Json::<Save>::from_request(req, &())
                .await
                .map_err(|e| AppError::BadRequest(format!("invalid JSON body: {e}")))?;

            let incoming: BTreeMap<String, String> = body
                .cells
                .unwrap_or_default()
                .into_iter()
                .map(|(k, v)| {
                    let s = match v {
                        Json::String(s) => s,
                        Json::Number(n) => n.to_string(),
                        Json::Bool(b) => b.to_string(),
                        Json::Null => String::new(),
                        other => other.to_string(),
                    };
                    (k, s)
                })
                .collect();

            let current = ctx.db().query(
                "SELECT title, cells FROM spreadsheets WHERE id = ?1 AND user_id = ?2",
                &[Value::text(&id), Value::text(&uid)],
            )?;
            let cur = current.first().ok_or_else(|| AppError::NotFound("Spreadsheet not found".into()))?;
            let new_title = match body.title {
                Some(t) => clean_title(&t),
                None => as_text(&cur[0]),
            };
            let merged = merge_cells(&as_text(&cur[1]), &incoming)?;

            let changed = ctx.db().execute(
                "UPDATE spreadsheets SET title = ?1, cells = ?2, updated_at = datetime('now') \
                 WHERE id = ?3 AND user_id = ?4",
                &[Value::text(&new_title), Value::text(&merged), Value::text(&id), Value::text(&uid)],
            )?;
            if changed == 0 {
                return Err(AppError::NotFound("Spreadsheet not found".into()));
            }
            Ok(ok(json!({ "success": true })))
        }
    })
}

fn sheet_delete(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let (id, _) = take_path::<String>(req).await?;
            let changed = ctx.db().execute(
                "DELETE FROM spreadsheets WHERE id = ?1 AND user_id = ?2",
                &[Value::text(&id), Value::text(&uid)],
            )?;
            if changed == 0 {
                return Err(AppError::NotFound("Spreadsheet not found".into()));
            }
            Ok(ok(json!({ "success": true })))
        }
    })
}

fn sheet_export(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let (id, _) = take_path::<String>(req).await?;
            let rows = ctx.db().query(
                "SELECT title, cells FROM spreadsheets WHERE id = ?1 AND user_id = ?2",
                &[Value::text(&id), Value::text(&uid)],
            )?;
            let row = rows.first().ok_or_else(|| AppError::NotFound("Spreadsheet not found".into()))?;
            let title = as_text(&row[0]);
            let cells = parse_cells(&as_text(&row[1]));
            let bytes = ods::cells_to_ods(&cells)?;
            let filename = filename_for_title(&title);
            Ok(Response::builder()
                .header(CONTENT_TYPE, MIME_ODS)
                .header(CONTENT_DISPOSITION, format!("attachment; filename=\"{}\"", filename.replace('"', "")))
                .body(axum::body::Body::from(bytes))
                .map_err(|e| AppError::Internal(format!("Export failed: {e}")))?)
        }
    })
}

fn sheet_import(ctx: Arc<PluginCtx>) -> RouteHandler {
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
                .unwrap_or("Imported spreadsheet");
            let title = clean_title(q.name.as_deref().unwrap_or(stem));

            let cells = ods::ods_to_cells(&data)?;
            let cells_json = serde_json::to_string(&cells)?;

            let mut max_row = 1i64;
            let mut max_col = 1i64;
            for ref_ in cells.keys() {
                if let Some((row, col)) = ref_row_col(ref_) {
                    max_row = max_row.max(row);
                    max_col = max_col.max(col);
                }
            }

            let id = uuid::Uuid::new_v4().to_string();
            ctx.db().execute(
                "INSERT INTO spreadsheets (id, user_id, title, cells, rows, cols, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'), datetime('now'))",
                &[
                    Value::text(&id),
                    Value::text(&uid),
                    Value::text(&title),
                    Value::text(&cells_json),
                    Value::Int(max_row.min(500)),
                    Value::Int(max_col.min(52)),
                ],
            )?;

            Ok(ok(json!({ "title": title })))
        }
    })
}

fn ref_row_col(ref_: &str) -> Option<(i64, i64)> {
    let bytes = ref_.as_bytes();
    let mut i = 0;
    let mut col: i64 = 0;
    while i < bytes.len() && bytes[i].is_ascii_uppercase() {
        col = col * 26 + (bytes[i] - b'A' + 1) as i64;
        i += 1;
    }
    if i == 0 || i >= bytes.len() {
        return None;
    }
    let row: i64 = ref_[i..].parse().ok()?;
    if row == 0 {
        return None;
    }
    Some((row, col))
}
