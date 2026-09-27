//! Qwen3-TTS: a second, higher-quality local TTS engine.
//!
//! Supertonic is the default (small, fast, always available). Qwen3-TTS is the
//! opt-in alternative: it runs `qwentts.cpp` (a C++17/GGML port of Qwen3-TTS)
//! which, unlike Supertonic's ONNX Runtime, has a real **Vulkan** backend — so
//! an AMD/Intel GPU is usable, not just CUDA or CPU. `voice/detect_accel.py`
//! picks the device (`Vulkan0` → `CUDA0` → `CPU`).
//!
//! ## Why this client is shaped the way it is
//!
//! Qwen3-TTS has a well-documented, model-intrinsic bug: it intermittently
//! never emits its codec EOS token, so generation runs to the frame cap and
//! produces minutes of audio for one sentence. It is not backend-specific —
//! QwenLM/Qwen3-TTS#118, sglang-omni#1179 and vllm-omni#6158 all document it,
//! and the fix has to be *detection*, because during the runaway the EOS
//! probability collapses to ~1e-14 (rank ~1500+), far outside `top_k`. No
//! sampling setting can recover it:
//!
//! * a per-request `max_new_tokens` budget bounds the damage, and
//! * long text is split into sentence-sized chunks so one utterance cannot
//!   ramble for minutes.
//!
//! Both are applied here. The sidecar is asked for **streaming PCM**, not a
//! buffered WAV: the buffered path builds one giant vocoder graph for the whole
//! utterance and hangs the GPU (observed as `amdgpu ring timeout` /
//! `i915 GPU HANG`); the streaming decoder emits small chunks and is stable.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

/// A whole utterance is decoded in one call; allow for a slow CPU fallback.
const SYNTH_TIMEOUT_SECS: u64 = 600;
/// The health probe must never stall a status request when the sidecar is down.
const HEALTH_TIMEOUT_SECS: u64 = 3;

/// Upper bound on how many codec frames one character of text may produce.
///
/// Normal speech lands near 1 frame/char (12.5 frames/s against ~13 chars/s
/// at 150 wpm); a slower, more expressive read measures ~3.8. The budget only
/// has to be *generous enough not to truncate a legitimate read* while still
/// bounding an EOS miss — so it sits above the observed clean rate, not at a
/// runaway's. At 5 a derailed sentence costs a few seconds, not tens.
const FRAMES_PER_CHAR: usize = 5;
/// Floor for very short text, so a one-word reply still gets room to finish.
const MIN_FRAMES: usize = 50;

/// (key, talker GGUF filename, label, approximate on-disk size, bundled).
///
/// `0.6b-customvoice` ships with the app; the 1.7B is an opt-in download, the
/// same way faster-whisper bundles `tiny` and offers `small`.
pub const QWEN_MODELS: &[(&str, &str, &str, &str, bool)] = &[
    (
        "0.6b-customvoice",
        "qwen-talker-0.6b-customvoice-Q4_K_M.gguf",
        "0.6B CustomVoice",
        "~605 MB",
        true,
    ),
    (
        "1.7b-customvoice",
        "qwen-talker-1.7b-customvoice-Q4_K_M.gguf",
        "1.7B CustomVoice",
        "~1.2 GB",
        false,
    ),
];

/// The codec (RVQ tokenizer) is shared by every talker and stays at Q4_K_M.
pub const QWEN_CODEC_FILE: &str = "qwen-tokenizer-12hz-Q4_K_M.gguf";
/// Same codec on Hugging Face, for the download script to fetch.
pub const QWEN_REPO: &str = "Serveurperso/Qwen3-TTS-GGUF";

/// Expected talker byte counts, for the download progress bar.
const MODEL_BYTES: &[(&str, u64)] = &[
    ("0.6b-customvoice", 604_878_080),
    ("1.7b-customvoice", 1_181_000_000),
];

/// The nine built-in speakers of the CustomVoice checkpoints.
pub const QWEN_SPEAKERS: &[&str] = &[
    "serena", "vivian", "uncle_fu", "ryan", "aiden", "ono_anna", "sohee", "eric", "dylan",
];

