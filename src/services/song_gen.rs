//! Local song generation through HOT-Step-CPP's YuE2 engine (`ace-server`).
//!
//! The assistant's `generate_song` tool drives this service: one `ace-server`
//! process per song, started on a private loopback port and stopped when the
//! render ends, so nothing stays resident between jobs. The engine renders
//! 48 kHz stereo WAV; the file is written into the caller's Files home under
//! `Music/Generated/` and served to the chat by `/api/songgen/tracks/:name`.
//!
//! Weights are fetched on demand by `voice/download_yue2.py` into
//! `YUE2_MODELS_DIR` (default `data/yue2-models`), laid out as
//! `<dir>/yue2/yue2-lm-*.gguf` + the shared VAE/companion/tokenizer.
//!
//! ## Baked-in operating point (measured on the 8 GB Navi14, see
//! `benchmarks/yue-v2/README.md`)
//!
//! - `nar_chunk_frames: 512` — a whole-song NAR chunk hard-hangs RADV on this
//!   class of card; four balanced chunks render in ~7 min for a 75 s song.
//! - The VAE GGUF is patched to `yue2vae.decode_core_frames = 128` by the
//!   downloader — the stock 512-frame tile builds a ~3.2 GB compute graph
//!   whose execution kills the device on 8 GB cards. Smaller tiles decode
//!   faster here (~9 s) and are harmless elsewhere.
//! - `ode_steps: 32` is the model's own schedule; `preview_max_frames` caps
//!   the song at the requested duration (~25 frames per second of audio).
//! - On AMD the child gets `GGML_VK_DISABLE_INTEGER_DOT_PRODUCT=1` — the same
//!   RADV workaround the TTS sidecar and sd-cli use.
//!
//! The engine evicts modules between stages (AR → NAR → VAE) and releases
//! everything when the process exits, so it coexists with the TTS sidecar the
//! same way `image_gen` does.

use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

/// One selectable language-model precision. `lm_file`/`lm_type` are the exact
/// on-disk names the engine discovers (`yue2-lm-<lm_type>.gguf`).
pub struct SongTier {
    pub key: &'static str,
    pub quant: &'static str,
    pub label: &'static str,
    pub lm_file: &'static str,
    pub lm_type: &'static str,
    pub bytes: u64,
}

/// Compact Q4_K_M-imat is the operating point we validated (imatrix-guided;
/// NAR rel-L2 0.116 vs bf16's 0.013). Q8_0 is the near-lossless opt-in.
pub const TIERS: &[SongTier] = &[
    SongTier {
        key: "q4",
        quant: "Q4_K_M-imat",
        label: "Compact",
        lm_file: "yue2-lm-Q4_K_M-imat.gguf",
        lm_type: "Q4_K_M-imat",
        bytes: 2_429_623_904,
    },
    SongTier {
        key: "q8",
        quant: "Q8_0",
        label: "Studio",
        lm_file: "yue2-lm-q8_0.gguf",
        lm_type: "q8_0",
        bytes: 3_916_164_544,
    },
];

/// The tier used unless the user picked another in Settings → Music.
pub const DEFAULT_TIER: &str = "q4";

/// Files every tier needs, relative to `<models_dir>/yue2/`.
pub const SHARED: &[(&str, &str, u64)] = &[
    ("yue2-vae-standard-f32.gguf", "Oobleck decoder (VAE)", 530_348_768),
    ("nar_lora_joint_v9.safetensors", "Decoder companion (v9)", 140_560_592),
];

/// BPE files the LM loads at startup, relative to `<models_dir>/yue2/`.
pub const TOKENIZER_FILES: &[(&str, &str, u64)] = &[
    ("tokenizer/vocab.json", "Tokenizer vocabulary", 3_080_121),
    ("tokenizer/merges.txt", "Tokenizer merges", 1_823_241),
    ("tokenizer/special_tokens.json", "Tokenizer special tokens", 14_141),
    ("tokenizer/pretokenizer.json", "Tokenizer pretokenizer", 900),
    ("tokenizer/expected-ids-corpus.json", "Tokenizer parity fixture", 24_681),
];

