//! Local song generation (YuE2 through HOT-Step-CPP's `ace-server`): model
//! status, background downloads, and serving finished tracks to the chat.
//!
//! The heavy lifting lives in `crate::services::song_gen`; these handlers are
//! the Settings → Music surface plus the audio endpoint the chat's track
//! player embeds. Paths are namespaced `/api/songgen/*` on purpose — a future
//! music plugin owns the `/api/music/*` namespace.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Extension;
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use crate::api::AppState;
use crate::errors::AppError;
use crate::models::Traveler;

/// Longest byte window served for one ranged request. HTML media asks for the
/// next window as it plays; capping keeps a multi-minute song from costing the
/// whole file in RAM per request.
const RANGE_WINDOW: usize = 16 * 1024 * 1024;

/// One background song render started from the Studio window.
#[derive(Clone)]
struct SongJob {
    owner: String,
    title: String,
    status: &'static str,
    started: std::time::Instant,
    result: Option<serde_json::Value>,
    error: Option<String>,
}

fn jobs() -> &'static std::sync::Mutex<std::collections::HashMap<String, SongJob>> {
    static JOBS: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, SongJob>>> =
        std::sync::OnceLock::new();
    JOBS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

#[derive(Deserialize)]
pub struct SongGenerateBody {
    pub style: String,
    #[serde(default)]
    pub lyrics: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub duration: Option<u32>,
    #[serde(default)]
    pub quality: Option<String>,
    #[serde(default)]
    pub seed: Option<u64>,
}

/// POST /api/songgen/generate — start a song render in the background and
/// return its job id at once; the Studio polls `/api/songgen/jobs/:id`.
/// Missing weights are reported (with the download size) instead of being
/// fetched implicitly — the window asks the user first.
pub async fn generate(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
    Json(body): Json<SongGenerateBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    let style = body.style.trim().to_string();
    if style.is_empty() {
        return Err(AppError::BadRequest("style required".into()));
    }
    if state.song_gen.engine().is_none() {
        return Ok(Json(json!({
            "success": false,
            "error": "the song engine is not installed",
            "data": { "engine_missing": true }
        })));
    }
    let quality = match body.quality.filter(|q| !q.trim().is_empty()) {
        Some(q) => q,
        None => crate::services::agent_tools::music_quality_pref(&state, &traveler.id)
            .await
            .unwrap_or_else(|| crate::services::song_gen::DEFAULT_TIER.to_string()),
    };
    let tier = crate::services::song_gen::resolve_tier(&quality)
        .ok_or_else(|| AppError::BadRequest(format!("unknown song quality '{quality}' (use q4 or q8)")))?;
    let missing = state.song_gen.missing_for(tier.key);
    if !missing.is_empty() {
        let bytes = state.song_gen.missing_bytes_for(tier.key);
        return Ok(Json(json!({
            "success": false,
            "error": "the song models are not downloaded yet",
            "data": {
                "needs_download": true,
                "quality": tier.key,
                "missing": missing,
                "download_bytes": bytes,
                "download_size": crate::services::media::human_bytes(bytes),
            }
        })));
    }
    let title = body.title.map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).unwrap_or_else(|| style.clone());
    let os_home = state.os_identity_for(&traveler).map(|u| u.home);
    let home = crate::services::media::files_home(&traveler.id, os_home.as_deref());
    let req = crate::services::song_gen::SongRequest {
        style,
        lyrics: body.lyrics,
        title: title.clone(),
        duration_secs: body.duration.unwrap_or(90).clamp(30, 300),
        seed: body.seed,
        quality: tier.key.to_string(),
        home,
    };
    let id = uuid::Uuid::new_v4().to_string();
    if let Ok(mut map) = jobs().lock() {
        // Forget finished jobs older than a day.
        map.retain(|_, j| j.status == "running" || j.started.elapsed().as_secs() < 86_400);
        map.insert(
            id.clone(),
            SongJob { owner: traveler.id.clone(), title: title.clone(), status: "running", started: std::time::Instant::now(), result: None, error: None },
        );
    }
    let client = state.song_gen.clone();
    let job_id = id.clone();
    tokio::spawn(async move {
        let outcome = tokio::task::spawn_blocking(move || client.generate(&req)).await;
        let mut map = match jobs().lock() {
            Ok(m) => m,
            Err(_) => return,
        };
        let Some(job) = map.get_mut(&job_id) else { return };
        match outcome {
            Ok(Ok(res)) => {
                job.status = "done";
                job.result = Some(json!({
                    "path": res.rel_path,
                    "file": res.file,
                    "url": res.url,
                    "title": job.title,
                    "duration_secs": res.duration_secs,
                    "seconds": res.seconds,
                    "quality": res.quality,
                }));
            }
            Ok(Err(e)) => {
                job.status = "failed";
                job.error = Some(e);
            }
            Err(e) => {
                job.status = "failed";
                job.error = Some(format!("song task panicked: {e}"));
            }
        }
    });
    Ok(Json(json!({ "success": true, "data": { "job_id": id, "status": "running", "title": title } })))
}

