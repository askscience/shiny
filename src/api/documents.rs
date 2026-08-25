//! REST surface for the word plugin's documents.
//!
//! Interim pattern (same as `/api/radio/nowplaying` and the traveler REST
//! handlers): plugin-contributed routes are a roadmap item, so the word
//! plugin's storage is served by core routes while its AI tools live in the
//! plugin. Storage is core-owned (`documents` table) and real .odt bytes.

use axum::extract::{Extension, Multipart, Path, Query, State};
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::response::Response;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::api::AppState;
use crate::errors::AppError;
use crate::models::Traveler;
use crate::services::documents;

#[derive(Deserialize)]
pub struct CreateRequest {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    html: Option<String>,
}

#[derive(Deserialize)]
pub struct SaveRequest {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    html: Option<String>,
}

#[derive(Deserialize)]
pub struct ImportQuery {
    #[serde(default)]
    name: Option<String>,
}

pub async fn list(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
) -> Result<Json<serde_json::Value>, AppError> {
    let data = documents::list_documents(&state.pool, &traveler.id).await?;
    Ok(Json(json!({ "success": true, "data": data })))
}

pub async fn create(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
    Json(body): Json<CreateRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let doc = documents::create_document(
        &state.pool,
        &traveler.id,
        body.title.as_deref().unwrap_or("Untitled"),
        body.html.as_deref().unwrap_or(""),
    )
    .await?;
    Ok(Json(json!({ "success": true, "data": doc })))
}

pub async fn get_one(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let doc = documents::load_document_html(&state.pool, &traveler.id, &id).await?;
    Ok(Json(json!({ "success": true, "data": doc })))
}

pub async fn save(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
    Path(id): Path<String>,
    Json(body): Json<SaveRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let html = body.html.unwrap_or_default();
    // Load the existing title when none given so renames never wipe it.
    let title = match body.title {
        Some(t) => t,
        None => documents::load_document_html(&state.pool, &traveler.id, &id)
            .await
            .map(|d| d.title)
            .unwrap_or_else(|_| "Untitled".into()),
    };
    documents::save_document_html(&state.pool, &traveler.id, &id, &title, &html).await?;
    Ok(Json(json!({ "success": true })))
}

pub async fn delete(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let removed = documents::delete_document(&state.pool, &traveler.id, &id).await?;
    if !removed {
        return Err(AppError::NotFound("Document not found".into()));
    }
    Ok(Json(json!({ "success": true })))
}

/// GET /api/documents/:id/export — download the real .odt file.
pub async fn export_odt(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let (title, bytes) = documents::load_document_odt(&state.pool, &traveler.id, &id).await?;
    let filename = documents::filename_for_title(&title);

    Response::builder()
        .header(CONTENT_TYPE, "application/vnd.oasis.opendocument.text")
        .header(
            CONTENT_DISPOSITION,
            format!("attachment; filename=\"{}\"", filename.replace('"', "")),
        )
        .body(axum::body::Body::from(bytes))
        .map_err(|e| AppError::Internal(format!("Export failed: {}", e)))
}

/// POST /api/documents/import — multipart `.odt` upload (raised body limit).
pub async fn import_odt(
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
        .unwrap_or("Imported document");
    let title = q.name.clone().unwrap_or_else(|| stem.to_string());

    let stored = documents::import_document_odt(&state.pool, &traveler.id, &title, &data).await?;
    Ok(Json(json!({ "success": true, "data": { "title": stored } })))
}