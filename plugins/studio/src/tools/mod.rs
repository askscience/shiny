//! Studio plugin tools: compose, list, get, render and delete tracks.
//!
//! Tools write through the plugin's own SQLite pool (`ctx.pool()`) and share
//! the same render engine as the REST routes (`crate::engine`).

use async_trait::async_trait;
use serde_json::{json, Value};
use sqlx::SqlitePool;

use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::outcome::ActionOutcome;
use shiny_plugin_sdk::services::PluginCtx;
use shiny_plugin_sdk::tools::{ParamHelpers, Tool, ToolRequest};

use crate::engine::{self, TrackConfig};
use crate::store;

async fn resolve_track_id(
    pool: &SqlitePool,
    user_id: &str,
    id_or_title: Option<String>,
) -> Result<Option<String>, AppError> {
    Ok(store::resolve_id(pool, user_id, id_or_title).await?)
}

fn cfg_fields(cfg: &TrackConfig) -> Value {
    json!({
        "title": cfg.title,
        "bpm": cfg.bpm,
        "steps": cfg.steps,
        "tuning": cfg.tuning,
        "voices": cfg.voices.len(),
        "kinds": cfg.voices.iter().map(|v| v.kind.clone()).collect::<Vec<_>>(),
    })
}

/* ── studio_list ────────────────────────────────────────────── */

pub struct StudioList;

#[async_trait]
impl Tool for StudioList {
    fn name(&self) -> &str { "studio_list" }
    fn aliases(&self) -> &[&str] { &["list_tracks", "tracks"] }
    fn step_label(&self) -> &str { "Listing studio tracks…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `studio_list` — List the user's studio tracks. params: `{}` — returns `tracks` (each with `track_id`, `title`, `bpm`, `steps`, `tuning`, `duration_ms`, `has_audio`, `kinds`, `updated_at`) and `count`.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let n = data.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
        format!("Found {n} studio tracks")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let rows = store::list(ctx.pool().await, req.traveler_id).await?;
        let tracks: Vec<Value> = rows.iter().map(store::meta_json).collect();
        Ok(ActionOutcome::ok(
            "studio_list",
            json!({ "tracks": tracks, "count": tracks.len() }),
        ))
    }
}

/* ── studio_create ──────────────────────────────────────────── */

pub struct StudioCreate;

#[async_trait]
impl Tool for StudioCreate {
    fn name(&self) -> &str { "studio_create" }
    fn aliases(&self) -> &[&str] { &["make_beat", "compose_track", "new_track"] }
    fn step_label(&self) -> &str { "Composing a studio track…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `studio_create` — Compose and render a track to audio. params: `{ title?, bpm?, steps?, tuning?, voices: [{ kind, rhythm, degree?, octave?, wave?, notes?, synth?, midi?, fx?, grid? }] }` — `kind` is one of kick/snare/hat/clap/tom/perc/bass/pluck/lead/pad/sub/organ/ep/bell/strings/brass/synthme/grid/drumkit; `rhythm` is `\"e<hits>,<rot>\"` (Euclidean) or an `\"x..x\"` string. Use `synthme` (custom synth via `synth`+`midi`+`fx`) or `grid` (WaveMe modular patch via `grid:{modules,cables}`). Returns the new track's metadata (`track_id`, `duration_ms`, `has_audio`).")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("track");
        format!("Composed \"{title}\"")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let cfg = engine::parse_config(req.params).map_err(AppError::BadRequest)?;
        let rendered = engine::render_track(&cfg).map_err(AppError::BadRequest)?;

        let id = uuid::Uuid::new_v4().to_string();
        let cfg_json = serde_json::to_string(&cfg).map_err(AppError::from)?;
        let title = if cfg.title.trim().is_empty() { "Untitled".into() } else { cfg.title.trim().to_string() };
        store::insert(
            ctx.pool().await,
            &id,
            req.traveler_id,
            &cfg_json,
            &title,
            cfg.bpm,
            cfg.steps as i64,
            &cfg.tuning,
            rendered.duration_ms as i64,
            &rendered.wav,
        )
        .await?;