/// Hard ceiling for one song; a 75 s song measures ~8 min on a Navi14, and
/// `preview_max_frames` bounds longer requests, but slow machines need slack.
const SONG_TIMEOUT: Duration = Duration::from_secs(3600);
/// Frames per second of audio in the semantic stream (25 Hz).
const FRAMES_PER_SEC: u32 = 25;
/// A whole-song NAR chunk hangs RADV on 8 GB cards; 512-frame chunks are the
/// validated operating point (see `benchmarks/yue-v2/README.md`).
const NAR_CHUNK_FRAMES: u32 = 512;
/// The checkpoint's own midpoint-solver schedule.
const ODE_STEPS: u32 = 32;
/// How long a first-use `allow_download` call waits for the fetch to finish.
pub const DOWNLOAD_WAIT: Duration = Duration::from_secs(2400);

#[derive(Clone, Debug)]
struct DownloadState {
    status: String,
    error: Option<String>,
}

#[derive(Clone)]
pub struct SongGenClient {
    /// Root weights directory; the engine store lives in `<dir>/yue2/`.
    models_dir: PathBuf,
    engine: Option<PathBuf>,
    /// Keyed `"song:<tier|base>"`.
    downloads: Arc<Mutex<HashMap<String, DownloadState>>>,
    /// One render at a time — the engine owns a port and the GPU.
    job_lock: Arc<Mutex<()>>,
}

/// What the tool asks the service to render.
pub struct SongRequest {
    /// Free-form style/description (`style` in the engine request).
    pub style: String,
    /// Lyrics with `[Section]` tags; may be empty.
    pub lyrics: String,
    /// Display/file title (slugged into the file name).
    pub title: String,
    /// Upper bound on song length, in seconds.
    pub duration_secs: u32,
    pub seed: Option<u64>,
    /// Tier key (`"q4"` / `"q8"`).
    pub quality: String,
    /// The Files-sandbox home the track is written into.
    pub home: PathBuf,
}

/// Where the finished track ended up + what it cost.
pub struct SongResult {
    pub file: String,
    pub rel_path: String,
    pub url: String,
    pub quality: String,
    pub quant: String,
    pub duration_secs: f64,
    pub seed: Option<u64>,
    pub seconds: f64,
    pub bytes: u64,
}

/// Resolve a tier: the key (`q4`), the quant (`Q4_K_M-imat`, `q4km`), or a
/// loose spelling (`q8`, `q8_0`). Comparison strips case and punctuation.
pub fn resolve_tier(input: &str) -> Option<&'static SongTier> {
    fn norm(s: &str) -> String {
        s.chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .map(|c| c.to_ascii_lowercase())
            .collect()
    }
    let needle = norm(input);
    if needle.is_empty() {
        return None;
    }
    TIERS.iter().find(|t| {
        let quant = norm(t.quant);
        let lm = norm(t.lm_type);
        needle == norm(t.key)
            || needle == quant
            || needle == lm
            || needle == quant.strip_suffix("imat").unwrap_or(&quant)
            || needle == lm.strip_suffix("imat").unwrap_or(&lm)
    })
}