pub fn model_file(key: &str) -> Option<&'static str> {
    QWEN_MODELS.iter().find(|(k, ..)| *k == key).map(|(_, f, ..)| *f)
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
pub struct QwenClient {
    base_url: String,
    client: reqwest::Client,
    models_dir: PathBuf,
    downloads: Arc<Mutex<HashMap<String, DownloadState>>>,
}

impl QwenClient {
    pub fn new(base_url: String, models_dir: PathBuf) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(SYNTH_TIMEOUT_SECS))
            .build()
            .unwrap_or_default();
        Self {
            base_url,
            client,
            models_dir,
            downloads: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Ask the sidecar who it is. `None` when it is not answering.
    pub async fn health(&self) -> Option<Value> {
        let resp = self
            .client
            .get(format!("{}/v1/audio/voices", self.base_url))
            .timeout(std::time::Duration::from_secs(HEALTH_TIMEOUT_SECS))
            .send()
            .await
            .ok()?;
        if !resp.status().is_success() {
            return None;
        }
        resp.json::<Value>().await.ok()
    }

    pub fn model_present(&self, key: &str) -> bool {
        let Some(file) = model_file(key) else { return false };
        self.models_dir.join(file).is_file() && self.models_dir.join(QWEN_CODEC_FILE).is_file()
    }

    /// On-disk model state, annotated with whether the sidecar is up and which
    /// speaker list it reports.
    pub fn inventory(&self, health: Option<&Value>) -> Value {
        let speakers: Vec<String> = health
            .and_then(|h| h.get("voices"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.get("name").and_then(|n| n.as_str()).map(String::from))
                    .collect()
            })
            .unwrap_or_else(|| QWEN_SPEAKERS.iter().map(|s| (*s).to_string()).collect());

        let mut out = serde_json::Map::new();
        for (key, file, label, size, bundled) in QWEN_MODELS {
            out.insert(
                (*key).to_string(),
                json!({
                    "key": key,
                    "file": file,
                    "label": label,
                    "size": size,
                    "bundled": bundled,
                    "present": self.model_present(key),
                }),
            );
        }
        json!({ "models": Value::Object(out), "speakers": speakers })
    }

    // ── Downloads (background) ───────────────────────────────────────────────

    /// Kick off `voice/download_qwen_tts.py <model>` in the background.
    pub fn start_download(&self, key: &str) -> Result<(), String> {
        if model_file(key).is_none() {
            return Err(format!("Unknown Qwen3-TTS model: {key}"));
        }
        {
            let mut map = self.downloads.lock().unwrap();
            if map.get(key).map(|s| s.status == "downloading").unwrap_or(false) {
                return Ok(());
            }
            map.insert(key.to_string(), DownloadState { status: "downloading".into(), error: None });
        }

        let client = self.clone();
        let key = key.to_string();
        tokio::spawn(async move {
            let models_dir = client.models_dir.to_string_lossy().to_string();
            let model_arg = key.clone();
            let result = tokio::task::spawn_blocking(move || {
                Command::new("python3")
                    .arg("voice/download_qwen_tts.py")
                    .arg(&model_arg)
                    .env("QWEN_TTS_MODELS_DIR", &models_dir)
                    .output()
            })
            .await;

            let state = match result {
                Ok(Ok(output)) if output.status.success() => {
                    DownloadState { status: "ready".into(), error: None }
                }
                Ok(Ok(output)) => DownloadState {
                    status: "error".into(),
                    error: Some(last_error_line(&output.stdout, &output.stderr)),
                },
                Ok(Err(e)) => DownloadState {
                    status: "error".into(),
                    error: Some(format!("Could not run the download script: {e}")),
                },
                Err(e) => DownloadState {
                    status: "error".into(),
                    error: Some(format!("Download task panicked: {e}")),
                },
            };
            tracing::info!(model = %key, status = %state.status, "Qwen3-TTS model download finished");
            client.downloads.lock().unwrap().insert(key, state);
        });
        Ok(())
    }

    /// Per-model download state, with live byte progress from a partial file.
    pub fn downloads(&self) -> Value {
        let map = self.downloads.lock().unwrap().clone();
        let mut out = serde_json::Map::new();
        for (key, _file, _label, _size, _bundled) in QWEN_MODELS {
            let state = map.get(*key);
            let status = state.map(|s| s.status.clone()).unwrap_or_else(|| {
                if self.model_present(key) { "ready".into() } else { "absent".into() }
            });
            let part = self.models_dir.join(format!("{}.part", model_file(key).unwrap_or_default()));
            let bytes = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
            out.insert(
                (*key).to_string(),
                json!({
                    "status": status,
                    "error": state.and_then(|s| s.error.clone()),
                    "bytes": bytes,
                    "total": expected_bytes(key),
                    "present": self.model_present(key),
                }),
            );
        }
        Value::Object(out)
    }

    // ── Synthesis ────────────────────────────────────────────────────────────

    /// Synthesize `text` and return 24 kHz mono PCM (s16le), ready for the
    /// caller to wrap in a WAV header.
    ///
    /// Text is split into sentence-sized chunks and each is requested with a
    /// `max_new_tokens` budget, so an EOS miss can only ever overrun by a few
    /// seconds. Everything is concatenated into one utterance so the reply
    /// still sounds continuous.
    pub async fn synthesize(
        &self,
        text: &str,
        lang: &str,
        voice: Option<&str>,
        speed: Option<f32>,
    ) -> Result<Vec<u8>, String> {
        let model = self
            .active_model()
            .ok_or_else(|| "Qwen3-TTS model is not downloaded".to_string())?;
        let speaker = voice.filter(|v| !v.is_empty()).unwrap_or("vivian");

        let chunks = split_sentences(text);
        if chunks.is_empty() {
            return Err("Nothing to say".into());
        }

        let mut pcm: Vec<u8> = Vec::new();
        for chunk in chunks {
            let budget = frames_budget(&chunk);
            let body = json!({
                "input": chunk,
                "voice": speaker,
                "language": language_label(lang),
                "response_format": "pcm",
                "max_new_tokens": budget,
                // The engine's own default is 1.05; a mild bump discourages the
                // repetitive loops that accompany an EOS miss.
                "repetition_penalty": 1.1,
            });

            let resp = self
                .client
                .post(format!("{}/v1/audio/speech", self.base_url))
                .json(&body)
                .send()
                .await
                .map_err(|e| format!("Qwen3-TTS request failed: {e}"))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let detail = resp.text().await.unwrap_or_default();
                return Err(format!("Qwen3-TTS error ({status}): {detail}"));
            }

            let bytes = resp
                .bytes()
                .await
                .map_err(|e| format!("Qwen3-TTS returned invalid audio: {e}"))?;
            // A sentence that missed its EOS arrives with a long quiet tail;
            // drop it so a derail costs a sentence, not a minute of hum.
            pcm.extend_from_slice(trim_runaway_tail(&bytes, 24_000));
        }

        if pcm.is_empty() {
            return Err("Qwen3-TTS produced no audio".into());
        }
        let _ = (model, speed); // speed is not supported by this engine
        Ok(pcm)
    }

    /// The first model on disk, preferring the bundled 0.6B.
    fn active_model(&self) -> Option<&'static str> {
        QWEN_MODELS
            .iter()
            .find(|(k, ..)| self.model_present(k))
            .map(|(k, ..)| *k)
    }
}

