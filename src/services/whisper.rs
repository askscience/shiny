//! faster-whisper streaming STT: sidecar client + model inventory/downloads.
//!
//! faster-whisper is CTranslate2 — native code that cannot run in the browser
//! the way Vosk's WASM build can. Core therefore proxies microphone audio to a
//! small local Python process (`voice/whisper_server.py`), which keeps one
//! Whisper model in memory and decodes the utterance incrementally.
//!
//! Audio is forwarded as raw 16 kHz mono PCM16LE; the sidecar owns all session
//! state, keyed by an opaque id the browser generates.
//!
//! The bundled model is **tiny** — it ships with the app so the default engine
//! works with no setup. **small** is opt-in and downloaded from Settings →
//! Voice; that download is long (hundreds of MB), so it runs in the background
//! and the page polls this manager for progress.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use serde_json::{json, Value};

/// One decode pass on a long utterance can take a moment on a busy machine.
const CHUNK_TIMEOUT_SECS: u64 = 60;
/// The health probe must never stall a status request when the sidecar is down.
const HEALTH_TIMEOUT_SECS: u64 = 3;

/// (key, model directory, label, approximate on-disk size).
pub const WHISPER_MODELS: &[(&str, &str, &str, &str)] = &[
    ("tiny", "faster-whisper-tiny", "Tiny", "~75 MB"),
    ("small", "faster-whisper-small", "Small", "~480 MB"),
];

/// Expected `model.bin` byte counts, used only to draw a progress bar while a
/// download is in flight (the byte size is stable for a released revision).
const MODEL_BYTES: &[(&str, u64)] = &[("tiny", 75_538_270), ("small", 483_546_902)];

pub fn model_dir_name(key: &str) -> Option<&'static str> {
    WHISPER_MODELS.iter().find(|(k, ..)| *k == key).map(|(_, d, ..)| *d)
}

pub fn expected_bytes(key: &str) -> u64 {
    MODEL_BYTES.iter().find(|(k, _)| *k == key).map(|(_, b)| *b).unwrap_or(0)
}

#[derive(Clone, Debug)]
struct DownloadState {
    status: String,
    error: Option<String>,
}

#[derive(Clone)]
pub struct WhisperClient {
    base_url: String,
    client: reqwest::Client,
    models_dir: PathBuf,
    downloads: Arc<Mutex<HashMap<String, DownloadState>>>,
}

impl WhisperClient {
    pub fn new(base_url: String, models_dir: String) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client: reqwest::Client::new(),
            models_dir: PathBuf::from(models_dir),
            downloads: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn models_dir(&self) -> &Path {
        &self.models_dir
    }

    /// `GET /health`, or `None` when the sidecar is not answering.
    pub async fn health(&self) -> Option<Value> {
        let res = self
            .client
            .get(format!("{}/health", self.base_url))
            .timeout(std::time::Duration::from_secs(HEALTH_TIMEOUT_SECS))
            .send()
            .await
            .ok()?;
        if !res.status().is_success() {
            return None;
        }
        res.json::<Value>().await.ok()
    }

    pub async fn is_available(&self) -> bool {
        self.health().await.is_some()
    }

    // ── Model inventory ──────────────────────────────────────────────────────

    pub fn model_present(&self, key: &str) -> bool {
        let Some(dir) = model_dir_name(key) else { return false };
        let path = self.models_dir.join(dir);
        path.join("model.bin").is_file() && path.join("config.json").is_file()
    }