impl SongGenClient {
    pub fn new(models_dir: PathBuf, engine_hint: Option<String>) -> Self {
        Self {
            models_dir,
            engine: resolve_engine(engine_hint),
            downloads: Arc::new(Mutex::new(HashMap::new())),
            job_lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn models_dir(&self) -> &Path {
        &self.models_dir
    }

    pub fn engine(&self) -> Option<&Path> {
        self.engine.as_deref()
    }

    /// The engine store all files live under.
    fn store(&self) -> PathBuf {
        self.models_dir.join("yue2")
    }

    fn file_present(&self, rel: &str) -> bool {
        self.store().join(rel).is_file()
    }

    fn tier_present(&self, tier: &SongTier) -> bool {
        self.file_present(tier.lm_file)
    }

    /// Shared files + tokenizer + the LM a tier needs, as human labels.
    pub fn missing_for(&self, quality: &str) -> Vec<String> {
        let Some(tier) = resolve_tier(quality) else {
            return vec![format!("unknown quality '{quality}'")];
        };
        let mut missing = Vec::new();
        for (file, label, ..) in SHARED {
            if !self.file_present(file) {
                missing.push((*label).to_string());
            }
        }
        for (file, label, ..) in TOKENIZER_FILES {
            if !self.file_present(file) {
                missing.push((*label).to_string());
            }
        }
        if !self.tier_present(tier) {
            missing.push(format!("{} language model", tier.quant));
        }
        missing
    }

    /// Bytes still to fetch for a tier (0 when everything is on disk).
    pub fn missing_bytes_for(&self, quality: &str) -> u64 {
        let Some(tier) = resolve_tier(quality) else { return 0 };
        let mut bytes = 0;
        for (file, _, size) in SHARED.iter().chain(TOKENIZER_FILES.iter()) {
            if !self.file_present(file) {
                bytes += size;
            }
        }
        if !self.tier_present(tier) {
            bytes += tier.bytes;
        }
        bytes
    }

    /// On-disk state of the engine and every tier — drives Settings → Music.
    pub fn inventory(&self) -> Value {
        let mut qualities = serde_json::Map::new();
        for tier in TIERS {
            qualities.insert(
                tier.key.to_string(),
                json!({
                    "key": tier.key,
                    "quant": tier.quant,
                    "label": tier.label,
                    "file": tier.lm_file,
                    "bytes": tier.bytes,
                    "size": crate::services::media::human_bytes(tier.bytes),
                    "present": self.tier_present(tier),
                    "ready": self.missing_for(tier.key).is_empty(),
                }),
            );
        }
        let shared: Vec<Value> = SHARED
            .iter()
            .chain(TOKENIZER_FILES.iter())
            .map(|(file, label, bytes)| {
                json!({
                    "file": file,
                    "label": label,
                    "bytes": bytes,
                    "size": crate::services::media::human_bytes(*bytes),
                    "present": self.file_present(file),
                })
            })
            .collect();
        json!({
            "engine": self.engine.as_ref().map(|p| p.to_string_lossy().to_string()),
            "dir": self.store().to_string_lossy().to_string(),
            "default_quality": DEFAULT_TIER,
            "shared": shared,
            "qualities": Value::Object(qualities),
        })
    }

    // ── Downloads (background) ───────────────────────────────────────────────

    /// Kick off the downloader in the background (`<tier>` or `all`).
    pub fn start_download(&self, target: &str) -> Result<(), String> {
        let target = target.trim().to_lowercase();
        let keys: Vec<&str> = if target == "all" {
            TIERS.iter().map(|t| t.key).collect()
        } else if let Some(tier) = resolve_tier(&target) {
            vec![tier.key]
        } else {
            return Err(format!("Unknown song quality: {target}"));
        };

        let base_key = "song:base".to_string();
        {
            let mut map = self.downloads.lock().unwrap();
            if map.get(&base_key).map(|s| s.status == "downloading").unwrap_or(false) {
                return Ok(());
            }
            for key in keys.iter().chain(["base"].iter()) {
                map.insert(
                    format!("song:{key}"),
                    DownloadState { status: "downloading".into(), error: None },
                );
            }
        }

        let client = self.clone();
        // The script speaks canonical keys only ("q4" | "q8" | "all").
        let arg = if target == "all" { "all".to_string() } else { keys[0].to_string() };
        let tracked: Vec<String> = keys
            .iter()
            .map(|k| format!("song:{k}"))
            .chain([base_key])
            .collect();
        let models_dir = self.models_dir.to_string_lossy().to_string();
        tokio::spawn(async move {
            let script_arg = arg.clone();
            let dir = models_dir.clone();
            let result = tokio::task::spawn_blocking(move || {
                Command::new("python3")
                    .arg("voice/download_yue2.py")
                    .arg(&script_arg)
                    .env("YUE2_MODELS_DIR", &dir)
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
            tracing::info!(target = %arg, status = %state.status, "song model download finished");
            let mut map = client.downloads.lock().unwrap();
            for key in tracked {
                map.insert(key, state.clone());
            }
        });
        Ok(())
    }

    /// Per-tier download state, with live byte progress from the partial files.
    pub fn downloads(&self) -> Value {
        let map = self.downloads.lock().unwrap().clone();
        let store = self.store();
        let part_bytes = |file: &str| -> u64 {
            std::fs::metadata(store.join(format!("{file}.part")))
                .map(|m| m.len())
                .unwrap_or(0)
        };

        let mut out = serde_json::Map::new();
        let base_present = SHARED
            .iter()
            .chain(TOKENIZER_FILES.iter())
            .all(|(file, ..)| self.file_present(file));
        let base_state = map.get("song:base");
        let base_status = base_state.map(|s| s.status.clone()).unwrap_or_else(|| {
            if base_present { "ready".into() } else { "absent".into() }
        });
        let base_total: u64 = SHARED
            .iter()
            .chain(TOKENIZER_FILES.iter())
            .map(|(.., b)| b)
            .sum();
        let base_bytes: u64 = if base_present {
            0
        } else {
            SHARED
                .iter()
                .chain(TOKENIZER_FILES.iter())
                .map(|(f, ..)| part_bytes(f))
                .sum()
        };
        out.insert(
            "base".to_string(),
            json!({
                "status": base_status,
                "error": base_state.and_then(|s| s.error.clone()),
                "bytes": base_bytes,
                "total": base_total,
                "present": base_present,
            }),
        );

        for tier in TIERS {
            let state = map.get(&format!("song:{}", tier.key));
            let status = state.map(|s| s.status.clone()).unwrap_or_else(|| {
                if self.tier_present(tier) { "ready".into() } else { "absent".into() }
            });
            out.insert(
                tier.key.to_string(),
                json!({
                    "status": status,
                    "error": state.and_then(|s| s.error.clone()),
                    "bytes": part_bytes(tier.lm_file),
                    "total": tier.bytes,
                    "present": self.tier_present(tier),
                }),
            );
        }
        Value::Object(out)
    }

    /// Wait for a tier (and the shared files) to become complete. Returns
    /// false on timeout or a reported download error.
    pub async fn wait_for_ready(&self, quality: &str, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if self.missing_for(quality).is_empty() {
                return true;
            }
            let failed = {
                let map = self.downloads.lock().unwrap();
                map.iter().any(|(k, s)| k.starts_with("song:") && s.status == "error")
            };
            if failed || Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }

    // ── Generation (blocking; call from spawn_blocking) ──────────────────────

    pub fn generate(&self, req: &SongRequest) -> Result<SongResult, String> {
        let Some(tier) = resolve_tier(&req.quality) else {
            return Err(format!("unknown song quality: {}", req.quality));
        };
        let missing = self.missing_for(tier.key);
        if !missing.is_empty() {
            return Err(format!("models-missing: {}", missing.join(", ")));
        }
        let engine = self.engine.clone().ok_or_else(|| {
            "the YuE2 engine is not installed — set YUE2_ENGINE or run scripts/install-yue2-engine.sh"
                .to_string()
        })?;

        // One engine and one GPU at a time.
        let _job = self.job_lock.lock().map_err(|_| "song job lock poisoned".to_string())?;

        let dir_out = req.home.join("Music").join("Generated");
        std::fs::create_dir_all(&dir_out)
            .map_err(|e| format!("cannot create {}: {e}", dir_out.display()))?;
        let name = crate::services::media::stamped_name(&req.title, "song", "wav");
        let out = dir_out.join(&name);

        let log_dir = PathBuf::from("data/music-logs");
        std::fs::create_dir_all(&log_dir).ok();
        let log_path = log_dir.join(format!("{}.log", name.trim_end_matches(".wav")));
        let log = File::create(&log_path).map_err(|e| format!("cannot open render log: {e}"))?;
        let log_err = log.try_clone().map_err(|e| format!("cannot open render log: {e}"))?;

        // A free loopback port for this render's engine instance.
        let port = {
            let listener = TcpListener::bind("127.0.0.1:0")
                .map_err(|e| format!("cannot reserve a local port: {e}"))?;
            listener.local_addr().map_err(|e| format!("cannot reserve a local port: {e}"))?.port()
        };

        let mut cmd = Command::new(&engine);
        cmd.arg("--models")
            .arg(&self.models_dir)
            .arg("--host")
            .arg("127.0.0.1")
            .arg("--port")
            .arg(port.to_string());
        if std::env::var_os("GGML_VK_DISABLE_INTEGER_DOT_PRODUCT").is_none()
            && crate::services::media::amd_gpu_present()
        {
            cmd.env("GGML_VK_DISABLE_INTEGER_DOT_PRODUCT", "1");
        }
        cmd.stdin(Stdio::null()).stdout(log).stderr(log_err);

        let start = Instant::now();
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("could not start the song engine: {e}"))?;

        let result = self.drive_engine(&mut child, port, req, tier, &log_path, start);

        // The engine is per-job: ask it to release weights, then stop it.
        let base = format!("http://127.0.0.1:{port}");
        if let Ok(client) = engine_http_client() {
            let _ = client.post(format!("{base}/yue2/unload")).send();
        }
        let _ = child.kill();
        let _ = child.wait();

        let result = result?;
        let wav = result.0;
        std::fs::write(&out, &wav)
            .map_err(|e| format!("cannot write {}: {e}", out.display()))?;
        tracing::info!(
            quality = %tier.key,
            quant = %tier.quant,
            seconds = start.elapsed().as_secs_f64(),
            "song generated"
        );
        Ok(SongResult {
            file: name.clone(),
            rel_path: format!("Music/Generated/{name}"),
            url: format!("/api/songgen/tracks/{name}"),
            quality: tier.key.to_string(),
            quant: tier.quant.to_string(),
            duration_secs: result.1,
            seed: req.seed,
            seconds: start.elapsed().as_secs_f64(),
            bytes: wav.len() as u64,
        })
    }

    /// Wait for `/health`, submit the synth job, poll it to completion and
    /// fetch the WAV bytes. Returns `(wav_bytes, duration_seconds)`.
    fn drive_engine(
        &self,
        child: &mut Child,
        port: u16,
        req: &SongRequest,
        tier: &SongTier,
        log_path: &Path,
        start: Instant,
    ) -> Result<(Vec<u8>, f64), String> {
        let base = format!("http://127.0.0.1:{port}");
        let client = engine_http_client()?;

        // 1. Wait for the HTTP server to come up.
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(status) = child_exited(child)? {
                return Err(format!(
                    "the song engine exited during startup ({status}): {}",
                    last_log_error(log_path)
                ));
            }
            if client.get(format!("{base}/health")).send().is_ok() {
                break;
            }
            if Instant::now() >= deadline {
                return Err("the song engine did not answer /health in 20 s".into());
            }
            std::thread::sleep(Duration::from_millis(250));
        }

        // 2. Submit the render. `preview_max_frames` bounds the length;
        //    `lm_type` pins the exact LM file this tier selected.
        let frames = req.duration_secs.clamp(30, 300) * FRAMES_PER_SEC;
        let mut body = json!({
            "style": &req.style,
            "lyrics": &req.lyrics,
            "cot": "full",
            "nar_chunk_frames": NAR_CHUNK_FRAMES,
            "ode_steps": ODE_STEPS,
            "preview_max_frames": frames,
            "lm_type": tier.lm_type,
            "vae_variant": "standard",
        });
        if let Some(seed) = req.seed {
            body["seed"] = json!(seed);
        }
        let resp = client
            .post(format!("{base}/yue2/synth"))
            .header("content-type", "application/json")
            .body(body.to_string())
            .send()
            .map_err(|e| format!("could not submit the song request: {e}"))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        if !status.is_success() {
            return Err(format!("the engine rejected the song request ({status}): {text}"));
        }
        let job_id = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| v.get("id").and_then(|i| i.as_str()).map(str::to_string))
            .ok_or_else(|| format!("the engine returned no job id: {text}"))?;
        tracing::info!(job = %job_id, "song render started");

        // 3. Poll until done.
        loop {
            if let Some(status) = child_exited(child)? {
                return Err(format!(
                    "the song engine stopped mid-render ({status}): {}",
                    last_log_error(log_path)
                ));
            }
            if start.elapsed() > SONG_TIMEOUT {
                return Err("song generation timed out".into());
            }
            let poll = client
                .get(format!("{base}/job?id={job_id}"))
                .send()
                .and_then(|r| r.text());
            if let Ok(text) = poll {
                if let Ok(v) = serde_json::from_str::<Value>(&text) {
                    match v.get("status").and_then(|s| s.as_str()) {
                        Some("done") => break,
                        Some("failed") | Some("cancelled") => {
                            return Err(format!(
                                "song generation failed: {}",
                                last_log_error(log_path)
                            ));
                        }
                        _ => {}
                    }
                }
            }
            std::thread::sleep(Duration::from_secs(1));
        }

        // 4. Fetch the finished audio (raw `audio/wav` body).
        let mut resp = client
            .get(format!("{base}/job?id={job_id}&result=1"))
            .send()
            .map_err(|e| format!("could not fetch the finished song: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("the engine could not return the song ({})", resp.status()));
        }
        let mut bytes = Vec::new();
        resp.read_to_end(&mut bytes)
            .map_err(|e| format!("could not read the song audio: {e}"))?;
        if bytes.len() < 44 {
            return Err("the engine returned no audio".into());
        }
        let seconds = wav_seconds(&bytes).unwrap_or(0.0);
        Ok((bytes, seconds))
    }
}

fn engine_http_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| format!("could not create the engine HTTP client: {e}"))
}