/// Trim the runaway tail off a decoded utterance.
///
/// Qwen3-TTS's EOS miss is documented to render the sentence correctly and
/// then sit in a stationary state (near-silence, or a low-level babble) until
/// the frame cap. Both sglang-omni#1179 and voicebox#1077 identify that
/// sustained low-energy tail as the reliable signature — so if the audio ends
/// in a long quiet stretch, the quiet stretch is discarded.
///
/// Only a *trailing* run of quiet is removed, and only when it is substantial:
/// a pause inside normal speech is short, and cutting it would clip words.
pub fn trim_runaway_tail(pcm: &[u8], sample_rate: u32) -> &[u8] {
    const QUIET_RMS: f32 = 0.006; // ≈ −44 dBFS
    const WINDOW_MS: u32 = 250;
    /// Quiet must last this long before it counts as a runaway tail.
    const MIN_QUIET_MS: u32 = 3_000;
    /// Never cut more than the tail; keep a little lead-out pad.
    const KEEP_MS: u32 = 250;

    let samples = pcm.len() / 2;
    let window = (sample_rate / 1000 * WINDOW_MS) as usize;
    if window == 0 || samples < window * 2 {
        return pcm;
    }

    // Walk backwards in windows while they are quiet.
    let mut quiet_windows = 0usize;
    let mut cut_sample = samples;
    let mut end = samples;
    while end >= window {
        let start = end - window;
        let mut sum = 0.0f64;
        for i in start..end {
            let v = i16::from_le_bytes([pcm[i * 2], pcm[i * 2 + 1]]) as f32 / 32768.0;
            sum += (v * v) as f64;
        }
        let rms = (sum / window as f64).sqrt() as f32;
        if rms > QUIET_RMS {
            break;
        }
        quiet_windows += 1;
        cut_sample = start;
        end = start;
    }

    let quiet_ms = quiet_windows as u32 * WINDOW_MS;
    if quiet_ms < MIN_QUIET_MS || cut_sample == samples {
        return pcm;
    }
    let keep = (sample_rate / 1000 * KEEP_MS) as usize;
    let keep_sample = (cut_sample + keep).min(samples);
    &pcm[..keep_sample * 2]
}