        let mut data = json!({
            "track_id": id,
            "title": title,
            "bpm": cfg.bpm,
            "steps": cfg.steps,
            "tuning": cfg.tuning,
            "duration_ms": rendered.duration_ms,
            "has_audio": true,
            "sample_rate": rendered.sample_rate,
        });
        if let Some(obj) = data.as_object_mut() {
            obj.insert("voices".into(), cfg_fields(&cfg));
        }
        Ok(ActionOutcome::ok("studio_create", data))
    }
}

/* ── studio_get ─────────────────────────────────────────────── */

pub struct StudioGet;

#[async_trait]
impl Tool for StudioGet {
    fn name(&self) -> &str { "studio_get" }
    fn aliases(&self) -> &[&str] { &["get_track", "track_info"] }
    fn step_label(&self) -> &str { "Loading studio track…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `studio_get` — Metadata for one track. params: `{ track_id }` — accepts the UUID or the exact title. Returns the track metadata plus its `config` (voices).")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("track");
        format!("Loaded \"{title}\"")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let pool = ctx.pool().await;
        let id = resolve_track_id(pool, req.traveler_id, req.params.param_str("track_id")).await?;
        let Some(id) = id else {
            return Err(AppError::NotFound("studio track not found".into()));
        };
        let row = store::get(pool, req.traveler_id, &id).await?;
        let Some(row) = row else {
            return Err(AppError::NotFound("studio track not found".into()));
        };
        Ok(ActionOutcome::ok("studio_get", store::full_json(&row)))
    }
}

/* ── studio_render ──────────────────────────────────────────── */

pub struct StudioRender;

#[async_trait]
impl Tool for StudioRender {
    fn name(&self) -> &str { "studio_render" }
    fn aliases(&self) -> &[&str] { &["render_track", "re_render"] }
    fn step_label(&self) -> &str { "Rendering studio track…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `studio_render` — Re-render a stored track to audio. params: `{ track_id }` — re-renders from its saved config and returns `{ track_id, duration_ms, has_audio }`.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("track");
        format!("Rendered \"{title}\"")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let pool = ctx.pool().await;
        let id = resolve_track_id(pool, req.traveler_id, req.params.param_str("track_id")).await?;
        let Some(id) = id else {
            return Err(AppError::NotFound("studio track not found".into()));
        };
        let row = store::get(pool, req.traveler_id, &id).await?;
        let Some(row) = row else {
            return Err(AppError::NotFound("studio track not found".into()));
        };
        let cfg: TrackConfig = serde_json::from_str(&row.7).map_err(AppError::from)?;
        let rendered = engine::render_track(&cfg).map_err(AppError::BadRequest)?;
        store::update_render(pool, &id, req.traveler_id, rendered.duration_ms as i64, &rendered.wav).await?;

        Ok(ActionOutcome::ok(
            "studio_render",
            json!({ "track_id": id, "title": row.1, "duration_ms": rendered.duration_ms, "has_audio": true }),
        ))
    }
}

/* ── studio_delete ──────────────────────────────────────────── */

pub struct StudioDelete;

#[async_trait]
impl Tool for StudioDelete {
    fn name(&self) -> &str { "studio_delete" }
    fn aliases(&self) -> &[&str] { &["delete_track", "remove_track"] }
    fn step_label(&self) -> &str { "Deleting studio track…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `studio_delete` — Permanently delete a track (requires `{ confirm: true }`). params: `{ track_id, confirm: true }`.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("track");
        format!("Deleted \"{title}\"")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        if !req.params.param_bool("confirm").unwrap_or(false) {
            return Err(AppError::BadRequest("confirm required — set `confirm: true` to delete".into()));
        }
        let pool = ctx.pool().await;
        let id = resolve_track_id(pool, req.traveler_id, req.params.param_str("track_id")).await?;
        let Some(id) = id else {
            return Err(AppError::NotFound("studio track not found".into()));
        };
        let title = store::delete(pool, &id, req.traveler_id).await?;
        Ok(ActionOutcome::ok(
            "studio_delete",
            json!({ "track_id": id, "title": title.unwrap_or_default() }),
        ))
    }
}

/* ── studio_preset_list ─────────────────────────────────────── */

pub struct StudioPresetList;

