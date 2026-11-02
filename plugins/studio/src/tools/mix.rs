//! Agent tools for the mixer, audio import (AI-generated songs) and export.
//!
//! These complete the AI surface: the model can place a `generate_song`
//! result on a track, edit an arrangement with small operations instead of
//! resending it whole, measure every track of a mix, and export the master
//! into the user's Files. Each tool that changes an arrangement reports its
//! `id` so the Studio window reloads it live.

use async_trait::async_trait;
use serde_json::{json, Value};

use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::outcome::ActionOutcome;
use shiny_plugin_sdk::services::PluginCtx;
use shiny_plugin_sdk::tools::{ParamHelpers, Tool, ToolRequest};
use shiny_plugin_sdk::Notification;

use crate::engine;
use crate::routes_mix;
use crate::store;

fn home(req: &ToolRequest<'_>) -> std::path::PathBuf {
    routes_mix::files_home(req.traveler_id, req.os_home)
}

/// Load an arrangement by id/title, or the most recently edited one.
async fn load(ctx: &PluginCtx, req: &ToolRequest<'_>) -> Result<(String, Value), AppError> {
    let pool = ctx.pool().await;
    let wanted = req.params.param_str("arrangement_id").or_else(|| req.params.param_str("id"));
    let id = match wanted {
        Some(w) => store::resolve_arrangement_id(pool, req.traveler_id, &w)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("arrangement `{w}` not found")))?,
        None => store::list_arrangements(pool, req.traveler_id)
            .await?
            .into_iter()
            .next()
            .map(|r| r.0)
            .ok_or_else(|| AppError::NotFound("there is no arrangement yet — create one with studio_arrangement_save".into()))?,
    };
    let row = store::get_arrangement(pool, req.traveler_id, &id)
        .await?
        .ok_or_else(|| AppError::NotFound("arrangement not found".into()))?;
    Ok((id, store::arr_full_json(&row)))
}

/// Validate and store an arrangement value.
async fn save(ctx: &PluginCtx, req: &ToolRequest<'_>, id: &str, value: &Value) -> Result<engine::Arrangement, AppError> {
    let arr = engine::parse_arrangement(value).map_err(AppError::BadRequest)?;
    let cfg = serde_json::to_string(&arr).map_err(AppError::from)?;
    let title = if arr.title.trim().is_empty() { "Untitled".to_string() } else { arr.title.trim().to_string() };
    let pool = ctx.pool().await;
    store::update_arrangement(pool, id, req.traveler_id, &title, arr.bpm, arr.length_beats, arr.master as f64, &cfg).await?;
    Ok(arr)
}