fn last_error_line(stdout: &[u8], stderr: &[u8]) -> String {
    let out = String::from_utf8_lossy(stdout);
    let err = String::from_utf8_lossy(stderr);
    out.lines()
        .last()
        .filter(|l| l.to_lowercase().contains("error"))
        .or_else(|| err.lines().last())
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| "Download failed".into())
}

/// Split `text` on sentence boundaries, merging fragments up to a sane size.
///
/// Each chunk becomes its own generation, which is the documented mitigation
/// for Qwen3-TTS's missing-EOS runaway: a short utterance has far less room to
/// derail, and a failure is contained to one sentence.
pub fn split_sentences(text: &str) -> Vec<String> {
    const MAX_CHARS: usize = 300;

    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut chars = text.trim().chars().peekable();

    while let Some(c) = chars.next() {
        current.push(c);
        let boundary = matches!(c, '.' | '!' | '?' | ';' | '\n');
        // A terminator counts only when followed by space/end, so "3.14" and
        // "example.com" stay intact.
        let followed_by_space = chars
            .peek()
            .map(|n| n.is_whitespace() || *n == '\n')
            .unwrap_or(true);

        if (boundary && followed_by_space) || current.chars().count() >= MAX_CHARS {
            let trimmed = current.trim();
            if !trimmed.is_empty() {
                chunks.push(trimmed.to_string());
            }
            current.clear();
        }
    }
    let tail = current.trim();
    if !tail.is_empty() {
        chunks.push(tail.to_string());
    }
    chunks
}

/// Per-request frame budget, bounded by what the text can plausibly fill.
pub fn frames_budget(text: &str) -> usize {
    (text.chars().count() * FRAMES_PER_CHAR).max(MIN_FRAMES)
}

/// Map an ISO-639-1 code to the label the sidecar expects.
pub fn language_label(lang: &str) -> String {
    match lang.trim().to_lowercase().as_str() {
        "en" => "English",
        "zh" | "cmn" => "Chinese",
        "ja" => "Japanese",
        "ko" => "Korean",
        "de" => "German",
        "fr" => "French",
        "ru" => "Russian",
        "pt" => "Portuguese",
        "es" => "Spanish",
        "it" => "Italian",
        other => return other.to_string(),
    }
    .to_string()
}

