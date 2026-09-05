//! Studio plugin REST routes — served through the plugin's `RouteSpec`s.
//!
//! The Studio window and the AI tools share these routes and the same render
//! engine. Rendered audio is stored as a WAV BLOB and served from
//! `GET /api/studio/:id/audio`.

use std::sync::Arc;

use axum::extract::Request;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::response::{IntoResponse, Response};
use serde_json::{json, Value};

use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::routes::{
    bridged_route, path_params_from_request, user_id_from_request, RouteHandler,
};
use shiny_plugin_sdk::services::PluginCtx;

use crate::engine;
use crate::store;

const MAX_BODY: usize = 1024 * 1024;

pub fn handle(ctx: &Arc<PluginCtx>, tag: &str) -> Option<RouteHandler> {
    let ctx = ctx.clone();
    Some(match tag {
        "studio_list" => studio_list(ctx),
        "studio_create" => studio_create(ctx),
        "studio_get" => studio_get(ctx),
        "studio_audio" => studio_audio(ctx),
        "studio_update" => studio_update(ctx),
        "studio_render" => studio_render(ctx),
        "studio_delete" => studio_delete(ctx),
        "studio_arrangement_render" => studio_arrangement_render(ctx),
        "studio_preview" => studio_preview(ctx),
        "studio_waveform" => studio_waveform(ctx),
        "studio_arrangement_list" => studio_arrangement_list(ctx),
        "studio_arrangement_save" => studio_arrangement_save(ctx),
        "studio_arrangement_get" => studio_arrangement_get(ctx),
        "studio_arrangement_update" => studio_arrangement_update(ctx),
        "studio_arrangement_delete" => studio_arrangement_delete(ctx),
        "studio_preset_list" => studio_preset_list(ctx),
        "studio_preset_save" => studio_preset_save(ctx),
        "studio_preset_delete" => studio_preset_delete(ctx),
        _ => return None,
    })
}

fn user_id(req: &Request) -> Result<String, AppError> {
    user_id_from_request(req).ok_or_else(|| AppError::Unauthorized("not authenticated".into()))
}

fn ok(data: Value) -> Response {
    axum::Json(json!({ "success": true, "data": data })).into_response()
}

fn take_path(req: &Request) -> Result<String, AppError> {
    let mut params = path_params_from_request(req)
        .ok_or_else(|| AppError::BadRequest("no path parameter found".into()))?;
    if params.len() != 1 {
        return Err(AppError::BadRequest("expected exactly one path parameter".into()));
    }
    Ok(params.remove(0).1)
}

async fn read_json(req: Request) -> Result<Value, AppError> {
    let bytes = axum::body::to_bytes(req.into_body(), MAX_BODY)
        .await
        .map_err(|e| AppError::BadRequest(format!("invalid body: {e}")))?;
    serde_json::from_slice(&bytes).map_err(|e| AppError::BadRequest(format!("invalid json: {e}")))
}

/* ── GET /api/studio ────────────────────────────────────────── */

fn studio_list(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let rows = store::list(ctx.pool().await, &uid).await?;
            let tracks: Vec<Value> = rows.iter().map(store::meta_json).collect();
            Ok(ok(json!({ "tracks": tracks, "count": tracks.len() })))
        }
    })
}

/* ── POST /api/studio (create + render) ─────────────────────── */

fn studio_create(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let value = read_json(req).await?;
            let cfg = engine::parse_config(&value).map_err(AppError::BadRequest)?;
            let rendered = engine::render_track(&cfg).map_err(AppError::BadRequest)?;

            let id = uuid::Uuid::new_v4().to_string();
            let cfg_json = serde_json::to_string(&cfg).map_err(AppError::from)?;
            let title = if cfg.title.trim().is_empty() { "Untitled".into() } else { cfg.title.trim().to_string() };
            store::insert(
                ctx.pool().await,
                &id,
                &uid,
                &cfg_json,
                &title,
                cfg.bpm,
                cfg.steps as i64,
                &cfg.tuning,
                rendered.duration_ms as i64,
                &rendered.wav,
            )
            .await?;

            Ok(ok(json!({
                "track_id": id,
                "title": title,
                "bpm": cfg.bpm,
                "steps": cfg.steps,
                "tuning": cfg.tuning,
                "duration_ms": rendered.duration_ms,
                "has_audio": true,
                "sample_rate": rendered.sample_rate,
            })))
        }
    })
}

/* ── GET /api/studio/:id ────────────────────────────────────── */

