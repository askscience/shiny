//! System font enumeration for the appearance font picker.
//!
//! The UI lists what fontconfig actually has installed rather than a baked-in
//! list, so a font the user installs (`apt install fonts-…`, dropping a file in
//! `~/.local/share/fonts`, …) shows up without a code change.

use axum::Json;
use serde_json::Value;

use crate::errors::AppError;

/// GET /api/fonts — font family names installed on the host, sorted.
pub async fn list() -> Result<Json<Value>, AppError> {
    let families = tokio::task::spawn_blocking(list_families)
        .await
        .map_err(|e| AppError::Internal(format!("font enumeration failed: {e}")))?;
    Ok(Json(serde_json::json!({ "success": true, "data": families })))
}

fn list_families() -> Vec<String> {
    // `family[0]` is the primary family; the full `family` property appends
    // style-named instances ("DM Sans 9pt") that make for noisy menu entries.
    let Ok(output) = std::process::Command::new("fc-list")
        .args(["-f", "%{family[0]}\n"])
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut families: Vec<String> = text
        .lines()
        .map(|line| line.trim().to_string())
        .filter(|name| !name.is_empty())
        .collect();
    families.sort_by_key(|name| name.to_lowercase());
    families.dedup();
    families
}
