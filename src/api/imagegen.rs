//! Local image generation (Z-Image Turbo / Qwen-Image 2.1 Turbo): model
//! status, background downloads, and serving finished images to the chat.
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
use crate::services::image_gen;

#[derive(Deserialize)]
pub struct ImageDownloadBody {
    pub quality: String,
    /// Family id ("z-image" | "qwen-image-2.1"); defaults to Z-Image.
    #[serde(default)]
    pub model: Option<String>,
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
/// `all`) for one model. Returns immediately; Settings polls
/// `/api/imagegen/status`.
pub async fn download(
    State(state): State<AppState>,
    Json(body): Json<ImageDownloadBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    let quality = body.quality.trim().to_lowercase();
    let model = body.model.as_deref().unwrap_or(image_gen::DEFAULT_FAMILY);
    let Some(family) = image_gen::resolve_family(model) else {
        return Err(AppError::BadRequest(format!("unknown image model: {model}")));
    };
    state
        .image_gen
        .start_download(family, &quality)
        .map_err(AppError::BadRequest)?;
    Ok(Json(json!({
        "success": true,
        "data": { "status": "downloading", "model": family.id, "quality": quality }
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