fn studio_get(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_path(&req)?;
            let row = store::get(ctx.pool().await, &uid, &id).await?;
            match row {
                Some(r) => Ok(ok(store::full_json(&r))),
                None => Err(AppError::NotFound("studio track not found".into())),
            }
        }
    })
}

/* ── GET /api/studio/:id/audio ──────────────────────────────── */

fn studio_audio(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_path(&req)?;
            let row = store::get(ctx.pool().await, &uid, &id).await?;
            let Some(row) = row else {
                return Err(AppError::NotFound("studio track not found".into()));
            };
            let Some(wav) = row.6 else {
                return Err(AppError::NotFound("track has not been rendered yet".into()));
            };
            Response::builder()
                .status(200)
                .header(CONTENT_TYPE, "audio/wav")
                .header(CACHE_CONTROL, "no-store")
                .body(axum::body::Body::from(wav))
                .map_err(|e| AppError::Internal(format!("response build: {e}")))
        }
    })
}

/* ── PUT /api/studio/:id (update config without render) ─────── */

fn studio_update(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_path(&req)?;
            let value = read_json(req).await?;
            let cfg = engine::parse_config(&value).map_err(AppError::BadRequest)?;
            let cfg_json = serde_json::to_string(&cfg).map_err(AppError::from)?;
            let title = if cfg.title.trim().is_empty() { "Untitled".into() } else { cfg.title.trim().to_string() };
            let changed = store::update_config(
                ctx.pool().await,
                &id,
                &uid,
                &cfg_json,
                &title,
                cfg.bpm,
                cfg.steps as i64,
                &cfg.tuning,
            )
            .await?;
            if !changed {
                return Err(AppError::NotFound("studio track not found".into()));
            }
            Ok(ok(json!({ "track_id": id, "title": title, "bpm": cfg.bpm, "steps": cfg.steps, "tuning": cfg.tuning, "has_audio": false })))
        }
    })
}

/* ── POST /api/studio/:id/render ────────────────────────────── */

fn studio_render(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_path(&req)?;
            let row = store::get(ctx.pool().await, &uid, &id).await?;
            let Some(row) = row else {
                return Err(AppError::NotFound("studio track not found".into()));
            };
            let cfg = serde_json::from_str::<engine::TrackConfig>(&row.7).map_err(AppError::from)?;
            let rendered = engine::render_track(&cfg).map_err(AppError::BadRequest)?;
            store::update_render(ctx.pool().await, &id, &uid, rendered.duration_ms as i64, &rendered.wav).await?;
            Ok(ok(json!({ "track_id": id, "title": row.1, "duration_ms": rendered.duration_ms, "has_audio": true })))
        }
    })
}

/* ── POST /api/studio/arrangement/render ────────────────────── */

fn studio_arrangement_render(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let ctx = ctx.clone();
        async move {
            let _uid = user_id(&req)?;
            let value = read_json(req).await?;
            let arr = engine::parse_arrangement(&value).map_err(AppError::BadRequest)?;
            let rendered = engine::render_arrangement(&arr).map_err(AppError::BadRequest)?;
            Response::builder()
                .status(200)
                .header(CONTENT_TYPE, "audio/wav")
                .header(CACHE_CONTROL, "no-store")
                .body(axum::body::Body::from(rendered.wav))
                .map_err(|e| AppError::Internal(format!("response build: {e}")))
        }
    })
}

/* ── POST /api/studio/preview (render a pattern without storing) ── */

fn studio_preview(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let ctx = ctx.clone();
        async move {
            let _uid = user_id(&req)?;
            let value = read_json(req).await?;
            let cfg = engine::parse_config(&value).map_err(AppError::BadRequest)?;
            let rendered = engine::render_track(&cfg).map_err(AppError::BadRequest)?;
            Response::builder()
                .status(200)
                .header(CONTENT_TYPE, "audio/wav")
                .header(CACHE_CONTROL, "no-store")
                .body(axum::body::Body::from(rendered.wav))
                .map_err(|e| AppError::Internal(format!("response build: {e}")))
        }
    })
}

/* ── POST /api/studio/waveform (peak envelope for a pattern) ── */

fn studio_waveform(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let ctx = ctx.clone();
        async move {
            let _uid = user_id(&req)?;
            let value = read_json(req).await?;
            let cfg = engine::parse_config(&value).map_err(AppError::BadRequest)?;
            let duration_ms = (cfg.steps as f64 / 4.0 * 60.0 / cfg.bpm * 1000.0).round() as u32;
            let peaks = engine::waveform_peaks(&cfg, 96).map_err(AppError::BadRequest)?;
            Ok(ok(json!({ "peaks": peaks, "duration_ms": duration_ms })))
        }
    })
}