/// Wrap raw 24 kHz mono s16le PCM in a WAV (RIFF) container.
///
/// The sidecar streams bare PCM, but `/api/tts` hands the browser something
/// `new Audio(...)` can play, so the container is added here.
pub fn pcm_to_wav(pcm: &[u8], sample_rate: u32) -> Vec<u8> {
    let channels: u16 = 1;
    let bits: u16 = 16;
    let block_align = channels * bits / 8;
    let byte_rate = sample_rate * block_align as u32;
    let data_len = pcm.len() as u32;
    let riff_len = 36 + data_len;

    let mut out = Vec::with_capacity(44 + pcm.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&riff_len.to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // PCM chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM format
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&bits.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    out.extend_from_slice(pcm);
    out
}

/// Convenience for the API layer: where the models live.
pub fn default_models_dir() -> PathBuf {
    PathBuf::from("data/qwen-tts-models")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentences_split_on_terminators() {
        let out = split_sentences("Hello there. How are you? Fine!");
        assert_eq!(out, vec!["Hello there.", "How are you?", "Fine!"]);
    }

    #[test]
    fn decimals_are_not_boundaries() {
        let out = split_sentences("Pi is 3.14 and so on.");
        assert_eq!(out, vec!["Pi is 3.14 and so on."]);
    }

    #[test]
    fn long_text_is_capped() {
        let text = "word ".repeat(200);
        let out = split_sentences(&text);
        assert!(out.len() > 1, "long text should split");
        assert!(out.iter().all(|c| c.chars().count() <= 300));
    }

    #[test]
    fn frame_budget_has_a_floor_and_scales() {
        assert_eq!(frames_budget("hi"), MIN_FRAMES);
        assert!(frames_budget(&"a".repeat(100)) > MIN_FRAMES);
        // A 42-char sentence (the size that ran away for 42 s at 12/char) is
        // now bounded to roughly 17 s — painful but bounded, and most sentences
        // finish in a fraction of it.
        assert_eq!(frames_budget(&"a".repeat(42)), 210);
    }

    #[test]
    fn wav_header_is_well_formed() {
        let pcm = vec![0u8; 200];
        let wav = pcm_to_wav(&pcm, 24000);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[36..40], b"data");
        assert_eq!(wav.len(), 44 + 200);
        assert_eq!(u32::from_le_bytes([wav[24], wav[25], wav[26], wav[27]]), 24000);
    }

    #[test]
    fn language_labels_match_the_sidecar() {
        assert_eq!(language_label("en"), "English");
        assert_eq!(language_label("ZH"), "Chinese");
        assert_eq!(language_label("xx"), "xx");
    }

    /// Build `secs` of quiet or loud 24 kHz mono s16le.
    fn pcm(secs: f32, amp: f32) -> Vec<u8> {
        let n = (24_000.0 * secs) as usize;
        let mut out = Vec::with_capacity(n * 2);
        for i in 0..n {
            // A tone for "loud" so RMS is well above the quiet threshold.
            let v = if amp > 0.01 {
                (i as f32 * 0.05).sin() * amp
            } else {
                0.0
            };
            out.extend_from_slice(&((v * 32767.0) as i16).to_le_bytes());
        }
        out
    }

    #[test]
    fn runaway_tail_is_trimmed() {
        let mut audio = pcm(2.0, 0.4); // speech
        audio.extend(pcm(20.0, 0.0)); // the stationary tail
        let trimmed = trim_runaway_tail(&audio, 24_000);
        assert!(trimmed.len() < audio.len(), "long quiet tail should be cut");
        // Kept the speech and only a short pad.
        assert!(trimmed.len() >= 2 * 24_000 * 2);
        assert!(trimmed.len() <= (2.0 + 1.0) as usize * 24_000 * 2);
    }

    #[test]
    fn short_natural_pause_is_kept() {
        let mut audio = pcm(1.0, 0.4);
        audio.extend(pcm(1.0, 0.0)); // a normal inter-sentence pause
        audio.extend(pcm(1.0, 0.4));
        assert_eq!(trim_runaway_tail(&audio, 24_000).len(), audio.len());
    }

    #[test]
    fn fully_loud_audio_is_untouched() {
        let audio = pcm(5.0, 0.4);
        assert_eq!(trim_runaway_tail(&audio, 24_000).len(), audio.len());
    }
}
