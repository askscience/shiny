//! Z-Image Turbo (local image generation): model status, background
//! downloads, and serving finished images to the chat.
//!
//! The heavy lifting lives in `crate::services::image_gen`; these handlers are
//! the Settings → Images surface plus the `<img>` endpoint the chat embeds.
//! Paths are namespaced `/api/imagegen/*` on purpose — the image plugin owns
//! the `/api/images/*` namespace.

use axum::extract::{Path, State};
use axum::Extension;
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use crate::api::AppState;
use crate::errors::AppError;
use crate::models::Traveler;

#[derive(Deserialize)]
pub struct ImageDownloadBody {
    pub quality: String,
}

/// GET /api/imagegen/status — model inventory + live download progress.
pub async fn status(State(state): State<AppState>) -> Result<Json<serde_json::Value>, AppError> {
    Ok(Json(json!({
        "success": true,
        "data": {
            "inventory": state.image_gen.inventory(),
            "downloads": state.image_gen.downloads(),
        }
    })))
}

/// POST /api/imagegen/download — start a background fetch of one tier (or
/// `all`). Returns immediately; Settings polls `/api/imagegen/status`.
pub async fn download(
    State(state): State<AppState>,
    Json(body): Json<ImageDownloadBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    let quality = body.quality.trim().to_lowercase();
    state
        .image_gen
        .start_download(&quality)
        .map_err(AppError::BadRequest)?;
    Ok(Json(json!({
        "success": true,
        "data": { "status": "downloading", "quality": quality }
    })))
}

/// GET /api/imagegen/images/:name — the PNG the chat embeds. The file lives in
/// the caller's Files home (`Pictures/Generated/`), so the name is validated
/// down to a bare file name before it ever touches the filesystem.
pub async fn serve(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
    Path(name): Path<String>,
) -> Result<axum::response::Response, AppError> {
    let valid = name.ends_with(".png")
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains("..")
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if !valid {
        return Err(AppError::BadRequest("invalid image name".into()));
    }

    let os_home = state.os_identity_for(&traveler).map(|u| u.home);
    let path = crate::services::image_gen::files_home(&traveler.id, os_home.as_deref())
        .join("Pictures")
        .join("Generated")
        .join(&name);
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|_| AppError::NotFound("image not found".into()))?;

    Ok(axum::response::Response::builder()
        .header("content-type", "image/png")
        // Content-addressed by name (timestamped), so a short cache is safe and
        // keeps re-rendered chat bubbles from refetching.
        .header("cache-control", "private, max-age=3600")
        .body(axum::body::Body::from(bytes))
        .map_err(|e| AppError::Internal(format!("response: {e}")))?)
}