    /// On-disk model state, annotated with what the sidecar has loaded when it
    /// is running. `health` is passed in so a status request only probes once.
    pub fn inventory(&self, health: Option<&Value>) -> Value {
        let mut out = serde_json::Map::new();
        for (key, dir, label, size) in WHISPER_MODELS {
            let loaded = health
                .and_then(|h| h.get("models"))
                .and_then(|m| m.get(*key))
                .and_then(|m| m.get("loaded"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            out.insert(
                (*key).to_string(),
                json!({
                    "key": key,
                    "dir": dir,
                    "label": label,
                    "size": size,
                    "present": self.model_present(key),
                    "loaded": loaded,
                }),
            );
        }
        Value::Object(out)
    }

    // ── Downloads (background) ───────────────────────────────────────────────

    /// Kick off `voice/download_whisper.py <model>` in the background. Returns
    /// immediately; poll [`Self::downloads`] for progress.
    pub fn start_download(&self, key: &str) -> Result<(), String> {
        if model_dir_name(key).is_none() {
            return Err(format!("Unknown whisper model: {key}"));
        }
        {
            let mut map = self.downloads.lock().unwrap();
            if map.get(key).map(|s| s.status == "downloading").unwrap_or(false) {
                return Ok(()); // already running — idempotent
            }
            map.insert(
                key.to_string(),
                DownloadState { status: "downloading".into(), error: None },
            );
        }

        let client = self.clone();
        let key = key.to_string();
        tokio::spawn(async move {
            let models_dir = client.models_dir.to_string_lossy().to_string();
            let model_arg = key.clone();
            let result = tokio::task::spawn_blocking(move || {
                Command::new("python3")
                    .arg("voice/download_whisper.py")
                    .arg(&model_arg)
                    .env("WHISPER_MODELS_DIR", &models_dir)
                    .output()
            })
            .await;

            let state = match result {
                Ok(Ok(output)) if output.status.success() => DownloadState {
                    status: "ready".into(),
                    error: None,
                },
                Ok(Ok(output)) => {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    let msg = stdout
                        .lines()
                        .last()
                        .filter(|l| l.contains("error"))
                        .unwrap_or(stderr.trim());
                    DownloadState {
                        status: "error".into(),
                        error: Some(if msg.is_empty() { "Download failed".into() } else { msg.to_string() }),
                    }
                }
                Ok(Err(e)) => DownloadState {
                    status: "error".into(),
                    error: Some(format!("Could not run the download script: {e}")),
                },
                Err(e) => DownloadState {
                    status: "error".into(),
                    error: Some(format!("Download task panicked: {e}")),
                },
            };
            tracing::info!(model = %key, status = %state.status, "faster-whisper model download finished");
            client.downloads.lock().unwrap().insert(key, state);
        });
        Ok(())
    }

    /// Per-model download state, including live byte progress from the
    /// partially written `model.bin.part` file.
    pub fn downloads(&self) -> Value {
        let map = self.downloads.lock().unwrap().clone();
        let mut out = serde_json::Map::new();
        for (key, _dir, _label, _size) in WHISPER_MODELS {
            let state = map.get(*key);
            let status = state.map(|s| s.status.clone()).unwrap_or_else(|| {
                if self.model_present(key) { "ready".into() } else { "absent".into() }
            });
            let total = expected_bytes(key);
            let part = self
                .models_dir
                .join(model_dir_name(key).unwrap_or_default())
                .join("model.bin.part");
            let bytes = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
            out.insert(
                (*key).to_string(),
                json!({
                    "status": status,
                    "error": state.and_then(|s| s.error.clone()),
                    "present": self.model_present(key),
                    "bytes": if status == "downloading" { bytes } else { 0 },
                    "total": total,
                }),
            );
        }
        Value::Object(out)
    }

    // ── Streaming ────────────────────────────────────────────────────────────

    /// Forward one audio chunk (or the final flush) and return the sidecar's
    /// JSON result: `{ text, partial, final }`.
    pub async fn chunk(
        &self,
        session: &str,
        lang: Option<&str>,
        model: Option<&str>,
        prompt: Option<&str>,
        is_final: bool,
        audio: Bytes,
    ) -> Result<Value, String> {
        let req = self
            .client
            .post(format!("{}/stt/chunk", self.base_url))
            .query(&[
                ("session", session),
                ("lang", lang.unwrap_or("")),
                ("model", model.unwrap_or("")),
                ("prompt", prompt.unwrap_or("")),
                ("final", if is_final { "true" } else { "false" }),
            ])
            .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
            .body(audio)
            .timeout(std::time::Duration::from_secs(CHUNK_TIMEOUT_SECS));

        let res = req.send().await.map_err(|e| {
            if e.is_connect() {
                "Faster-whisper is not running. Start it from Settings → Voice, or run ./voice/start_whisper.sh.".to_string()
            } else if e.is_timeout() {
                "Faster-whisper timed out on this chunk.".to_string()
            } else {
                format!("Faster-whisper request failed: {e}")
            }
        })?;

        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        if !status.is_success() {
            let detail = serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|v| v.get("detail").and_then(|d| d.as_str().map(String::from)))
                .unwrap_or(body);
            return Err(format!("Faster-whisper error ({status}): {detail}"));
        }

        serde_json::from_str::<Value>(&body)
            .map_err(|e| format!("Faster-whisper returned invalid JSON: {e}"))
    }

    /// Drop a session without producing a final transcript (user cancelled).
    pub async fn close(&self, session: &str) -> Result<Value, String> {
        let res = self
            .client
            .post(format!("{}/stt/close", self.base_url))
            .query(&[("session", session)])
            .timeout(std::time::Duration::from_secs(HEALTH_TIMEOUT_SECS))
            .send()
            .await
            .map_err(|e| format!("Faster-whisper request failed: {e}"))?;
        res.json::<Value>()
            .await
            .map_err(|e| format!("Faster-whisper returned invalid JSON: {e}"))
    }
}
