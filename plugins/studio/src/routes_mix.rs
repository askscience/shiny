//! Mixer, export and audio-import routes.
//!
//! * `POST /api/studio/mix/stems` — render an arrangement into cached
//!   per-track stems + returns and report the master loudness; the window
//!   plays the stems through its live mixer.
//! * `GET  /api/studio/mix/stem/:key` — one cached stem as 16-bit WAV.
//! * `POST /api/studio/mix/analyze` — master + per-track measurements.
//! * `POST /api/studio/export` — master the arrangement and write it into the
//!   user's Files home (`Music/Studio/<title>.wav`).
//! * `GET  /api/studio/audio/library` — WAV files under `Music/`.
//! * `GET  /api/studio/audio/info?path=` — duration, rate, tempo, peaks.
//! * `GET  /api/studio/audio/file?path=` — the file itself (preview).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::{FromRequestParts, Request};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::response::Response;
use serde::Deserialize;
use serde_json::{json, Value};

use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::routes::{bridged_route, os_home_from_request, ok, path_params_from_request, user_id, RouteHandler};
use shiny_plugin_sdk::services::PluginCtx;

use crate::audio::{self, HomeResolver};
use crate::engine;
use crate::mix::{self, MixOptions};

const MAX_BODY: usize = 8 * 1024 * 1024;

/// The caller's Files home: their real OS home when bound to a Linux
/// account, else the virtual `~/.shiny/home/<id>` (same rule as the Files
/// plugin and core).
pub fn files_home(user_id: &str, os_home: Option<&str>) -> PathBuf {
    if let Some(home) = os_home.map(str::trim).filter(|h| !h.is_empty()) {
        return PathBuf::from(home);
    }
    let cleaned: String = user_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        .take(64)
        .collect();
    let name = if cleaned.is_empty() { "default".to_string() } else { cleaned };
    let os = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    os.join(".shiny").join("home").join(name)
}

fn request_home(req: &Request) -> Result<PathBuf, AppError> {
    let uid = user_id(req)?;
    Ok(files_home(&uid, os_home_from_request(req).as_deref()))
}

async fn blocking<T, F>(f: F) -> Result<T, AppError>
where
    F: FnOnce() -> Result<T, AppError> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| AppError::Internal(format!("studio worker task failed: {e}")))?
}

async fn read_json(req: Request) -> Result<Value, AppError> {
    let bytes = axum::body::to_bytes(req.into_body(), MAX_BODY)
        .await
        .map_err(|e| AppError::BadRequest(format!("invalid body: {e}")))?;
    serde_json::from_slice(&bytes).map_err(|e| AppError::BadRequest(format!("invalid json: {e}")))
}

async fn query<T: serde::de::DeserializeOwned + Send + 'static>(req: Request) -> Result<T, AppError> {
    let (mut parts, _) = req.into_parts();
    let q = axum::extract::Query::<T>::from_request_parts(&mut parts, &())
        .await
        .map_err(|e| AppError::BadRequest(format!("invalid query: {e}")))?;
    Ok(q.0)
}

fn wav_response(bytes: Vec<u8>, cache: &str) -> Result<Response, AppError> {
    Response::builder()
        .status(200)
        .header(CONTENT_TYPE, "audio/wav")
        .header(CACHE_CONTROL, cache)
        .body(axum::body::Body::from(bytes))
        .map_err(|e| AppError::Internal(format!("response build: {e}")))
}

pub fn handle(ctx: &Arc<PluginCtx>, tag: &str) -> Option<RouteHandler> {
    let _ = ctx;
    Some(match tag {
        "studio_mix_stems" => mix_stems(),
        "studio_mix_stem" => mix_stem(),
        "studio_mix_analyze" => mix_analyze(),
        "studio_export" => export(),
        "studio_audio_library" => audio_library(),
        "studio_audio_info" => audio_info(),
        "studio_audio_file" => audio_file(),
        _ => return None,
    })
}

/* ── stems ─────────────────────────────────────────────────── */

/// Encode a stem as 16-bit WAV, normalised to its own peak; the manifest
/// carries the `scale` that restores the true level.
fn stem_scale(peak: f32) -> f32 {
    if peak > 0.98 {
        peak / 0.98
    } else {
        1.0
    }
}

