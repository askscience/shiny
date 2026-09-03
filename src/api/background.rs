//! Per-user desktop background image upload / serve / remove.
//!
//! The frontend stores a *reference* (the `/api/background` URL) in the user's
//! `ui.background` preference; the bytes live on disk under `data/backgrounds`
//! (one file per user, so a fresh upload simply replaces the previous one).

use axum::extract::{Multipart, State};
use axum::Extension;
use axum::Json;
use serde_json::json;

use crate::api::AppState;
use crate::errors::AppError;
use crate::models::Traveler;

const MAX_BYTES: usize = 12 * 1024 * 1024; // 12 MB
const ALLOWED: &[(&str, &str)] = &[
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("webp", "image/webp"),
    ("gif", "image/gif"),
];

fn backgrounds_dir(state: &AppState) -> std::path::PathBuf {
    std::path::Path::new(&state.config.backgrounds_dir).to_path_buf()
}

fn file_for_user(state: &AppState, user_id: &str) -> Option<std::path::PathBuf> {
    let dir = backgrounds_dir(state);
    for (ext, _) in ALLOWED {
        let path = dir.join(format!("{user_id}.{ext}"));
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

fn mime_for_path(path: &std::path::Path) -> &'static str {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    ALLOWED
        .iter()
        .find(|(e, _)| *e == ext)
        .map(|(_, mime)| *mime)
        .unwrap_or("application/octet-stream")
}

fn detect_ext(content_type: Option<&str>, bytes: &[u8]) -> Option<&'static str> {
    if let Some(ct) = content_type {
        for (ext, mime) in ALLOWED {
            if ct.starts_with(mime) {
                return Some(ext);
            }
        }
    }
    // Magic-byte fallback (the browser usually sends the right content type).
    if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        return Some("png");
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("jpg");
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("webp");
    }
    if bytes.len() >= 6 && (&bytes[0..6] == b"GIF87a" || &bytes[0..6] == b"GIF89a") {
        return Some("gif");
    }
    None
}

/// POST /api/background — multipart upload, field `file`. Replaces any previous
/// background image for the caller.
pub async fn upload(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
    mut multipart: Multipart,
) -> Result<Json<serde_json::Value>, AppError> {
    // Drain the WHOLE multipart stream (don't break early). Leaving unread
    // body bytes on a keep-alive connection corrupts the next request on the
    // same connection, which surfaces in the browser as a "network error".
    let mut bytes: Option<Vec<u8>> = None;
    let mut content_type: Option<String> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("multipart error: {e}")))?
    {
        let is_file = field.name() == Some("file");
        let field_ct = field.content_type().map(|c| c.to_string());
        let data = field
            .bytes()
            .await
            .map_err(|e| AppError::BadRequest(format!("read error: {e}")))?
            .to_vec();
        if is_file {
            content_type = field_ct;
            bytes = Some(data);
        }
    }
    let bytes = bytes.ok_or_else(|| AppError::BadRequest("missing 'file' field".into()))?;
    if bytes.len() > MAX_BYTES {
        return Err(AppError::BadRequest(
            "Background image is too large (12 MB max)".into(),
        ));
    }

    let ext = detect_ext(content_type.as_deref(), &bytes).ok_or_else(|| {
        AppError::BadRequest("Background must be a PNG, JPEG, WebP or GIF image".into())
    })?;

    let dir = backgrounds_dir(&state);
    std::fs::create_dir_all(&dir)
        .map_err(|e| AppError::Internal(format!("create dir: {e}")))?;

    // Remove any previous background for this user before writing the new one.
    for (old_ext, _) in ALLOWED {
        let _ = std::fs::remove_file(dir.join(format!("{}.{old_ext}", traveler.id)));
    }

    let path = dir.join(format!("{}.{ext}", traveler.id));
    std::fs::write(&path, &bytes).map_err(|e| AppError::Internal(format!("write: {e}")))?;

    Ok(Json(json!({
        "success": true,
        "data": { "url": "/api/background", "ext": ext }
    })))
}

/// GET /api/background — serve the caller's background image.
pub async fn serve(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
) -> Result<axum::response::Response, AppError> {
    let path = file_for_user(&state, &traveler.id)
        .ok_or_else(|| AppError::NotFound("No background image uploaded".into()))?;
    let bytes = std::fs::read(&path).map_err(|e| AppError::Internal(format!("read: {e}")))?;
    let mime = mime_for_path(&path);
    Ok(axum::response::Response::builder()
        .header("content-type", mime)
        .header("cache-control", "no-store")
        .body(axum::body::Body::from(bytes))
        .map_err(|e| AppError::Internal(format!("response: {e}")))?)
}

/// DELETE /api/background — remove the caller's background image.
pub async fn remove(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
) -> Result<Json<serde_json::Value>, AppError> {
    if let Some(path) = file_for_user(&state, &traveler.id) {
        let _ = std::fs::remove_file(path);
    }
    Ok(Json(json!({ "success": true })))
}
