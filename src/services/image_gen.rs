//! Z-Image Turbo: local text-to-image through stable-diffusion.cpp (`sd-cli`).
//!
//! The assistant's `generate_image` tool drives this service: one `sd-cli`
//! process per image, so every byte of model state — the 2.5 GB Qwen3 text
//! encoder included — is released the moment the job ends. Weights are fetched
//! on demand by `voice/download_zimage.py`; the shipped default is the `fast`
//! tier (Q3_K, ~3.1 GB) plus the shared encoder and VAE (~2.8 GB).
//!
//! ## Why these flags
//!
//! `--backend te=cpu` keeps the Qwen3 text encoder out of VRAM entirely: it
//! runs once per prompt before sampling, so the CPU placement costs ~3 s and
//! leaves the whole device budget to the diffusion graph. `--offload-to-cpu`
//! keeps weights in RAM and stages them per graph segment, and `--diffusion-fa`
//! (flash attention) shrinks the sampling workspace. `--max-vram 2` is the
//! important one on a card that is shared with the TTS sidecar (~1.7 GiB): with
//! sd.cpp's default `--max-vram 0` ("use live free VRAM, no budget") the
//! graph-cut planner flips to a monolithic plan whenever plenty of VRAM looks
//! free, overcommits, and then fails its own workspace capacity check. An
//! explicit budget keeps the segmented plan that is known to fit.
//! `--vae-conv-direct` (plus `--vae-tiling` at 1024²) caps the decode peak.
//!
//! On RADV/Navi14 `GGML_VK_DISABLE_INTEGER_DOT_PRODUCT=1` is exported — the
//! same Q4_K dequant-shader workaround the TTS sidecar and bench scripts use.

use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

/// Quality tiers advertised in Settings, cheapest first.
/// `(key, diffusion quant, display label, file bytes)`.
pub const QUALITIES: &[(&str, &str, &str, u64)] = &[
    ("fast", "Q3_K", "Fast", 3_143_559_104),
    ("balanced", "Q4_K", "Balanced", 3_864_250_304),
    ("high", "Q6_K", "High", 5_263_239_104),
    ("max", "Q8_0", "Max", 6_577_440_704),
];

/// The tier used unless the user picked another in Settings → Images.
pub const DEFAULT_QUALITY: &str = "fast";

/// Text encoder and VAE are shared by every quant.
pub const ENCODER_FILE: &str = "Qwen3-4B-Instruct-2507-Q4_K_M.gguf";
pub const ENCODER_BYTES: u64 = 2_497_281_120;
pub const VAE_FILE: &str = "ae.safetensors";
pub const VAE_BYTES: u64 = 335_304_388;

/// Hard ceiling for one render; 1024×1024 measures ~6 min on a Navi14.
const GENERATE_TIMEOUT: Duration = Duration::from_secs(1200);
/// How long a first-use `allow_download` call waits for the fetch to finish.
pub const DOWNLOAD_WAIT: Duration = Duration::from_secs(1800);

#[derive(Clone, Debug)]
struct DownloadState {
    status: String,
    error: Option<String>,
}

#[derive(Clone)]
pub struct ImageGenClient {
    models_dir: PathBuf,
    sd_cli: Option<PathBuf>,
    downloads: Arc<Mutex<HashMap<String, DownloadState>>>,
}

/// What the tool asks the service to render.
pub struct GenerateRequest {
    pub prompt: String,
    pub quality: String,
    pub width: u32,
    pub height: u32,
    pub steps: u32,
    pub seed: i64,
    /// The Files-sandbox home the image is written into.
    pub home: PathBuf,
}

/// Where the finished image ended up + what it cost.
pub struct GenerateResult {
    pub file: String,
    pub rel_path: String,
    pub url: String,
    pub quality: String,
    pub quant: String,
    pub width: u32,
    pub height: u32,
    pub steps: u32,
    pub seed: i64,
    pub seconds: f64,
    pub bytes: u64,
}

pub fn human_bytes(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.1} GB", bytes as f64 / 1e9)
    } else {
        format!("{} MB", bytes / 1_000_000)
    }
}

/// Resolve a user/model-supplied quality: the tier key (`fast`), the quant
/// (`Q3_K`), or a loose spelling of it (`q3`, `q3k`).
pub fn resolve_quality(input: &str) -> Option<(&'static str, &'static str, &'static str, u64)> {
    let needle = input.trim().to_ascii_lowercase();
    QUALITIES.iter().copied().find(|(key, quant, ..)| {
        let quant = quant.to_ascii_lowercase();
        needle == *key || needle == quant || needle == quant.replace('_', "")
    })
}

pub fn quant_file(quant: &str) -> String {
    format!("z_image_turbo-{quant}.gguf")
}