fn stems_manifest(a: &engine::Arrangement, m: &mix::MixResult) -> Value {
    let stems: Vec<Value> = m
        .tracks
        .iter()
        .map(|s| {
            let peak = s.stem.peak();
            json!({
                "track_id": s.track_id,
                "key": s.key,
                "url": format!("/api/studio/mix/stem/{}", s.key),
                "scale": stem_scale(peak),
                "silent": peak == 0.0,
                "lufs": s.lufs,
                "peak": s.peak,
            })
        })
        .collect();
    let ret = |r: &Option<(String, Arc<mix::Stem>)>, level: Option<f64>| match r {
        Some((k, s)) => json!({ "key": k, "url": format!("/api/studio/mix/stem/{k}"), "scale": stem_scale(s.peak()), "level": level.unwrap_or(1.0) }),
        None => Value::Null,
    };
    let report = crate::dsp::loudness::measure(&m.master.channels, m.sample_rate as f64);
    json!({
        "sample_rate": m.sample_rate,
        "frames": m.frames,
        "duration_ms": (m.frames as f64 / m.sample_rate as f64 * 1000.0).round(),
        "bpm": a.bpm,
        "stems": stems,
        "returns": { "reverb": ret(&m.reverb, a.returns.reverb_level), "delay": ret(&m.delay, a.returns.delay_level) },
        "master": {
            "lufs": report.integrated_lufs,
            "peak": report.peak,
            "pre_lufs": m.pre_lufs,
            "trim_db": m.trim_db,
            "ceiling": a.fx.get("ceiling").copied().unwrap_or(-1.0),
            "glue": a.fx.get("glue").copied().unwrap_or(0.0),
        },
        "tracks": mix::track_reports(a, m),
    })
}

/// Latest stems request number. The window asks for a new mix on every edit;
/// a request that is still waiting when a newer one arrives is dropped
/// (`superseded`) instead of rendering a mix nobody will hear.
static STEMS_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn mix_stems() -> RouteHandler {
    bridged_route(move |req: Request| async move {
        let home = request_home(&req)?;
        let value = read_json(req).await?;
        let a = engine::parse_arrangement(&value).map_err(AppError::BadRequest)?;
        let my_gen = STEMS_GEN.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        blocking(move || {
            // Wait for any running render, then check we are still wanted.
            drop(mix::wait_idle());
            if STEMS_GEN.load(std::sync::atomic::Ordering::SeqCst) != my_gen {
                return Ok(ok(json!({ "superseded": true })));
            }
            let res = HomeResolver { home };
            let m = mix::render_mix(&a, &res, &MixOptions::monitor()).map_err(AppError::BadRequest)?;
            Ok(ok(stems_manifest(&a, &m)))
        })
        .await
    })
}

fn mix_stem() -> RouteHandler {
    bridged_route(move |req: Request| async move {
        let _ = user_id(&req)?;
        let key = path_params_from_request(&req)
            .and_then(|p| p.into_iter().next())
            .map(|(_, v)| v)
            .ok_or_else(|| AppError::BadRequest("missing stem key".into()))?;
        let stem = mix::cached_stem(&key).ok_or_else(|| AppError::NotFound("stem expired — render the mix again".into()))?;
        let bytes = blocking(move || {
            let scale = stem_scale(stem.peak());
            let inv = 1.0 / scale;
            let l: Vec<f32> = stem.l.iter().map(|x| x * inv).collect();
            let r: Vec<f32> = stem.r.iter().map(|x| x * inv).collect();
            // 24-bit: a pre-fader stem may be turned up a lot in the mixer, so
            // keep its quiet parts clean.
            Ok(crate::wav::encode_wav_bits(&[l, r], stem.sample_rate, 24))
        })
        .await?;
        // Content-addressed: the key changes whenever the audio would.
        wav_response(bytes, "private, max-age=3600")
    })
}

fn mix_analyze() -> RouteHandler {
    bridged_route(move |req: Request| async move {
        let home = request_home(&req)?;
        let value = read_json(req).await?;
        let a = engine::parse_arrangement(&value).map_err(AppError::BadRequest)?;
        blocking(move || Ok(ok(analyze_value(&a, &home)?))).await
    })
}

/// Master analysis + per-track loudness (shared with the agent tool).
pub fn analyze_value(a: &engine::Arrangement, home: &Path) -> Result<Value, AppError> {
    let res = HomeResolver { home: home.to_path_buf() };
    let m = mix::render_mix(a, &res, &MixOptions::monitor()).map_err(AppError::BadRequest)?;
    let master = engine::analyze_planar(&m.master);
    Ok(json!({ "master": master, "trim_db": m.trim_db, "tracks": mix::track_reports(a, &m) }))
}

/* ── export ────────────────────────────────────────────────── */

#[derive(Deserialize)]
struct ExportBody {
    arrangement: Value,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    bits: Option<u16>,
}

/// File-name slug: lowercase ASCII words joined by `-`.
pub fn slug(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
        if out.len() >= 60 {
            break;
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "studio-mix".into()
    } else {
        out
    }
}