/// GET /api/songgen/jobs/:id — status of a background song render.
pub async fn job(
    Extension(traveler): Extension<Traveler>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let job = jobs()
        .lock()
        .ok()
        .and_then(|m| m.get(&id).cloned())
        .filter(|j| j.owner == traveler.id)
        .ok_or_else(|| AppError::NotFound("song job not found".into()))?;
    Ok(Json(json!({
        "success": true,
        "data": {
            "job_id": id,
            "status": job.status,
            "title": job.title,
            "elapsed_secs": job.started.elapsed().as_secs(),
            "result": job.result,
            "error": job.error,
        }
    })))
}

#[derive(Deserialize)]
pub struct SongDownloadBody {
    pub quality: String,
}

/// GET /api/songgen/status — model inventory + live download progress.
pub async fn status(State(state): State<AppState>) -> Result<Json<serde_json::Value>, AppError> {
    Ok(Json(json!({
        "success": true,
        "data": {
            "inventory": state.song_gen.inventory(),
            "downloads": state.song_gen.downloads(),
        }
    })))
}

/// POST /api/songgen/download — start a background fetch of one tier (or
/// `all`). Returns immediately; Settings polls `/api/songgen/status`.
pub async fn download(
    State(state): State<AppState>,
    Json(body): Json<SongDownloadBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    let quality = body.quality.trim().to_lowercase();
    state
        .song_gen
        .start_download(&quality)
        .map_err(AppError::BadRequest)?;
    Ok(Json(json!({
        "success": true,
        "data": { "status": "downloading", "quality": quality }
    })))
}