/// The Files-sandbox home for a user — kept in lockstep with the Files plugin
/// (`plugins/files/src/fs_util.rs`): the real OS home when bound, otherwise the
/// virtual `~/.shiny/home/<id>`.
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

fn resolve_sd_cli(hint: Option<String>) -> Option<PathBuf> {
    if let Some(hint) = hint.filter(|h| !h.trim().is_empty()) {
        return Some(PathBuf::from(hint));
    }
    // Distribution build first (mirroring `build/qwentts.cpp/`), then the
    // benchmark checkout this machine has today.
    [
        "build/stable-diffusion.cpp/build/bin/sd-cli",
        "benchmarks/z-image/build/stable-diffusion.cpp/build/bin/sd-cli",
    ]
    .iter()
    .map(PathBuf::from)
    .find(|p| p.is_file())
}

/// True when an AMD GPU is present (vendor 0x1002 in DRM sysfs). The
/// integer-dot workaround only matters for RADV, but the flag is harmless.
fn amd_gpu_present() -> bool {
    let Ok(entries) = std::fs::read_dir("/sys/class/drm") else { return false };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with("card") && !name.contains('-') {
            if let Ok(vendor) = std::fs::read_to_string(entry.path().join("device/vendor")) {
                if vendor.trim() == "0x1002" {
                    return true;
                }
            }
        }
    }
    false
}

fn output_name(prompt: &str) -> String {
    let mut slug = String::new();
    for c in prompt.chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
        if slug.len() >= 40 {
            break;
        }
    }
    let slug = slug.trim_matches('-');
    let slug = if slug.is_empty() { "image" } else { slug };
    format!("{}-{}.png", chrono::Local::now().format("%Y%m%d-%H%M%S"), slug)
}