/// Master an arrangement and write it under `Music/Studio/` in `home`.
/// Returns the export metadata (relative path, loudness, peak, duration).
pub fn export_to_home(a: &engine::Arrangement, home: &Path, name: Option<&str>, bits: Option<u16>) -> Result<Value, AppError> {
    let res = HomeResolver { home: home.to_path_buf() };
    let m = mix::render_mix(a, &res, &MixOptions::export()).map_err(AppError::BadRequest)?;
    let bits = bits.unwrap_or_else(|| engine::arrangement_bits(a));
    let r = engine::encode_rendered(&m.master, bits);
    let dir = home.join("Music").join("Studio");
    std::fs::create_dir_all(&dir).map_err(|e| AppError::Internal(format!("cannot create {}: {e}", dir.display())))?;
    let base = slug(name.unwrap_or(&a.title));
    let mut file = format!("{base}.wav");
    let mut n = 2;
    while dir.join(&file).exists() {
        file = format!("{base}-{n}.wav");
        n += 1;
    }
    std::fs::write(dir.join(&file), &r.wav).map_err(|e| AppError::Internal(format!("cannot write the export: {e}")))?;
    Ok(json!({
        "path": format!("Music/Studio/{file}"),
        "file": file,
        "lufs": r.lufs,
        "peak": r.peak,
        "duration_ms": r.duration_ms,
        "sample_rate": r.sample_rate,
        "bits": r.bits,
        "bytes": r.wav.len(),
    }))
}

fn export() -> RouteHandler {
    bridged_route(move |req: Request| async move {
        let home = request_home(&req)?;
        let value = read_json(req).await?;
        let body: ExportBody = serde_json::from_value(value).map_err(|e| AppError::BadRequest(format!("invalid export request: {e}")))?;
        let a = engine::parse_arrangement(&body.arrangement).map_err(AppError::BadRequest)?;
        blocking(move || Ok(ok(export_to_home(&a, &home, body.name.as_deref(), body.bits)?))).await
    })
}

/* ── audio library / info / file ───────────────────────────── */

/// WAV files under `<home>/Music` (three levels deep), newest first.
pub fn library(home: &Path) -> Vec<Value> {
    let root = home.join("Music");
    let mut out: Vec<(std::time::SystemTime, Value)> = Vec::new();
    let mut stack = vec![(root.clone(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let Ok(meta) = e.metadata() else { continue };
            if meta.is_dir() {
                if depth < 3 && !e.file_name().to_string_lossy().starts_with('.') {
                    stack.push((p, depth + 1));
                }
                continue;
            }
            let name = e.file_name().to_string_lossy().to_string();
            if !name.to_ascii_lowercase().ends_with(".wav") {
                continue;
            }
            let rel = p.strip_prefix(home).map(|r| r.to_string_lossy().replace('\\', "/")).unwrap_or_default();
            let modified = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
            let secs = modified.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            let generated = rel.starts_with("Music/Generated/");
            out.push((modified, json!({ "path": rel, "name": name, "bytes": meta.len(), "modified": secs, "generated": generated })));
        }
    }
    out.sort_by(|a, b| b.0.cmp(&a.0));
    out.into_iter().take(500).map(|(_, v)| v).collect()
}

fn audio_library() -> RouteHandler {
    bridged_route(move |req: Request| async move {
        let home = request_home(&req)?;
        let files = blocking(move || Ok(library(&home))).await?;
        Ok(ok(json!({ "files": files, "count": files.len() })))
    })
}

#[derive(Deserialize)]
struct PathQuery {
    path: String,
    #[serde(default)]
    buckets: Option<usize>,
}

/// Duration, format, tempo estimate and waveform peaks of a WAV file.
pub fn audio_info_value(home: &Path, path: &str, buckets: usize) -> Result<Value, AppError> {
    let p = audio::safe_join(home, path).map_err(AppError::BadRequest)?;
    let d = audio::load(&p).map_err(AppError::BadRequest)?;
    let peaks = audio::peaks(&d, buckets);
    let bpm = audio::estimate_bpm(&d);
    let report = crate::dsp::loudness::measure(&d.channels, d.sample_rate as f64);
    Ok(json!({
        "path": path,
        "sample_rate": d.sample_rate,
        "channels": d.channels.len(),
        "duration": d.frames() as f64 / d.sample_rate as f64,
        "bpm": bpm,
        "lufs": report.integrated_lufs,
        "peak": report.peak,
        "peaks": peaks,
    }))
}

fn audio_info() -> RouteHandler {
    bridged_route(move |req: Request| async move {
        let home = request_home(&req)?;
        let q: PathQuery = query(req).await?;
        let buckets = q.buckets.unwrap_or(800).clamp(16, 4096);
        blocking(move || Ok(ok(audio_info_value(&home, &q.path, buckets)?))).await
    })
}

fn audio_file() -> RouteHandler {
    bridged_route(move |req: Request| async move {
        let home = request_home(&req)?;
        let q: PathQuery = query(req).await?;
        let p = audio::safe_join(&home, &q.path).map_err(AppError::BadRequest)?;
        let meta = tokio::fs::metadata(&p).await.map_err(|_| AppError::NotFound("audio file not found".into()))?;
        if meta.len() > audio::MAX_FILE_BYTES {
            return Err(AppError::BadRequest("file too large".into()));
        }
        let bytes = tokio::fs::read(&p).await.map_err(|_| AppError::NotFound("audio file not found".into()))?;
        wav_response(bytes, "private, no-cache")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_is_file_safe() {
        assert_eq!(slug("Night Drive (v2)!"), "night-drive-v2");
        assert_eq!(slug("…"), "studio-mix");
    }
}