/// GET /api/songgen/tracks/:name — the WAV the chat's track player embeds.
/// The file lives in the caller's Files home (`Music/Generated/`), so the name
/// is validated down to a bare file name before it ever touches the filesystem.
///
/// Ranged requests are served as bounded windows (`206`, `Accept-Ranges:
/// bytes`), the same contract as the Files plugin's `/api/files/raw`: media
/// elements ask with `Range: bytes=0-`, and a range-capable source is treated
/// as streamable/seekable instead of one that must be fetched whole.
pub async fn serve(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Result<axum::response::Response, AppError> {
    let valid = name.ends_with(".wav")
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains("..")
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if !valid {
        return Err(AppError::BadRequest("invalid track name".into()));
    }

    let os_home = state.os_identity_for(&traveler).map(|u| u.home);
    let path = crate::services::media::files_home(&traveler.id, os_home.as_deref())
        .join("Music")
        .join("Generated")
        .join(&name);
    let meta = tokio::fs::metadata(&path)
        .await
        .map_err(|_| AppError::NotFound("track not found".into()))?;
    let len = meta.len();

    if let Some((start, end)) = headers
        .get(axum::http::header::RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|r| parse_range(r, len as usize))
    {
        use tokio::io::{AsyncReadExt, AsyncSeekExt};
        let end = end.min(start.saturating_add(RANGE_WINDOW - 1));
        let mut file = tokio::fs::File::open(&path)
            .await
            .map_err(|_| AppError::NotFound("track not found".into()))?;
        file.seek(std::io::SeekFrom::Start(start as u64))
            .await
            .map_err(|e| AppError::Internal(format!("seek: {e}")))?;
        let mut buf = vec![0u8; end - start + 1];
        file.read_exact(&mut buf)
            .await
            .map_err(|e| AppError::Internal(format!("read: {e}")))?;
        return Ok(axum::response::Response::builder()
            .status(StatusCode::PARTIAL_CONTENT)
            .header("content-type", "audio/wav")
            .header("content-length", buf.len().to_string())
            .header("content-range", format!("bytes {start}-{end}/{len}"))
            .header("accept-ranges", "bytes")
            .header("cache-control", "private, max-age=3600")
            .body(axum::body::Body::from(buf))
            .map_err(|e| AppError::Internal(format!("response: {e}")))?);
    }

    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|_| AppError::NotFound("track not found".into()))?;
    Ok(axum::response::Response::builder()
        .header("content-type", "audio/wav")
        .header("content-length", bytes.len().to_string())
        .header("accept-ranges", "bytes")
        // Content-addressed by name (timestamped), so a short cache is safe and
        // keeps replaying a bubbled player from refetching.
        .header("cache-control", "private, max-age=3600")
        .body(axum::body::Body::from(bytes))
        .map_err(|e| AppError::Internal(format!("response: {e}")))?)
}

/// A single `Range: bytes=…` spec against a file of `len` bytes → inclusive
/// `(start, end)`. `None` for anything unparseable, multi-range, or out of
/// bounds, so the caller falls back to the full body.
fn parse_range(value: &str, len: usize) -> Option<(usize, usize)> {
    let spec = value.strip_prefix("bytes=")?;
    if spec.contains(',') {
        return None;
    }
    let (start_s, end_s) = spec.split_once('-')?;
    let (start, end) = if start_s.trim().is_empty() {
        // Suffix range: the last N bytes.
        let suffix: usize = end_s.trim().parse().ok()?;
        if suffix == 0 || len == 0 {
            return None;
        }
        (len.saturating_sub(suffix), len - 1)
    } else {
        let start: usize = start_s.trim().parse().ok()?;
        let end = if end_s.trim().is_empty() {
            len.saturating_sub(1)
        } else {
            end_s.trim().parse().ok()?
        };
        (start, end.min(len.saturating_sub(1)))
    };
    if len == 0 || start > end || start >= len {
        return None;
    }
    Some((start, end))
}

#[cfg(test)]
mod tests {
    use super::parse_range;

    #[test]
    fn ranges_parse_like_the_files_route() {
        assert_eq!(parse_range("bytes=0-", 1000), Some((0, 999)));
        assert_eq!(parse_range("bytes=100-199", 1000), Some((100, 199)));
        assert_eq!(parse_range("bytes=900-", 1000), Some((900, 999)));
        assert_eq!(parse_range("bytes=-100", 1000), Some((900, 999)));
        assert_eq!(parse_range("bytes=500-10000", 1000), Some((500, 999)));
        assert_eq!(parse_range("bytes=1000-", 1000), None);
        assert_eq!(parse_range("bytes=0-0", 1000), Some((0, 0)));
        assert_eq!(parse_range("bytes=0-1,5-6", 1000), None);
        assert_eq!(parse_range("items=0-10", 1000), None);
        assert_eq!(parse_range("bytes=-0", 1000), None);
        assert_eq!(parse_range("bytes=0-", 0), None);
    }
}