impl ImageGenClient {
    pub fn new(models_dir: PathBuf, sd_cli_hint: Option<String>) -> Self {
        Self {
            models_dir,
            sd_cli: resolve_sd_cli(sd_cli_hint),
            downloads: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn models_dir(&self) -> &Path {
        &self.models_dir
    }

    fn file_present(&self, name: &str) -> bool {
        self.models_dir.join(name).is_file()
    }

    pub fn quant_present(&self, quant: &str) -> bool {
        self.file_present(&quant_file(quant))
    }

    /// Shared files + the diffusion quant a tier needs, as human labels.
    pub fn missing_for(&self, quality: &str) -> Vec<String> {
        let Some((_, quant, ..)) = resolve_quality(quality) else {
            return vec![format!("unknown quality '{quality}'")];
        };
        let mut missing = Vec::new();
        if !self.file_present(ENCODER_FILE) {
            missing.push("Qwen3-4B text encoder".to_string());
        }
        if !self.file_present(VAE_FILE) {
            missing.push("VAE (ae.safetensors)".to_string());
        }
        if !self.quant_present(quant) {
            missing.push(format!("{quant} diffusion model"));
        }
        missing
    }

    /// Bytes still to fetch for a tier (0 when everything is on disk).
    pub fn missing_bytes_for(&self, quality: &str) -> u64 {
        let Some((_, quant, ..)) = resolve_quality(quality) else { return 0 };
        let mut bytes = 0;
        if !self.file_present(ENCODER_FILE) {
            bytes += ENCODER_BYTES;
        }
        if !self.file_present(VAE_FILE) {
            bytes += VAE_BYTES;
        }
        if !self.quant_present(quant) {
            bytes += QUALITIES
                .iter()
                .find(|(k, ..)| *k == quality || quant == quality)
                .map(|(.., b)| *b)
                .unwrap_or(0);
        }
        bytes
    }

    /// On-disk state of every tier — drives Settings → Images.
    pub fn inventory(&self) -> Value {
        let mut qualities = serde_json::Map::new();
        for (key, quant, label, bytes) in QUALITIES {
            qualities.insert(
                (*key).to_string(),
                json!({
                    "key": key,
                    "quant": quant,
                    "label": label,
                    "file": quant_file(quant),
                    "bytes": bytes,
                    "size": human_bytes(*bytes),
                    "present": self.quant_present(quant),
                    "ready": self.missing_for(key).is_empty(),
                }),
            );
        }
        json!({
            "dir": self.models_dir.to_string_lossy(),
            "sd_cli": self.sd_cli.as_ref().map(|p| p.to_string_lossy().to_string()),
            "default": DEFAULT_QUALITY,
            "encoder": {
                "file": ENCODER_FILE,
                "bytes": ENCODER_BYTES,
                "size": human_bytes(ENCODER_BYTES),
                "present": self.file_present(ENCODER_FILE),
            },
            "vae": {
                "file": VAE_FILE,
                "bytes": VAE_BYTES,
                "size": human_bytes(VAE_BYTES),
                "present": self.file_present(VAE_FILE),
            },
            "qualities": Value::Object(qualities),
        })
    }

    // ── Downloads (background) ───────────────────────────────────────────────

    /// Kick off `voice/download_zimage.py <target>` in the background.
    /// `target` is a tier key or `all`.
    pub fn start_download(&self, target: &str) -> Result<(), String> {
        let target = target.trim().to_lowercase();
        let keys: Vec<&str> = if target == "all" {
            QUALITIES.iter().map(|(k, ..)| *k).collect()
        } else if resolve_quality(&target).is_some() {
            vec![QUALITIES
                .iter()
                .find(|(k, ..)| *k == target)
                .map(|(k, ..)| *k)
                .unwrap_or(DEFAULT_QUALITY)]
        } else {
            return Err(format!("Unknown image quality: {target}"));
        };

        {
            let mut map = self.downloads.lock().unwrap();
            if map.get(&target).map(|s| s.status == "downloading").unwrap_or(false) {
                return Ok(());
            }
            for key in keys.iter().chain(["base"].iter()) {
                map.insert((*key).to_string(), DownloadState { status: "downloading".into(), error: None });
            }
        }

        let client = self.clone();
        let arg = target.clone();
        let tracked: Vec<String> = keys.iter().map(|k| (*k).to_string()).chain(["base".to_string()]).collect();
        tokio::spawn(async move {
            let models_dir = client.models_dir.to_string_lossy().to_string();
            let script_arg = arg.clone();
            let result = tokio::task::spawn_blocking(move || {
                Command::new("python3")
                    .arg("voice/download_zimage.py")
                    .arg(&script_arg)
                    .env("Z_IMAGE_MODELS_DIR", &models_dir)
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
            tracing::info!(target = %arg, status = %state.status, "Z-Image model download finished");
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
        let mut out = serde_json::Map::new();

        let part_bytes = |file: &str| -> u64 {
            std::fs::metadata(self.models_dir.join(format!("{file}.part"))).map(|m| m.len()).unwrap_or(0)
        };
        let base_present = self.file_present(ENCODER_FILE) && self.file_present(VAE_FILE);
        let base_ready = part_bytes(ENCODER_FILE) + part_bytes(VAE_FILE) == 0 && base_present;
        let base_state = map.get("base");
        let base_status = base_state.map(|s| s.status.clone()).unwrap_or_else(|| {
            if base_present { "ready".into() } else { "absent".into() }
        });
        out.insert(
            "base".to_string(),
            json!({
                "status": base_status,
                "error": base_state.and_then(|s| s.error.clone()),
                "bytes": if base_ready { 0 } else { part_bytes(ENCODER_FILE) + part_bytes(VAE_FILE) },
                "total": ENCODER_BYTES + VAE_BYTES,
                "present": base_present,
            }),
        );

        for (key, quant, ..) in QUALITIES {
            let state = map.get(*key);
            let file = quant_file(quant);
            let status = state.map(|s| s.status.clone()).unwrap_or_else(|| {
                if self.quant_present(quant) { "ready".into() } else { "absent".into() }
            });
            out.insert(
                (*key).to_string(),
                json!({
                    "status": status,
                    "error": state.and_then(|s| s.error.clone()),
                    "bytes": part_bytes(&file),
                    "total": QUALITIES.iter().find(|(k, ..)| k == key).map(|(.., b)| *b).unwrap_or(0),
                    "present": self.quant_present(quant),
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
                map.values().any(|s| s.status == "error")
            };
            if failed || Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }

    // ── Generation (blocking; call from spawn_blocking) ──────────────────────

    pub fn generate(&self, req: &GenerateRequest) -> Result<GenerateResult, String> {
        let Some((key, quant, ..)) = resolve_quality(&req.quality) else {
            return Err(format!("unknown quality: {}", req.quality));
        };
        let missing = self.missing_for(key);
        if !missing.is_empty() {
            return Err(format!("models-missing: {}", missing.join(", ")));
        }
        let cli = self
            .sd_cli
            .as_ref()
            .ok_or_else(|| "sd-cli is not available on this machine (Z_IMAGE_SD_CLI unset and no build found)".to_string())?;

        // Keep the graphs legal: sane bounds, 16-px alignment.
        let width = (req.width.clamp(256, 1536) / 16) * 16;
        let height = (req.height.clamp(256, 1536) / 16) * 16;

        let dir = req.home.join("Pictures").join("Generated");
        std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        let name = output_name(&req.prompt);
        let out = dir.join(&name);

        let log_dir = PathBuf::from("data/imagegen-logs");
        std::fs::create_dir_all(&log_dir).ok();
        let log_path = log_dir.join(format!("{}.log", name.trim_end_matches(".png")));
        let log = File::create(&log_path).map_err(|e| format!("cannot open render log: {e}"))?;
        let log_err = log.try_clone().map_err(|e| format!("cannot open render log: {e}"))?;

        let mut cmd = Command::new(cli);
        cmd.arg("--diffusion-model").arg(self.models_dir.join(quant_file(quant)))
            .arg("--vae").arg(self.models_dir.join(VAE_FILE))
            .arg("--llm").arg(self.models_dir.join(ENCODER_FILE))
            .arg("--backend").arg("te=cpu")
            .arg("--offload-to-cpu")
            .arg("--diffusion-fa")
            .arg("--max-vram").arg("2")
            .arg("--vae-conv-direct");
        if let Some(threshold) = cachedit_threshold() {
            cmd.arg("--cache-mode")
                .arg("cache-dit")
                .arg("--cache-option")
                .arg(format!("warmup=1,threshold={threshold}"));
        }
        if width * height >= 1_048_576 {
            cmd.arg("--vae-tiling");
        }
        cmd.arg("-p").arg(&req.prompt)
            .arg("--cfg-scale").arg("1.0")
            .arg("-W").arg(width.to_string())
            .arg("-H").arg(height.to_string())
            .arg("--steps").arg(req.steps.to_string())
            .arg("-s").arg(req.seed.to_string())
            .arg("-o").arg(&out);
        if std::env::var_os("GGML_VK_DISABLE_INTEGER_DOT_PRODUCT").is_none() && amd_gpu_present() {
            cmd.env("GGML_VK_DISABLE_INTEGER_DOT_PRODUCT", "1");
        }
        cmd.stdin(Stdio::null()).stdout(log).stderr(log_err);

        let start = Instant::now();
        let mut child = cmd.spawn().map_err(|e| format!("could not start sd-cli: {e}"))?;
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    if !status.success() {
                        return Err(format!(
                            "image generation failed ({status}): {}",
                            last_log_error(&log_path)
                        ));
                    }
                    break;
                }
                Ok(None) => {}
                Err(e) => return Err(format!("could not track sd-cli: {e}")),
            }
            if start.elapsed() > GENERATE_TIMEOUT {
                let _ = child.kill();
                let _ = child.wait();
                return Err("image generation timed out".into());
            }
            std::thread::sleep(Duration::from_millis(250));
        }

        let meta = std::fs::metadata(&out)
            .map_err(|e| format!("sd-cli reported success but {} is missing: {e}", out.display()))?;
        tracing::info!(
            quality = %key, quant = %quant, width, height, steps = req.steps,
            seconds = start.elapsed().as_secs_f64(), "image generated"
        );
        Ok(GenerateResult {
            file: name.clone(),
            rel_path: format!("Pictures/Generated/{name}"),
            url: format!("/api/imagegen/images/{name}"),
            quality: (*key).to_string(),
            quant: (*quant).to_string(),
            width,
            height,
            steps: req.steps,
            seed: req.seed,
            seconds: start.elapsed().as_secs_f64(),
            bytes: meta.len(),
        })
    }
}

/// Cache-DiT threshold from `Z_IMAGE_CACHEDIT` (default `0.30`; `0`/`off`
/// disables the cache). 0.30 measured ~1.5× at 512² with no visible quality
/// loss; `warmup=1` is required because the default warmup of 8 never caches
/// inside an 8-step turbo run.
fn cachedit_threshold() -> Option<String> {
    let raw = std::env::var("Z_IMAGE_CACHEDIT").unwrap_or_default();
    let raw = raw.trim();
    if raw.eq_ignore_ascii_case("off") || raw == "0" {
        return None;
    }
    if !raw.is_empty() && raw.parse::<f32>().is_ok() {
        return Some(raw.to_string());
    }
    Some("0.30".to_string())
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

/// Last meaningful lines of an sd-cli log, for a failed render.
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
    fn quality_aliases_resolve() {
        assert_eq!(resolve_quality("fast").map(|(k, ..)| k), Some("fast"));
        assert_eq!(resolve_quality("Q4_K").map(|(k, ..)| k), Some("balanced"));
        assert_eq!(resolve_quality("q6k").map(|(k, ..)| k), Some("high"));
        assert!(resolve_quality("ultra").is_none());
    }

    #[test]
    fn output_names_are_slugged_and_unique() {
        let name = output_name("A red, fox! Sleeping");
        assert!(name.ends_with("a-red-fox-sleeping.png"), "{name}");
        assert!(name.starts_with("20"));
    }

    #[test]
    fn human_bytes_reads_naturally() {
        assert_eq!(human_bytes(3_143_559_104), "3.1 GB");
        assert_eq!(human_bytes(335_304_388), "335 MB");
    }
}