fn child_exited(child: &mut Child) -> Result<Option<std::process::ExitStatus>, String> {
    child
        .try_wait()
        .map_err(|e| format!("could not track the song engine: {e}"))
}

/// Duration of a PCM WAV, from its own header (0.0 when unreadable).
fn wav_seconds(bytes: &[u8]) -> Option<f64> {
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let channels = u16::from_le_bytes([bytes[22], bytes[23]]) as f64;
    let rate = u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]) as f64;
    let bits = u16::from_le_bytes([bytes[34], bytes[35]]) as f64;
    if channels <= 0.0 || rate <= 0.0 || bits <= 0.0 {
        return None;
    }
    let mut i = 12;
    while i + 8 <= bytes.len() {
        let id = &bytes[i..i + 4];
        let size = u32::from_le_bytes([bytes[i + 4], bytes[i + 5], bytes[i + 6], bytes[i + 7]]) as usize;
        if id == b"data" {
            return Some(size as f64 / (rate * channels * (bits / 8.0)));
        }
        i += 8 + size + (size & 1);
    }
    None
}

/// The engine binary: an explicit hint, the distribution build, then the
/// benchmark checkout this machine has today.
fn resolve_engine(hint: Option<String>) -> Option<PathBuf> {
    if let Some(hint) = hint.filter(|h| !h.trim().is_empty()) {
        return Some(PathBuf::from(hint));
    }
    [
        "build/hot-step/engine/ace-server",
        "benchmarks/yue-v2/build/hot-step/engine/ace-server",
    ]
    .iter()
    .map(PathBuf::from)
    .find(|p| p.is_file())
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

/// Last meaningful lines of an engine log, for a failed render.
fn last_log_error(path: &Path) -> String {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let tail: Vec<&str> = text
        .lines()
        .rev()
        .filter(|l| !l.trim().is_empty())
        .take(3)
        .collect();
    if tail.is_empty() {
        return "no output captured".into();
    }
    tail.into_iter().rev().collect::<Vec<_>>().join(" | ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiers_resolve_by_key_and_quant() {
        assert_eq!(resolve_tier("q4").map(|t| t.key), Some("q4"));
        assert_eq!(resolve_tier("Q4_K_M-imat").map(|t| t.key), Some("q4"));
        assert_eq!(resolve_tier("q4km").map(|t| t.key), Some("q4"));
        assert_eq!(resolve_tier("Q8_0").map(|t| t.key), Some("q8"));
        assert_eq!(resolve_tier("q8_0").map(|t| t.key), Some("q8"));
        assert!(resolve_tier("q2").is_none());
    }

    #[test]
    fn wav_duration_reads_the_header() {
        // 48 kHz stereo 16-bit, 1 s of silence.
        let data = vec![0u8; 48_000 * 2 * 2];
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36u32 + data.len() as u32).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes()); // channels
        wav.extend_from_slice(&48_000u32.to_le_bytes()); // rate
        wav.extend_from_slice(&192_000u32.to_le_bytes());
        wav.extend_from_slice(&4u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes()); // bits
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
        wav.extend_from_slice(&data);
        assert_eq!(wav_seconds(&wav), Some(1.0));
        assert_eq!(wav_seconds(b"nope"), None);
    }
}
