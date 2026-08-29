//! REST surface for the calc plugin's spreadsheets.
//!
//! Interim pattern (same as `/api/documents` and `/api/radio/nowplaying`):
//! plugin-contributed routes are a roadmap item, so the calc plugin's storage
//! is served by core routes while its AI tools live in the plugin. Storage is
//! core-owned (`spreadsheets` table); cells travel as a JSON map "A1" -> value.

use axum::extract::{Extension, Multipart, Path, Query, State};
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::response::Response;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::api::AppState;
use crate::errors::AppError;
use crate::models::Traveler;
use crate::services::spreadsheets;

#[derive(Deserialize)]
pub struct CreateRequest {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    rows: Option<i64>,
    #[serde(default)]
    cols: Option<i64>,
}

#[derive(Deserialize)]
pub struct SaveRequest {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    cells: Option<Map<String, Value>>,
}

pub async fn list(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
) -> Result<Json<serde_json::Value>, AppError> {
    let data = spreadsheets::list_spreadsheets(&state.pool, &traveler.id).await?;
    Ok(Json(json!({ "success": true, "data": data })))
}

pub async fn create(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
    Json(body): Json<CreateRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let sheet = spreadsheets::create_spreadsheet(
        &state.pool,
        &traveler.id,
        body.title.as_deref().unwrap_or("Untitled"),
        body.rows,
        body.cols,
    )
    .await?;
    Ok(Json(json!({ "success": true, "data": sheet })))
}

pub async fn get_one(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let sheet = spreadsheets::load_spreadsheet(&state.pool, &traveler.id, &id).await?;
    Ok(Json(json!({ "success": true, "data": sheet })))
}

pub async fn save(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
    Path(id): Path<String>,
    Json(body): Json<SaveRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Values may be numbers/bools from a JSON client — normalize to strings
    // the same way the plugin tools do.
    let cells = body
        .cells
        .unwrap_or_default()
        .into_iter()
        .map(|(k, v)| {
            let s = match v {
                Value::String(s) => s,
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                Value::Null => String::new(),
                other => other.to_string(),
            };
            (k, s)
        })
        .collect();

    spreadsheets::save_spreadsheet(
        &state.pool,
        &traveler.id,
        &id,
        body.title.as_deref(),
        &cells,
    )
    .await?;
    Ok(Json(json!({ "success": true })))
}

pub async fn delete(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let removed = spreadsheets::delete_spreadsheet(&state.pool, &traveler.id, &id).await?;
    if !removed {
        return Err(AppError::NotFound("Spreadsheet not found".into()));
    }
    Ok(Json(json!({ "success": true })))
}

/// GET /api/spreadsheets/:id/export — download a real `.ods` file.
pub async fn export_ods(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let (title, bytes) = spreadsheets::load_spreadsheet_ods(&state.pool, &traveler.id, &id).await?;
    let filename = spreadsheets::filename_for_title(&title);

    Response::builder()
        .header(CONTENT_TYPE, "application/vnd.oasis.opendocument.spreadsheet")
        .header(
            CONTENT_DISPOSITION,
            format!("attachment; filename=\"{}\"", filename.replace('"', "")),
        )
        .body(axum::body::Body::from(bytes))
        .map_err(|e| AppError::Internal(format!("Export failed: {}", e)))
}

#[derive(Deserialize)]
pub struct ImportQuery {
    #[serde(default)]
    name: Option<String>,
}

/// POST /api/spreadsheets/import — multipart `.ods` upload (raised body limit).
pub async fn import_ods(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
    Query(q): Query<ImportQuery>,
    mut multipart: Multipart,
) -> Result<Json<serde_json::Value>, AppError> {
    let mut bytes: Option<Vec<u8>> = None;
    let mut original_name: Option<String> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("multipart error: {}", e)))?
    {
        if field.name() == Some("file") {
            original_name = field
                .file_name()
                .map(|f| f.to_string())
                .or_else(|| original_name);
            let data = field
                .bytes()
                .await
                .map_err(|e| AppError::BadRequest(format!("read error: {}", e)))?;
            bytes = Some(data.to_vec());
        }
    }
    let data = bytes.ok_or_else(|| AppError::BadRequest("missing 'file' field".into()))?;

    let stem = original_name
        .as_deref()
        .map(|n| n.rsplit('.').nth(1).unwrap_or(n))
        .unwrap_or("Imported spreadsheet");
    let title = q.name.clone().unwrap_or_else(|| stem.to_string());

    let stored = spreadsheets::import_spreadsheet_ods(&state.pool, &traveler.id, &title, &data).await?;
    Ok(Json(json!({ "success": true, "data": { "title": stored } })))
}
