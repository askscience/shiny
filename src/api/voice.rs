use std::path::PathBuf;
use std::process::Command;

use axum::extract::{Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::fs;

use crate::api::AppState;
use crate::errors::AppError;

#[derive(Deserialize)]
pub struct VoiceStatusQuery {
    pub lang: Option<String>,
}

#[derive(Serialize)]
pub struct VoiceStatusResponse {
    pub success: bool,
    pub vosk: String,
    pub supertonic: String,
    pub stt_lang: String,
    pub supertonic_lang: String,
    /// `ready` when the faster-whisper sidecar is answering, else `unavailable`.
    pub whisper: String,
    /// Engine the app should use unless the user overrode it in Settings.
    pub default_engine: String,
    pub whisper_default_model: String,
    /// Per-model presence/loaded state for the Settings → Voice panel.
    pub whisper_models: serde_json::Value,
    pub whisper_downloads: serde_json::Value,
}

#[derive(Deserialize)]
pub struct VoiceDownloadBody {
    pub lang: String,
}

#[derive(Deserialize)]
pub struct WhisperDownloadBody {
    pub model: String,
}

#[derive(Deserialize)]
pub struct SttChunkQuery {
    pub session: String,
    pub lang: Option<String>,
    pub model: Option<String>,
    /// Decoder bias (Whisper `initial_prompt`); the browser sends the wake
    /// phrase so an unusual assistant name survives the tiny model.
    pub prompt: Option<String>,
    #[serde(rename = "final", default)]
    pub is_final: bool,
}

#[derive(Deserialize)]
pub struct SttSessionQuery {
    pub session: String,
}

#[derive(Serialize)]
pub struct VoiceDownloadResponse {
    pub success: bool,
    pub data: serde_json::Value,
}

#[derive(Serialize)]
pub struct LanguagesResponse {
    pub success: bool,
    pub data: Vec<LanguageInfo>,
}

#[derive(Serialize)]
pub struct LanguageInfo {
    pub code: String,
    pub supertonic: String,
    pub vosk_available: bool,
    pub vosk_stt_lang: String,
}

#[derive(Deserialize)]
pub struct TtsRequest {
    pub text: String,
    pub lang: Option<String>,
    pub voice: Option<String>,
    pub speed: Option<f32>,
}

fn models_dir(config: &crate::config::Config) -> PathBuf {
    PathBuf::from(&config.vosk_models_dir)
}

fn resolve_stt_lang(lang: &str, map: &serde_json::Map<String, serde_json::Value>) -> (String, String) {
    let entry = map.get(lang).or_else(|| map.get("en"));
    let supertonic = entry
        .and_then(|e| e.get("supertonic"))
        .and_then(|v| v.as_str())
        .unwrap_or(lang)
        .to_string();

    if let Some(fallback) = entry.and_then(|e| e.get("vosk_stt_fallback")).and_then(|v| v.as_str()) {
        return (fallback.to_string(), supertonic);
    }
    if entry.and_then(|e| e.get("vosk_zip")).is_some() {
        return (lang.to_string(), supertonic);
    }
    ("en".to_string(), supertonic)
}

async fn load_lang_map() -> serde_json::Map<String, serde_json::Value> {
    let content = fs::read_to_string("voice/lang_map.json")
        .await
        .unwrap_or_else(|_| "{}".into());
    serde_json::from_str(&content).unwrap_or_default()
}

pub async fn voice_status(
    State(state): State<AppState>,
    Query(q): Query<VoiceStatusQuery>,
) -> Result<Json<VoiceStatusResponse>, AppError> {
    let lang = q.lang.unwrap_or_else(|| "en".into());
    let map = load_lang_map().await;
    let (stt_lang, supertonic_lang) = resolve_stt_lang(&lang, &map);

    let tar = models_dir(&state.config).join(format!("{}.tar.gz", stt_lang));
    let vosk = if tar.exists() { "ready" } else { "missing" };

    let supertonic = if state.supertonic.is_available().await {
        "ready"
    } else {
        "unavailable"
    };

    // One probe covers both "is it running" and "which models are loaded".
    let health = state.whisper.health().await;
    let whisper = if health.is_some() { "ready" } else { "unavailable" };

    Ok(Json(VoiceStatusResponse {
        success: true,
        vosk: vosk.into(),
        supertonic: supertonic.into(),
        stt_lang,
        supertonic_lang,
        whisper: whisper.into(),
        default_engine: "whisper".into(),
        whisper_default_model: "tiny".into(),
        whisper_models: state.whisper.inventory(health.as_ref()),
        whisper_downloads: state.whisper.downloads(),
    }))
}

pub async fn voice_download(
    State(state): State<AppState>,
    Json(body): Json<VoiceDownloadBody>,
) -> Result<Json<VoiceDownloadResponse>, AppError> {
    let dir = models_dir(&state.config);
    fs::create_dir_all(&dir).await.map_err(|e| AppError::Internal(e.to_string()))?;

    let output = Command::new("python3")
        .arg("voice/download_vosk.py")
        .arg(&body.lang)
        .env("VOSK_MODELS_DIR", &state.config.vosk_models_dir)
        .output()
        .map_err(|e| AppError::Internal(format!("Failed to run download script: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AppError::Internal(format!("Vosk download failed: {}", stderr)));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let data: serde_json::Value = stdout
        .lines()
        .last()
        .and_then(|l| serde_json::from_str(l).ok())
        .unwrap_or(json!({ "status": "ready" }));

    Ok(Json(VoiceDownloadResponse {
        success: true,
        data,
    }))
}

/// Start a background download of an optional faster-whisper model (`small`).
/// Returns immediately; the Settings page polls `/api/voice/status` for progress.
pub async fn voice_whisper_download(
    State(state): State<AppState>,
    Json(body): Json<WhisperDownloadBody>,
) -> Result<Json<VoiceDownloadResponse>, AppError> {
    let model = body.model.trim().to_lowercase();
    state
        .whisper
        .start_download(&model)
        .map_err(AppError::BadRequest)?;

    Ok(Json(VoiceDownloadResponse {
        success: true,
        data: json!({ "status": "downloading", "model": model }),
    }))
}

/// Forward one microphone chunk (or the final flush) to the STT sidecar.
/// The body is raw 16 kHz mono PCM16LE.
pub async fn voice_stt_chunk(
    State(state): State<AppState>,
    Query(q): Query<SttChunkQuery>,
    audio: Bytes,
) -> Result<Json<serde_json::Value>, AppError> {
    let value = state
        .whisper
        .chunk(
            &q.session,
            q.lang.as_deref(),
            q.model.as_deref(),
            q.prompt.as_deref(),
            q.is_final,
            audio,
        )
        .await
        .map_err(AppError::Internal)?;
    Ok(Json(value))
}

/// Drop a streaming session (microphone cancelled before a result).
pub async fn voice_stt_close(
    State(state): State<AppState>,
    Query(q): Query<SttSessionQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    // A cancelled session is best-effort: the sidecar already expires idle
    // sessions, so a dead connection must not surface as an error to the UI.
    let value = state
        .whisper
        .close(&q.session)
        .await
        .unwrap_or_else(|_| json!({ "text": "", "final": true }));
    Ok(Json(value))
}

pub async fn voice_languages() -> Result<Json<LanguagesResponse>, AppError> {
    let map = load_lang_map().await;
    let mut data: Vec<LanguageInfo> = map
        .iter()
        .map(|(code, entry)| {
            let supertonic = entry
                .get("supertonic")
                .and_then(|v| v.as_str())
                .unwrap_or(code.as_str())
                .to_string();
            let vosk_available = entry.get("vosk_zip").is_some();
            let vosk_stt_lang = entry
                .get("vosk_stt_fallback")
                .and_then(|v| v.as_str())
                .map(String::from)
                .unwrap_or_else(|| code.clone());
            LanguageInfo {
                code: code.clone(),
                supertonic,
                vosk_available,
                vosk_stt_lang,
            }
        })
        .collect();
    data.sort_by(|a, b| a.code.cmp(&b.code));
    Ok(Json(LanguagesResponse { success: true, data }))
}

pub async fn tts(
    State(state): State<AppState>,
    Json(body): Json<TtsRequest>,
) -> Result<Response, AppError> {
    let lang = body.lang.unwrap_or_else(|| "en".into());
    let map = load_lang_map().await;
    let (_, supertonic_lang) = resolve_stt_lang(&lang, &map);

    let wav = state
        .supertonic
        .synthesize(&body.text, &supertonic_lang, body.voice.as_deref(), body.speed)
        .await?;

    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, "audio/wav")],
        wav,
    )
        .into_response())
}