fn summary(arr: &engine::Arrangement) -> Value {
    json!({
        "title": arr.title,
        "bpm": arr.bpm,
        "length_beats": arr.length_beats,
        "tracks": arr.tracks.iter().map(|t| json!({
            "id": t.id, "name": t.name, "kind": t.kind, "level": t.level, "pan": t.pan,
            "mute": t.mute, "solo": t.solo, "sends": t.sends, "fx": t.fx.iter().map(|e| e.kind.clone()).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "clips": arr.clips.iter().enumerate().map(|(i, c)| json!({
            "index": i, "track": c.track, "start": c.start, "length_beats": c.length_beats,
            "audio": c.audio.as_ref().map(|a| a.src.clone()),
            "pattern": if c.audio.is_none() { Some(c.pattern.title.clone()) } else { None },
        })).collect::<Vec<_>>(),
    })
}

/* ── studio_import_audio ────────────────────────────────────── */

pub struct StudioImportAudio;

#[async_trait]
impl Tool for StudioImportAudio {
    fn name(&self) -> &str { "studio_import_audio" }
    fn aliases(&self) -> &[&str] { &["import_song", "add_audio_track", "studio_add_song"] }
    fn step_label(&self) -> &str { "Placing audio in the Studio…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `studio_import_audio` — Put a WAV (e.g. the `path` a `generate_song` call returned, or any file under Music/) on a new audio track of an arrangement. params: `{ path, arrangement_id?, track_name?, start_beat?, match_tempo?, level?, create? }` — omit `arrangement_id` to use the most recent one (or `create:true` to start a new arrangement around the song). `match_tempo:true` sets the project BPM to the song's detected tempo. Returns the arrangement `id`, the new `track_id`, the song `duration`/`bpm`.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let name = data.get("track_name").and_then(|v| v.as_str()).unwrap_or("audio");
        format!("Added \"{name}\" to the Studio")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let path = req.params.param_str("path").ok_or_else(|| AppError::BadRequest("path required (relative to the Files home, e.g. Music/Generated/song.wav)".into()))?;
        let path = path.trim().trim_start_matches("~/").to_string();
        let home = home(&req);
        let info = {
            let (h, p) = (home.clone(), path.clone());
            tokio::task::spawn_blocking(move || routes_mix::audio_info_value(&h, &p, 64))
                .await
                .map_err(|e| AppError::Internal(format!("audio task failed: {e}")))??
        };
        let duration = info["duration"].as_f64().unwrap_or(0.0);
        let song_bpm = info["bpm"].as_f64();
        let name = req
            .params
            .param_str("track_name")
            .unwrap_or_else(|| path.rsplit('/').next().unwrap_or("Audio").trim_end_matches(".wav").to_string());

        let create = req.params.get("create").and_then(|v| v.as_bool()).unwrap_or(false);
        let pool = ctx.pool().await;
        let (id, mut value) = if create {
            let id = uuid::Uuid::new_v4().to_string();
            let bpm = song_bpm.unwrap_or(120.0);
            let v = json!({ "title": name, "bpm": bpm, "length_beats": 16, "tracks": [], "clips": [],
                            "fx": { "ceiling": -1.0 } });
            let arr = engine::parse_arrangement(&v).map_err(AppError::BadRequest)?;
            let cfg = serde_json::to_string(&arr).map_err(AppError::from)?;
            store::insert_arrangement(pool, &id, req.traveler_id, &name, arr.bpm, arr.length_beats, arr.master as f64, &cfg).await?;
            (id, v)
        } else {
            load(ctx, &req).await?
        };

        if req.params.get("match_tempo").and_then(|v| v.as_bool()).unwrap_or(create) {
            if let Some(b) = song_bpm {
                value["bpm"] = json!(b);
            }
        }
        let bpm = value["bpm"].as_f64().unwrap_or(120.0).clamp(40.0, 240.0);
        let start = req.params.get("start_beat").and_then(|v| v.as_f64()).unwrap_or(0.0).max(0.0);
        let beats = duration * bpm / 60.0;
        let track_id = format!("audio-{}", &uuid::Uuid::new_v4().to_string()[..8]);
        let level = req.params.get("level").and_then(|v| v.as_f64()).unwrap_or(0.8);
        value["tracks"].as_array_mut().ok_or_else(|| AppError::Internal("bad arrangement".into()))?.push(json!({
            "id": track_id, "name": name, "kind": "audio", "level": level, "pan": 0.0,
            "color": 4, "sends": { "reverb": 0.0, "delay": 0.0 },
        }));
        value["clips"].as_array_mut().ok_or_else(|| AppError::Internal("bad arrangement".into()))?.push(json!({
            "track": track_id, "start": start,
            "audio": { "src": path, "name": name, "fade_in": 0.005, "fade_out": 0.05, "bpm": song_bpm },
        }));
        let needed = (start + beats).ceil();
        if value["length_beats"].as_f64().unwrap_or(16.0) < needed {
            value["length_beats"] = json!(needed.min(engine::MAX_BEATS));
        }
        let arr = save(ctx, &req, &id, &value).await?;
        Ok(ActionOutcome::ok(
            "studio_import_audio",
            json!({
                "id": id, "arrangement_id": id, "title": arr.title, "track_id": track_id, "track_name": name,
                "path": path, "duration": duration, "bpm": song_bpm, "project_bpm": arr.bpm,
                "start_beat": start, "length_beats": arr.length_beats,
            }),
        )
        .with_notification(Notification::new(format!("\"{name}\" → {}", arr.title)).title("Studio — audio added").plugin("studio")))
    }
}

/* ── studio_arrangement_edit ────────────────────────────────── */

pub struct StudioArrangementEdit;

fn track_index(v: &Value, key: &str) -> Result<usize, AppError> {
    let tracks = v["tracks"].as_array().ok_or_else(|| AppError::BadRequest("arrangement has no tracks".into()))?;
    tracks
        .iter()
        .position(|t| t["id"].as_str() == Some(key) || t["name"].as_str().map(|n| n.eq_ignore_ascii_case(key)).unwrap_or(false))
        .ok_or_else(|| AppError::BadRequest(format!("no track `{key}` (use a track id or name)")))
}

fn merge(dst: &mut Value, src: &Value, keys: &[&str]) {
    for k in keys {
        if let Some(v) = src.get(*k) {
            dst[*k] = v.clone();
        }
    }
}

/// Apply one edit operation to an arrangement value.
fn apply_op(v: &mut Value, op: &Value) -> Result<String, AppError> {
    let kind = op["op"].as_str().ok_or_else(|| AppError::BadRequest("each op needs an `op` field".into()))?;
    match kind {
        "set" => {
            merge(v, op, &["title", "bpm", "length_beats", "master", "master_fx"]);
            if let Some(fx) = op.get("fx").and_then(|f| f.as_object()) {
                if !v["fx"].is_object() {
                    v["fx"] = json!({});
                }
                for (k, val) in fx {
                    v["fx"][k] = val.clone();
                }
            }
            if let Some(r) = op.get("returns") {
                v["returns"] = r.clone();
            }
            Ok("set".into())
        }
        "add_track" => {
            let id = op["id"].as_str().map(str::to_string).unwrap_or_else(|| format!("t-{}", &uuid::Uuid::new_v4().to_string()[..8]));
            let mut t = json!({ "id": id, "name": op["name"].as_str().unwrap_or("Track"), "level": 0.8, "pan": 0.0 });
            merge(&mut t, op, &["kind", "level", "pan", "mute", "solo", "sends", "fx", "color", "automation"]);
            if let Some(a) = v["tracks"].as_array_mut() {
                a.push(t);
            }
            Ok(format!("added track {id}"))
        }
        "update_track" => {
            let key = op["track"].as_str().ok_or_else(|| AppError::BadRequest("update_track needs `track`".into()))?;
            let i = track_index(v, key)?;
            merge(&mut v["tracks"][i], op, &["name", "kind", "level", "pan", "mute", "solo", "sends", "fx", "color"]);
            Ok(format!("updated track {key}"))
        }
        "remove_track" => {
            let key = op["track"].as_str().ok_or_else(|| AppError::BadRequest("remove_track needs `track`".into()))?;
            let i = track_index(v, key)?;
            let id = v["tracks"][i]["id"].as_str().unwrap_or_default().to_string();
            if let Some(a) = v["tracks"].as_array_mut() {
                a.remove(i);
            }
            if let Some(a) = v["clips"].as_array_mut() {
                a.retain(|c| c["track"].as_str() != Some(&id));
            }
            Ok(format!("removed track {key}"))
        }
        "add_clip" => {
            let key = op["track"].as_str().ok_or_else(|| AppError::BadRequest("add_clip needs `track`".into()))?;
            let i = track_index(v, key)?;
            let id = v["tracks"][i]["id"].clone();
            let mut c = json!({ "track": id, "start": op["start"].as_f64().unwrap_or(0.0) });
            merge(&mut c, op, &["length_beats", "gain_db", "loop", "offset_beats", "pattern", "audio"]);
            if c.get("pattern").is_none() && c.get("audio").is_none() {
                return Err(AppError::BadRequest("add_clip needs a `pattern` or an `audio`".into()));
            }
            if let Some(a) = v["clips"].as_array_mut() {
                a.push(c);
            }
            Ok("added clip".into())
        }
        "update_clip" | "remove_clip" => {
            let idx = op["index"].as_u64().ok_or_else(|| AppError::BadRequest(format!("{kind} needs a clip `index`")))? as usize;
            let len = v["clips"].as_array().map(|a| a.len()).unwrap_or(0);
            if idx >= len {
                return Err(AppError::BadRequest(format!("no clip #{idx} (there are {len})")));
            }
            if kind == "remove_clip" {
                if let Some(a) = v["clips"].as_array_mut() {
                a.remove(idx);
            }
                return Ok(format!("removed clip {idx}"));
            }
            if let Some(key) = op["track"].as_str() {
                let i = track_index(v, key)?;
                v["clips"][idx]["track"] = v["tracks"][i]["id"].clone();
            }
            merge(&mut v["clips"][idx], op, &["start", "length_beats", "gain_db", "loop", "offset_beats", "pattern", "audio"]);
            Ok(format!("updated clip {idx}"))
        }
        "set_automation" => {
            let key = op["track"].as_str().ok_or_else(|| AppError::BadRequest("set_automation needs `track`".into()))?;
            let param = op["param"].as_str().ok_or_else(|| AppError::BadRequest("set_automation needs `param`".into()))?;
            let i = track_index(v, key)?;
            let t = &mut v["tracks"][i];
            if !t["automation"].is_object() {
                t["automation"] = json!({ "lanes": [] });
            }
            if !t["automation"]["lanes"].is_array() {
                t["automation"]["lanes"] = json!([]);
            }
            let lanes = t["automation"]["lanes"].as_array_mut().expect("array");
            lanes.retain(|l| l["param"].as_str() != Some(param));
            let points = op.get("points").cloned().unwrap_or(json!([]));
            if points.as_array().map(|p| !p.is_empty()).unwrap_or(false) {
                lanes.push(json!({ "param": param, "points": points }));
            }
            Ok(format!("automation {param} on {key}"))
        }
        other => Err(AppError::BadRequest(format!(
            "unknown op `{other}` (set, add_track, update_track, remove_track, add_clip, update_clip, remove_clip, set_automation)"
        ))),
    }
}

#[async_trait]
impl Tool for StudioArrangementEdit {
    fn name(&self) -> &str { "studio_arrangement_edit" }
    fn aliases(&self) -> &[&str] { &["edit_arrangement", "studio_edit"] }
    fn step_label(&self) -> &str { "Editing the arrangement…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `studio_arrangement_edit` — Change an arrangement with small operations (no need to resend it). params: `{ arrangement_id?, ops: [ … ] }` (default: the most recent arrangement). Ops: `{op:\"set\", title?, bpm?, length_beats?, master?, fx?:{…merged}, master_fx?, returns?}`, `{op:\"add_track\", name, kind?, level?, pan?, sends?, fx?}`, `{op:\"update_track\", track, name?/level?/pan?/mute?/solo?/sends?/fx?}`, `{op:\"remove_track\", track}`, `{op:\"add_clip\", track, start, length_beats?, pattern?|audio?, gain_db?}`, `{op:\"update_clip\", index, …}`, `{op:\"remove_clip\", index}`, `{op:\"set_automation\", track, param, points:[{beat,value}]}`. `track` is a track id or name; clip `index` comes from `studio_arrangement_get`. Returns the updated summary.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let n = data.get("applied").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
        format!("Applied {n} arrangement edits")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let ops = req.params.get("ops").and_then(|v| v.as_array()).cloned().ok_or_else(|| AppError::BadRequest("ops required (an array)".into()))?;
        if ops.len() > 64 {
            return Err(AppError::BadRequest("at most 64 ops per call".into()));
        }
        let (id, mut value) = load(ctx, &req).await?;
        let mut applied = Vec::new();
        for op in &ops {
            applied.push(apply_op(&mut value, op)?);
        }
        let arr = save(ctx, &req, &id, &value).await?;
        let mut data = summary(&arr);
        data["id"] = json!(id);
        data["applied"] = json!(applied);
        Ok(ActionOutcome::ok("studio_arrangement_edit", data))
    }
}

/* ── studio_mix_analyze ─────────────────────────────────────── */

pub struct StudioMixAnalyze;

#[async_trait]
impl Tool for StudioMixAnalyze {
    fn name(&self) -> &str { "studio_mix_analyze" }
    fn aliases(&self) -> &[&str] { &["analyze_mix", "studio_arrangement_analyze"] }
    fn step_label(&self) -> &str { "Measuring the mix…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `studio_mix_analyze` — Render an arrangement and measure it: master `{ lufs, peak, rms_db, crest_db, clipped, range_lu, bands:[sub,low,mid,highmid,air] }`, the loudness `trim_db` applied, and per-track `{ name, lufs, peak, muted }` (post-fader). params: `{ arrangement_id? }` (default: the most recent). Use it to balance levels: lead elements a few LU above pads, kick/bass not drowning the rest, master near the `fx.loudness` target.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let lufs = data.pointer("/master/lufs").and_then(|v| v.as_f64()).unwrap_or(0.0);
        format!("Mix measured at {lufs:.1} LUFS")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let (id, value) = load(ctx, &req).await?;
        let arr = engine::parse_arrangement(&value).map_err(AppError::BadRequest)?;
        let h = home(&req);
        let mut data = tokio::task::spawn_blocking(move || routes_mix::analyze_value(&arr, &h))
            .await
            .map_err(|e| AppError::Internal(format!("analysis task failed: {e}")))??;
        data["id"] = json!(id);
        Ok(ActionOutcome::ok("studio_mix_analyze", data))
    }
}

/* ── studio_export ──────────────────────────────────────────── */

pub struct StudioExport;

#[async_trait]
impl Tool for StudioExport {
    fn name(&self) -> &str { "studio_export" }
    fn aliases(&self) -> &[&str] { &["export_song", "studio_bounce", "studio_arrangement_export"] }
    fn step_label(&self) -> &str { "Exporting the mix…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `studio_export` — Master an arrangement (limiter + its `fx.loudness` target) and save it as WAV in the user's Files under `Music/Studio/`. params: `{ arrangement_id?, name?, bits? (16|24) }`. Returns `{ path, lufs, peak, duration_ms }`.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let p = data.get("path").and_then(|v| v.as_str()).unwrap_or("Music/Studio");
        format!("Exported to {p}")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let (id, value) = load(ctx, &req).await?;
        let arr = engine::parse_arrangement(&value).map_err(AppError::BadRequest)?;
        let h = home(&req);
        let name = req.params.param_str("name");
        let bits = req.params.get("bits").and_then(|v| v.as_u64()).map(|b| b as u16);
        let mut data = tokio::task::spawn_blocking(move || routes_mix::export_to_home(&arr, &h, name.as_deref(), bits))
            .await
            .map_err(|e| AppError::Internal(format!("export task failed: {e}")))??;
        data["id"] = json!(id);
        let body = format!("{} · {:.1} LUFS", data["path"].as_str().unwrap_or(""), data["lufs"].as_f64().unwrap_or(0.0));
        Ok(ActionOutcome::ok("studio_export", data).with_notification(Notification::new(body).title("Studio — exported").plugin("studio")))
    }
}

/* ── studio_audio_info / studio_audio_list ──────────────────── */

pub struct StudioAudioInfo;

#[async_trait]
impl Tool for StudioAudioInfo {
    fn name(&self) -> &str { "studio_audio_info" }
    fn aliases(&self) -> &[&str] { &["audio_info", "song_tempo"] }
    fn step_label(&self) -> &str { "Reading the audio file…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `studio_audio_info` — Duration, sample rate, detected tempo (`bpm`, may be null) and loudness of a WAV in the user's Files. params: `{ path }`. Without `path` it lists the WAV files under Music/ (newest first; `generated:true` marks songs made by `generate_song`).")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        match data.get("bpm").and_then(|v| v.as_f64()) {
            Some(b) => format!("Audio at ~{b:.0} BPM"),
            None => "Read audio file".into(),
        }
    }

    async fn invoke(&self, _ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let h = home(&req);
        let path = req.params.param_str("path");
        let data = tokio::task::spawn_blocking(move || match path {
            Some(p) => routes_mix::audio_info_value(&h, p.trim().trim_start_matches("~/"), 32).map(|mut v| {
                if let Some(o) = v.as_object_mut() {
                    o.remove("peaks");
                }
                v
            }),
            None => {
                let files = routes_mix::library(&h);
                Ok(json!({ "files": files.into_iter().take(40).collect::<Vec<_>>() }))
            }
        })
        .await
        .map_err(|e| AppError::Internal(format!("audio task failed: {e}")))??;
        Ok(ActionOutcome::ok("studio_audio_info", data))
    }
}