#[async_trait]
impl Tool for StudioPresetList {
    fn name(&self) -> &str { "studio_preset_list" }
    fn aliases(&self) -> &[&str] { &["list_presets", "presets"] }
    fn step_label(&self) -> &str { "Listing studio presets…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `studio_preset_list` — List saved presets (instruments, SynthMe synths, WaveMe patches). params: `{}` — returns `presets` (each `{ id, kind, name, params }`) and `count`.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let n = data.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
        format!("Found {n} studio presets")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let rows = store::list_presets(ctx.pool().await, req.traveler_id).await?;
        let presets: Vec<Value> = rows.iter().map(store::preset_json).collect();
        Ok(ActionOutcome::ok(
            "studio_preset_list",
            json!({ "presets": presets, "count": presets.len() }),
        ))
    }
}

/* ── studio_preset_save ─────────────────────────────────────── */

pub struct StudioPresetSave;

#[async_trait]
impl Tool for StudioPresetSave {
    fn name(&self) -> &str { "studio_preset_save" }
    fn aliases(&self) -> &[&str] { &["save_preset", "save_instrument"] }
    fn step_label(&self) -> &str { "Saving studio preset…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `studio_preset_save` — Save a reusable preset. params: `{ kind, name, params }` — `kind` is an instrument kind, `synthme` (custom synth), or `grid` (WaveMe patch); `params` is the same object a voice uses (`synth`/`midi`/`fx` for synthme, `grid` for WaveMe). Returns `{ id, kind, name }`.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let name = data.get("name").and_then(|v| v.as_str()).unwrap_or("preset");
        format!("Saved preset \"{name}\"")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let kind = req.params.param_str("kind").unwrap_or_default();
        let kind = if kind.trim().is_empty() { "synthme".to_string() } else { kind.trim().to_string() };
        let name = req.params.param_str("name").unwrap_or_default();
        let name = if name.trim().is_empty() { "Preset".to_string() } else { name.trim().to_string() };
        let params = req.params.get("params").cloned().unwrap_or(json!({}));
        let params_json = serde_json::to_string(&params).map_err(AppError::from)?;
        let id = uuid::Uuid::new_v4().to_string();
        store::insert_preset(ctx.pool().await, &id, req.traveler_id, &kind, &name, &params_json).await?;
        Ok(ActionOutcome::ok("studio_preset_save", json!({ "id": id, "kind": kind, "name": name })))
    }
}

/* ── studio_preset_delete ───────────────────────────────────── */

pub struct StudioPresetDelete;

#[async_trait]
impl Tool for StudioPresetDelete {
    fn name(&self) -> &str { "studio_preset_delete" }
    fn aliases(&self) -> &[&str] { &["delete_preset"] }
    fn step_label(&self) -> &str { "Deleting studio preset…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `studio_preset_delete` — Delete a saved preset. params: `{ id }` — the preset `id` from `studio_preset_list`.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let name = data.get("name").and_then(|v| v.as_str()).unwrap_or("preset");
        format!("Deleted preset \"{name}\"")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let id = req.params.param_str("id").ok_or_else(|| AppError::BadRequest("id required".into()))?;
        let name = store::delete_preset(ctx.pool().await, &id, req.traveler_id).await?;
        match name {
            Some(n) => Ok(ActionOutcome::ok("studio_preset_delete", json!({ "id": id, "name": n }))),
            None => Err(AppError::NotFound("preset not found".into())),
        }
    }
}

/* ── studio_arrangement_list ────────────────────────────────── */

pub struct StudioArrangementList;

#[async_trait]
impl Tool for StudioArrangementList {
    fn name(&self) -> &str { "studio_arrangement_list" }
    fn aliases(&self) -> &[&str] { &["list_arrangements", "arrangements"] }
    fn step_label(&self) -> &str { "Listing studio arrangements…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `studio_arrangement_list` — List the user's arrangements. params: `{}` — returns `arrangements` (each `{ id, title, bpm, length_beats, master, tracks, clips, updated_at }`) and `count`.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let n = data.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
        format!("Found {n} arrangements")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let rows = store::list_arrangements(ctx.pool().await, req.traveler_id).await?;
        let arrangements: Vec<Value> = rows.iter().map(store::arr_meta_json).collect();
        Ok(ActionOutcome::ok(
            "studio_arrangement_list",
            json!({ "arrangements": arrangements, "count": arrangements.len() }),
        ))
    }
}