/* ── Arrangement CRUD ──────────────────────────────────────── */

fn studio_arrangement_list(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let rows = store::list_arrangements(ctx.pool().await, &uid).await?;
            let arrangements: Vec<Value> = rows.iter().map(store::arr_meta_json).collect();
            Ok(ok(json!({ "arrangements": arrangements, "count": arrangements.len() })))
        }
    })
}

fn studio_arrangement_save(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let value = read_json(req).await?;
            let arr = engine::parse_arrangement(&value).map_err(AppError::BadRequest)?;
            let cfg_json = serde_json::to_string(&arr).map_err(AppError::from)?;
            let title = if arr.title.trim().is_empty() { "Untitled".into() } else { arr.title.trim().to_string() };
            let id = uuid::Uuid::new_v4().to_string();
            store::insert_arrangement(ctx.pool().await, &id, &uid, &title, arr.bpm, arr.length_beats, arr.master as f64, &cfg_json).await?;
            Ok(ok(json!({ "id": id, "title": title })))
        }
    })
}

fn studio_arrangement_get(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_path(&req)?;
            let row = store::get_arrangement(ctx.pool().await, &uid, &id).await?;
            match row {
                Some(r) => Ok(ok(store::arr_full_json(&r))),
                None => Err(AppError::NotFound("arrangement not found".into())),
            }
        }
    })
}

fn studio_arrangement_update(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_path(&req)?;
            let value = read_json(req).await?;
            let arr = engine::parse_arrangement(&value).map_err(AppError::BadRequest)?;
            let cfg_json = serde_json::to_string(&arr).map_err(AppError::from)?;
            let title = if arr.title.trim().is_empty() { "Untitled".into() } else { arr.title.trim().to_string() };
            let changed = store::update_arrangement(ctx.pool().await, &id, &uid, &title, arr.bpm, arr.length_beats, arr.master as f64, &cfg_json).await?;
            if !changed {
                return Err(AppError::NotFound("arrangement not found".into()));
            }
            Ok(ok(json!({ "id": id, "title": title })))
        }
    })
}

fn studio_arrangement_delete(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_path(&req)?;
            let title = store::delete_arrangement(ctx.pool().await, &id, &uid).await?;
            match title {
                Some(t) => Ok(ok(json!({ "id": id, "title": t }))),
                None => Err(AppError::NotFound("arrangement not found".into())),
            }
        }
    })
}

/* ── Presets CRUD ───────────────────────────────────────────── */

fn studio_preset_list(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let rows = store::list_presets(ctx.pool().await, &uid).await?;
            let presets: Vec<Value> = rows.iter().map(store::preset_json).collect();
            Ok(ok(json!({ "presets": presets, "count": presets.len() })))
        }
    })
}

fn studio_preset_save(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let value = read_json(req).await?;
            let kind = value.get("kind").and_then(|v| v.as_str()).unwrap_or("kick").to_string();
            let name = value.get("name").and_then(|v| v.as_str()).unwrap_or("Preset").trim().to_string();
            let name = if name.is_empty() { "Preset".into() } else { name };
            let params = value.get("params").cloned().unwrap_or(json!({}));
            let params_json = serde_json::to_string(&params).map_err(AppError::from)?;
            let id = uuid::Uuid::new_v4().to_string();
            store::insert_preset(ctx.pool().await, &id, &uid, &kind, &name, &params_json).await?;
            Ok(ok(json!({ "id": id, "kind": kind, "name": name })))
        }
    })
}

fn studio_preset_delete(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_path(&req)?;
            let name = store::delete_preset(ctx.pool().await, &id, &uid).await?;
            match name {
                Some(n) => Ok(ok(json!({ "id": id, "name": n }))),
                None => Err(AppError::NotFound("preset not found".into())),
            }
        }
    })
}

/* ── DELETE /api/studio/:id ─────────────────────────────────── */

fn studio_delete(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let id = take_path(&req)?;
            let title = store::delete(ctx.pool().await, &id, &uid).await?;
            match title {
                Some(t) => Ok(ok(json!({ "track_id": id, "title": t }))),
                None => Err(AppError::NotFound("studio track not found".into())),
            }
        }
    })
}
