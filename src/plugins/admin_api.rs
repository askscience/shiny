//! Plugin API: install, list, uninstall, activate, deactivate.
//!
//! Every authenticated user can manage their own workspace — there is no
//! admin role. Installation (uploading a cdylib) affects the shared server,
//! but activation state is per-user (stored in `user_plugin_states`).

use axum::extract::{Multipart, State};
use axum::response::IntoResponse;
use axum::Extension;
use axum::Json;
use serde::Serialize;
use serde_json::json;

use crate::api::AppState;
use crate::errors::AppError;
use crate::models::Traveler;

#[derive(Serialize)]
struct PluginListEntry {
    name: String,
    version: String,
    api_level: u32,
    description: Option<String>,
    enabled: bool,
    summary: Option<String>,
}

/// GET /api/plugins — every authenticated user sees the same installed list
/// but with their own activation flags.
pub async fn list(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
) -> Result<Json<serde_json::Value>, AppError> {
    let active = active_plugin_set(&state, &traveler.id).await?;
    let entries: Vec<PluginListEntry> = state
        .plugins
        .list()
        .into_iter()
        .map(|m| {
            let enabled = active.contains(&m.name);
            PluginListEntry {
                name: m.name,
                version: m.version.to_string(),
                api_level: m.api_level,
                description: m.description,
                summary: m.summary,
                enabled,
            }
        })
        .collect();
    Ok(Json(json!({ "success": true, "data": entries })))
}

/// GET /api/plugins/active — minimal endpoint the frontend uses to decide what
/// UI to render. Returns an array of plugin names active for this user.
pub async fn active(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
) -> Result<Json<serde_json::Value>, AppError> {
    let active = active_plugin_set(&state, &traveler.id).await?;
    let names: Vec<String> = active.into_iter().collect();
    Ok(Json(json!({ "success": true, "data": names })))
}

/// POST /api/plugins/install — multipart upload, field `file` = the archive.
/// Auth required (any logged-in user). Installed cdylibs are shared server-side.
pub async fn install(
    State(state): State<AppState>,
    Extension(_traveler): Extension<Traveler>,
    mut multipart: Multipart,
) -> Result<Json<serde_json::Value>, AppError> {
    let mut archive_bytes: Option<Vec<u8>> = None;
    while let Some(field) = multipart.next_field().await
        .map_err(|e| AppError::BadRequest(format!("multipart error: {}", e)))?
    {
        let name = field.name().unwrap_or("").to_string();
        if name == "file" {
            let bytes = field.bytes().await
                .map_err(|e| AppError::BadRequest(format!("read error: {}", e)))?;
            archive_bytes = Some(bytes.to_vec());
        }
    }
    let bytes = archive_bytes.ok_or_else(|| AppError::BadRequest("missing 'file' field".into()))?;

    let base_ctx = state.plugin_ctx();
    let plugins_dir = std::path::Path::new(&state.config.plugins_dir);
    let name = crate::plugins::installer::install_archive(
        bytes,
        plugins_dir,
        &state.plugins,
        base_ctx,
    ).await?;

    // After install, the plugin is enabled by default for every existing user
    // (no row in user_plugin_states yet). The provided API treats absence as
    // enabled, so nothing to do here.

    if let Some(rebuild) = &state.router_rebuild {
        rebuild();
    }

    Ok(Json(json!({ "success": true, "data": { "installed": name } })))
}

/// POST /api/plugins/uninstall  body  {"name":"hello"}  — removes the plugin
/// from the server. Available to any logged-in user.
pub async fn uninstall(
    State(state): State<AppState>,
    Extension(_traveler): Extension<Traveler>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, AppError> {
    let name = body.get("name").and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("name required".into()))?
        .to_string();
    let removed = state.plugins.uninstall(&name);
    let dir = std::path::Path::new(&state.config.plugins_dir).join(&name);
    if dir.exists() {
        let _ = std::fs::remove_dir_all(&dir);
    }
    log_event(&state.config.plugins_dir, &format!("uninstall-ok name={name} removed={removed}"));

    if let Some(rebuild) = &state.router_rebuild {
        rebuild();
    }
    Ok(Json(json!({ "success": removed, "data": { "name": name } })))
}