/* ── studio_arrangement_save ────────────────────────────────── */

pub struct StudioArrangementSave;

#[async_trait]
impl Tool for StudioArrangementSave {
    fn name(&self) -> &str { "studio_arrangement_save" }
    fn aliases(&self) -> &[&str] { &["save_arrangement", "create_arrangement"] }
    fn step_label(&self) -> &str { "Saving studio arrangement…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `studio_arrangement_save` — Create (or update, when `id` is given) an arrangement. params: `{ title?, bpm?, length_beats?, master?, tracks: [{ id, name?, color?, mute?, level?, pan?, automation? }], clips: [{ track, start, pattern }], id? }` — `pattern` is a track config (same voice shape as `studio_create`). Returns `{ id, title }`.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("arrangement");
        format!("Saved arrangement \"{title}\"")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let arr = engine::parse_arrangement(req.params).map_err(AppError::BadRequest)?;
        let cfg_json = serde_json::to_string(&arr).map_err(AppError::from)?;
        let title = if arr.title.trim().is_empty() { "Untitled".into() } else { arr.title.trim().to_string() };
        let pool = ctx.pool().await;

        if let Some(id) = req.params.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()) {
            let changed = store::update_arrangement(pool, &id, req.traveler_id, &title, arr.bpm, arr.length_beats, arr.master as f64, &cfg_json).await?;
            if !changed {
                return Err(AppError::NotFound("arrangement not found".into()));
            }
            Ok(ActionOutcome::ok("studio_arrangement_save", json!({ "id": id, "title": title })))
        } else {
            let id = uuid::Uuid::new_v4().to_string();
            store::insert_arrangement(pool, &id, req.traveler_id, &title, arr.bpm, arr.length_beats, arr.master as f64, &cfg_json).await?;
            Ok(ActionOutcome::ok("studio_arrangement_save", json!({ "id": id, "title": title })))
        }
    }
}

/* ── studio_arrangement_get ─────────────────────────────────── */

pub struct StudioArrangementGet;

#[async_trait]
impl Tool for StudioArrangementGet {
    fn name(&self) -> &str { "studio_arrangement_get" }
    fn aliases(&self) -> &[&str] { &["get_arrangement"] }
    fn step_label(&self) -> &str { "Loading studio arrangement…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `studio_arrangement_get` — Full arrangement (tracks + clips + automation) by id. params: `{ id }`.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("arrangement");
        format!("Loaded arrangement \"{title}\"")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let id = req.params.param_str("id").ok_or_else(|| AppError::BadRequest("id required".into()))?;
        let row = store::get_arrangement(ctx.pool().await, req.traveler_id, &id).await?;
        match row {
            Some(r) => Ok(ActionOutcome::ok("studio_arrangement_get", store::arr_full_json(&r))),
            None => Err(AppError::NotFound("arrangement not found".into())),
        }
    }
}

/* ── studio_arrangement_delete ──────────────────────────────── */

pub struct StudioArrangementDelete;

#[async_trait]
impl Tool for StudioArrangementDelete {
    fn name(&self) -> &str { "studio_arrangement_delete" }
    fn aliases(&self) -> &[&str] { &["delete_arrangement"] }
    fn step_label(&self) -> &str { "Deleting studio arrangement…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `studio_arrangement_delete` — Delete an arrangement (needs `{ confirm: true }`). params: `{ id, confirm: true }`.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("arrangement");
        format!("Deleted arrangement \"{title}\"")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        if !req.params.param_bool("confirm").unwrap_or(false) {
            return Err(AppError::BadRequest("confirm required — set `confirm: true` to delete".into()));
        }
        let id = req.params.param_str("id").ok_or_else(|| AppError::BadRequest("id required".into()))?;
        let title = store::delete_arrangement(ctx.pool().await, &id, req.traveler_id).await?;
        match title {
            Some(t) => Ok(ActionOutcome::ok("studio_arrangement_delete", json!({ "id": id, "title": t }))),
            None => Err(AppError::NotFound("arrangement not found".into())),
        }
    }
}