/// POST /api/plugins/activate  body {"name":"hello"}  — re-enable a plugin
/// for the current user only.
pub async fn activate(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, AppError> {
    let name = body.get("name").and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("name required".into()))?
        .to_string();
    if !state.plugins.is_installed(&name) {
        return Err(AppError::BadRequest(format!("Plugin '{name}' is not installed")));
    }
    set_user_enabled(&state, &traveler.id, &name, true).await?;
    log_event(&state.config.plugins_dir, &format!("activate user={} name={}", traveler.username.clone().unwrap_or_default(), name));
    Ok(Json(json!({ "success": true, "data": { "name": name, "enabled": true } })))
}

/// POST /api/plugins/deactivate  body {"name":"hello"}  — disable a plugin
/// for the current user only.
pub async fn deactivate(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, AppError> {
    let name = body.get("name").and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("name required".into()))?
        .to_string();
    if !state.plugins.is_installed(&name) {
        return Err(AppError::BadRequest(format!("Plugin '{name}' is not installed")));
    }
    set_user_enabled(&state, &traveler.id, &name, false).await?;
    log_event(&state.config.plugins_dir, &format!("deactivate user={} name={}", traveler.username.clone().unwrap_or_default(), name));
    Ok(Json(json!({ "success": true, "data": { "name": name, "enabled": false } })))
}

/// GET /api/plugins/install.log  — last N lines of the install log. Any
/// authenticated user can see it (audit transparency).
pub async fn install_log(
    State(state): State<AppState>,
    Extension(_traveler): Extension<Traveler>,
) -> Result<axum::response::Response, AppError> {
    let path = std::path::Path::new(&state.config.plugins_dir).join("install.log");
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return Ok(axum::response::Response::builder()
            .status(axum::http::StatusCode::OK)
            .header(axum::http::header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .body(axum::body::Body::from(String::new()))
            .unwrap()),
    };
    let tail: String = content.lines().rev().take(200).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n");
    Ok(axum::response::Response::builder()
        .status(axum::http::StatusCode::OK)
        .header(axum::http::header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(axum::body::Body::from(tail))
        .unwrap())
}

// ---------- per-user activation helpers -------------------------------------

async fn active_plugin_set(state: &AppState, user_id: &str) -> Result<std::collections::BTreeSet<String>, AppError> {
    // A plugin is ACTIVE unless there's an explicit row with enabled=0 for the
    // current user. Installed plugins start enabled-by-default.
    let disabled_rows: Vec<String> = sqlx::query_scalar(
        "SELECT plugin_name FROM user_plugin_states WHERE user_id = ?1 AND enabled = 0",
    )
    .bind(user_id)
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    let installed: Vec<String> = state.plugins.list().into_iter().map(|m| m.name).collect();
    let disabled: std::collections::BTreeSet<String> = disabled_rows.into_iter().collect();
    let active: std::collections::BTreeSet<String> = installed.into_iter().filter(|n| !disabled.contains(n)).collect();
    Ok(active)
}

async fn set_user_enabled(state: &AppState, user_id: &str, plugin: &str, enabled: bool) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO user_plugin_states (user_id, plugin_name, enabled, updated_at) \
         VALUES (?1, ?2, ?3, datetime('now')) \
         ON CONFLICT(user_id, plugin_name) DO UPDATE SET enabled = excluded.enabled, updated_at = datetime('now')",
    )
    .bind(user_id)
    .bind(plugin)
    .bind(if enabled { 1 } else { 0 })
    .execute(&state.pool)
    .await?;
    Ok(())
}

fn log_event(plugins_dir: &str, line: &str) {
    let path = std::path::Path::new(plugins_dir).join("install.log");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    let entry = format!("[{ts}] {line}\n");
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = f.write_all(entry.as_bytes());
    }
}