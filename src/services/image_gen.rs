//! Local image generation through stable-diffusion.cpp (`sd-cli`).
//!
//! Two model families are supported, each with its own weights directory:
//! **Z-Image Turbo** and **Qwen-Image 2.1 Turbo**. The assistant's
//! `generate_image` tool drives this service: one `sd-cli` process per image,
//! so every byte of model state — the multi-GB text encoders included — is
//! released the moment the job ends. Weights are fetched on demand by
//! `voice/download_zimage.py` / `voice/download_qwen_image.py`; the shipped
//! default for each family is the `fast` tier (Q3_K).
//!
//! ## Why these flags — Z-Image
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
//! ## Why these flags — Qwen-Image 2.1 Turbo
//!
//! Same engine, measured in `benchmarks/qwen-image-2.1/README.md`: the 8-step
//! Turbo schedule must be passed as `--sigmas` (a plain `--steps 8` does not
//! reproduce it), `--max-vram 3` fits the bigger DiT beside the TTS sidecar,
//! `--model-args qwen_image_2_1_prefix_cache_type=q8_0` halves the prefix KV
//! cache, and the TAEQI2.1 tiny autoencoder (`--taesd`) replaces the full VAE
//! for encoding/decoding (2.8 s vs 57.6 s at 1024², no visible difference).
//! `--backend te=cpu` stays the safe placement for text-to-image; the GPU text
//! encoder only pays off for image editing, which the tool does not do yet.
//!
//! On RADV/Navi14 `GGML_VK_DISABLE_INTEGER_DOT_PRODUCT=1` is exported for both
//! families — the same Q4_K dequant-shader workaround the TTS sidecar and bench
//! scripts use.

use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

/// One local image model family and everything needed to run it.
pub struct ImageFamily {
    /// Stable id used by Settings, the `image.model` preference and the tool
    /// argument (e.g. `"z-image"`, `"qwen-image-2.1"`).
    pub id: &'static str,
    /// Human name for Settings and tool results.
    pub label: &'static str,
    /// The tier used unless the user picked another in Settings → Images.
    pub default_quality: &'static str,
    /// Advertised tiers, cheapest first: `(key, quant, label, file bytes)`.
    pub qualities: &'static [(&'static str, &'static str, &'static str, u64)],
    /// Files every tier needs: `(file, label, bytes)`.
    pub shared: &'static [(&'static str, &'static str, u64)],
    /// `python3 <script> <target>` fetches this family's files.
    pub download_script: &'static str,
    /// Env var the download script reads for its destination directory.
    pub download_env: &'static str,
    /// Diffusion file name; `{}` is the quant.
    pub quant_pattern: &'static str,
    /// Sizes must be multiples of this (sd.cpp requirement per model family).
    pub align: u32,
}

const Z_ENCODER_FILE: &str = "Qwen3-4B-Instruct-2507-Q4_K_M.gguf";
const Z_VAE_FILE: &str = "ae.safetensors";
const QWEN_ENCODER_FILE: &str = "Qwen3VL-8B-Instruct-Q4_K_M.gguf";
const QWEN_VAE_FILE: &str = "qwen_image_2.1_vae_bf16.safetensors";
const QWEN_TAE_FILE: &str = "taeqi2_1.safetensors";

/// Z-Image Turbo: 6B S3-DiT + Qwen3-4B encoder, FLUX-family VAE. Flag choices
/// and sizes are measured in `benchmarks/z-image/README.md`.
pub const Z_IMAGE: ImageFamily = ImageFamily {
    id: "z-image",
    label: "Z-Image Turbo",
    default_quality: "fast",
    qualities: &[
        ("fast", "Q3_K", "Fast", 3_143_559_104),
        ("balanced", "Q4_K", "Balanced", 3_864_250_304),
        ("high", "Q6_K", "High", 5_263_239_104),
        ("max", "Q8_0", "Max", 6_577_440_704),
    ],
    shared: &[
        (Z_ENCODER_FILE, "Qwen3-4B text encoder", 2_497_281_120),
        (Z_VAE_FILE, "VAE (ae.safetensors)", 335_304_388),
    ],
    download_script: "voice/download_zimage.py",
    download_env: "Z_IMAGE_MODELS_DIR",
    quant_pattern: "z_image_turbo-{}.gguf",
    align: 16,
};

/// Qwen-Image 2.1 Turbo: 7B DiT + Qwen3-VL-8B encoder, RGBA VAE, decoded by
/// the tiny TAEQI2.1 autoencoder. Flag choices and sizes are measured in
/// `benchmarks/qwen-image-2.1/README.md`. The weights are under the **Qwen
/// Research License (non-commercial)** — unlike Z-Image's Apache-2.0.
pub const QWEN_IMAGE: ImageFamily = ImageFamily {
    id: "qwen-image-2.1",
    label: "Qwen-Image 2.1 Turbo",
    default_quality: "fast",
    qualities: &[
        ("fast", "Q3_K", "Fast", 3_110_431_776),
        ("balanced", "Q4_K", "Balanced", 4_050_916_384),
        ("high", "Q6_K", "High", 5_876_562_976),
        ("max", "Q8_0", "Max", 7_591_558_176),
    ],
    shared: &[
        (QWEN_ENCODER_FILE, "Qwen3-VL 8B text encoder", 5_027_784_800),
        (QWEN_VAE_FILE, "VAE (qwen_image_2.1_vae_bf16)", 675_509_688),
        (QWEN_TAE_FILE, "Fast decoder (TAEQI2.1)", 30_591_264),
    ],
    download_script: "voice/download_qwen_image.py",
    download_env: "QWEN_IMAGE_MODELS_DIR",
    quant_pattern: "qwen-image-2.1-turbo-{}.gguf",
    align: 32,
};

/// Every family the app can render with, in Settings order.
pub const FAMILIES: &[&ImageFamily] = &[&Z_IMAGE, &QWEN_IMAGE];

/// The family used unless the user picked another in Settings → Images.
pub const DEFAULT_FAMILY: &str = "z-image";

/// The Turbo schedule saved in the official checkpoint (`model_index.json`'s
/// sample sigmas + a trailing 0.0). Without it an 8-step euler run uses the
/// wrong schedule; with it, a larger `--steps` is forced back to 8 by sd.cpp.
const QWEN_TURBO_SIGMAS: &str =
    "1.0,0.978453,0.95418,0.926626,0.89508,0.845148,0.704534,0.414568,0.0";

/// Hard ceiling for one render; Qwen at 1024×1024 measures ~8 min on a Navi14
/// before the fast-path flags, ~5 min after.
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
    /// One weights directory per family id.
    dirs: HashMap<&'static str, PathBuf>,
    sd_cli: Option<PathBuf>,
    /// Keyed `"<family>:<tier|base>"`.
    downloads: Arc<Mutex<HashMap<String, DownloadState>>>,
}

/// What the tool asks the service to render.
pub struct GenerateRequest {
    pub prompt: String,
    /// Family id (`"z-image"` / `"qwen-image-2.1"`).
    pub family: String,
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
    /// Family id the image was rendered with.
    pub model: String,
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

/// Resolve a user/model-supplied family: the id (`qwen-image-2.1`), the short
/// name (`qwen`), or a loose spelling (`Z-Image`).
pub fn resolve_family(input: &str) -> Option<&'static ImageFamily> {
    let needle = input.trim().to_ascii_lowercase().replace('_', "-");
    match needle.as_str() {
        "z-image" | "zimage" | "z" | "z-image-turbo" => Some(&Z_IMAGE),
        "qwen-image-2.1" | "qwen-image-21" | "qwen" | "qwen-image"
        | "qwen-image-2.1-turbo" | "qwen21" => Some(&QWEN_IMAGE),
        _ => None,
    }
}

/// Resolve a quality within a family: the tier key (`fast`), the quant
/// (`Q3_K`), or a loose spelling of it (`q3`, `q3k`).
pub fn resolve_quality(
    family: &ImageFamily,
    input: &str,
) -> Option<(&'static str, &'static str, &'static str, u64)> {
    let needle = input.trim().to_ascii_lowercase();
    family.qualities.iter().copied().find(|(key, quant, ..)| {
        let quant = quant.to_ascii_lowercase();
        needle == *key || needle == quant || needle == quant.replace('_', "")
    })
}

pub fn quant_file(family: &ImageFamily, quant: &str) -> String {
    family.quant_pattern.replace("{}", quant)
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
    // benchmark checkouts this machine has today. One binary serves both
    // families; what matters is that it is recent enough for Qwen-Image 2.1
    // (sd.cpp master since 2026-09-23).
    [
        "build/stable-diffusion.cpp/build/bin/sd-cli",
        "benchmarks/z-image/build/stable-diffusion.cpp/build/bin/sd-cli",
        "benchmarks/qwen-image-2.1/build/stable-diffusion.cpp/build/bin/sd-cli",
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
    /// `dirs` pairs each family id with its weights directory; families can be
    /// added to the registry without touching this constructor.
    pub fn new(dirs: Vec<(&'static str, PathBuf)>, sd_cli_hint: Option<String>) -> Self {
        Self {
            dirs: dirs.into_iter().collect(),
            sd_cli: resolve_sd_cli(sd_cli_hint),
            downloads: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn models_dir(&self, family: &ImageFamily) -> Option<&Path> {
        self.dirs.get(family.id).map(|p| p.as_path())
    }

    fn file_present(&self, family: &ImageFamily, name: &str) -> bool {
        self.models_dir(family)
            .map(|dir| dir.join(name).is_file())
            .unwrap_or(false)
    }

    pub fn quant_present(&self, family: &ImageFamily, quant: &str) -> bool {
        self.file_present(family, &quant_file(family, quant))
    }

    /// Shared files + the diffusion quant a tier needs, as human labels.
    pub fn missing_for(&self, family: &ImageFamily, quality: &str) -> Vec<String> {
        let Some((_, quant, ..)) = resolve_quality(family, quality) else {
            return vec![format!("unknown quality '{quality}'")];
        };
        let mut missing = Vec::new();
        for (file, label, ..) in family.shared {
            if !self.file_present(family, file) {
                missing.push((*label).to_string());
            }
        }
        if !self.quant_present(family, quant) {
            missing.push(format!("{quant} diffusion model"));
        }
        missing
    }

    /// Bytes still to fetch for a tier (0 when everything is on disk).
    pub fn missing_bytes_for(&self, family: &ImageFamily, quality: &str) -> u64 {
        let Some((_, quant, ..)) = resolve_quality(family, quality) else { return 0 };
        let mut bytes = 0;
        for (file, _, size) in family.shared {
            if !self.file_present(family, file) {
                bytes += size;
            }
        }
        if !self.quant_present(family, quant) {
            bytes += family
                .qualities
                .iter()
                .find(|(k, q, ..)| *k == quality || q == &quant)
                .map(|(.., b)| *b)
                .unwrap_or(0);
        }
        bytes
    }

    /// On-disk state of every family and tier — drives Settings → Images.
    pub fn inventory(&self) -> Value {
        let mut families = serde_json::Map::new();
        for family in FAMILIES {
            let mut qualities = serde_json::Map::new();
            for (key, quant, label, bytes) in family.qualities {
                qualities.insert(
                    (*key).to_string(),
                    json!({
                        "key": key,
                        "quant": quant,
                        "label": label,
                        "file": quant_file(family, quant),
                        "bytes": bytes,
                        "size": human_bytes(*bytes),
                        "present": self.quant_present(family, quant),
                        "ready": self.missing_for(family, key).is_empty(),
                    }),
                );
            }
            let shared: Vec<Value> = family
                .shared
                .iter()
                .map(|(file, label, bytes)| {
                    json!({
                        "file": file,
                        "label": label,
                        "bytes": bytes,
                        "size": human_bytes(*bytes),
                        "present": self.file_present(family, file),
                    })
                })
                .collect();
            let dir = self.models_dir(family).map(|p| p.to_string_lossy().to_string());
            families.insert(
                family.id.to_string(),
                json!({
                    "id": family.id,
                    "label": family.label,
                    "default": family.default_quality,
                    "dir": dir,
                    "shared": shared,
                    "qualities": Value::Object(qualities),
                }),
            );
        }
        json!({
            "sd_cli": self.sd_cli.as_ref().map(|p| p.to_string_lossy().to_string()),
            "default_model": DEFAULT_FAMILY,
            "families": Value::Object(families),
        })
    }

    // ── Downloads (background) ───────────────────────────────────────────────

    /// Kick off this family's downloader in the background (`<tier>` or `all`).
    pub fn start_download(&self, family: &ImageFamily, target: &str) -> Result<(), String> {
        let target = target.trim().to_lowercase();
        let keys: Vec<&str> = if target == "all" {
            family.qualities.iter().map(|(k, ..)| *k).collect()
        } else if resolve_quality(family, &target).is_some() {
            vec![family
                .qualities
                .iter()
                .find(|(k, ..)| *k == target)
                .map(|(k, ..)| *k)
                .unwrap_or(family.default_quality)]
        } else {
            return Err(format!("Unknown image quality: {target}"));
        };

        let Some(dir) = self.models_dir(family) else {
            return Err(format!("No models directory configured for '{}'", family.id));
        };

        let base_key = format!("{}:base", family.id);
        {
            let mut map = self.downloads.lock().unwrap();
            if map.get(&base_key).map(|s| s.status == "downloading").unwrap_or(false) {
                return Ok(());
            }
            for key in keys.iter().chain(["base"].iter()) {
                map.insert(
                    format!("{}:{}", family.id, key),
                    DownloadState { status: "downloading".into(), error: None },
                );
            }
        }

        let client = self.clone();
        let arg = target.clone();
        let tracked: Vec<String> = keys
            .iter()
            .map(|k| format!("{}:{}", family.id, k))
            .chain([base_key])
            .collect();
        let script = family.download_script;
        let env_var = family.download_env;
        let dir = dir.to_path_buf();
        let family_id = family.id;
        tokio::spawn(async move {
            let models_dir = dir.to_string_lossy().to_string();
            let script_arg = arg.clone();
            let result = tokio::task::spawn_blocking(move || {
                Command::new("python3")
                    .arg(script)
                    .arg(&script_arg)
                    .env(env_var, &models_dir)
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
            tracing::info!(family = family_id, target = %arg, status = %state.status, "image model download finished");
            let mut map = client.downloads.lock().unwrap();
            for key in tracked {
                map.insert(key, state.clone());
            }
        });
        Ok(())
    }

    /// Per-family, per-tier download state, with live byte progress from the
    /// partial files.
    pub fn downloads(&self) -> Value {
        let map = self.downloads.lock().unwrap().clone();
        let mut out = serde_json::Map::new();
        for family in FAMILIES {
            let dir = self.models_dir(family);
            let part_bytes = |file: &str| -> u64 {
                dir.and_then(|d| std::fs::metadata(d.join(format!("{file}.part"))).ok())
                    .map(|m| m.len())
                    .unwrap_or(0)
            };
            let key_of = |key: &str| format!("{}:{}", family.id, key);

            let base_present = family
                .shared
                .iter()
                .all(|(file, ..)| self.file_present(family, file));
            let base_state = map.get(&key_of("base"));
            let base_status = base_state.map(|s| s.status.clone()).unwrap_or_else(|| {
                if base_present { "ready".into() } else { "absent".into() }
            });
            let mut entries = serde_json::Map::new();
            let base_total: u64 = family.shared.iter().map(|(.., b)| b).sum();
            let base_bytes: u64 = if base_present {
                0
            } else {
                family.shared.iter().map(|(f, ..)| part_bytes(f)).sum()
            };
            entries.insert(
                "base".to_string(),
                json!({
                    "status": base_status,
                    "error": base_state.and_then(|s| s.error.clone()),
                    "bytes": base_bytes,
                    "total": base_total,
                    "present": base_present,
                }),
            );

            for (key, quant, ..) in family.qualities {
                let state = map.get(&key_of(key));
                let file = quant_file(family, quant);
                let status = state.map(|s| s.status.clone()).unwrap_or_else(|| {
                    if self.quant_present(family, quant) { "ready".into() } else { "absent".into() }
                });
                let total = family
                    .qualities
                    .iter()
                    .find(|(k, ..)| k == key)
                    .map(|(.., b)| *b)
                    .unwrap_or(0);
                entries.insert(
                    (*key).to_string(),
                    json!({
                        "status": status,
                        "error": state.and_then(|s| s.error.clone()),
                        "bytes": part_bytes(&file),
                        "total": total,
                        "present": self.quant_present(family, quant),
                    }),
                );
            }
            out.insert(family.id.to_string(), Value::Object(entries));
        }
        Value::Object(out)
    }

    /// Wait for a tier (and the shared files) to become complete. Returns
    /// false on timeout or a reported download error for this family.
    pub async fn wait_for_ready(
        &self,
        family: &ImageFamily,
        quality: &str,
        timeout: Duration,
    ) -> bool {
        let deadline = Instant::now() + timeout;
        let prefix = format!("{}:", family.id);
        loop {
            if self.missing_for(family, quality).is_empty() {
                return true;
            }
            let failed = {
                let map = self.downloads.lock().unwrap();
                map.iter()
                    .any(|(k, s)| k.starts_with(&prefix) && s.status == "error")
            };
            if failed || Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }

    // ── Generation (blocking; call from spawn_blocking) ──────────────────────

    pub fn generate(&self, req: &GenerateRequest) -> Result<GenerateResult, String> {
        let Some(family) = resolve_family(&req.family) else {
            return Err(format!("unknown image model: {}", req.family));
        };
        let Some(dir) = self.models_dir(family) else {
            return Err(format!("no models directory configured for '{}'", family.id));
        };
        let Some((key, quant, ..)) = resolve_quality(family, &req.quality) else {
            return Err(format!("unknown quality: {}", req.quality));
        };
        let missing = self.missing_for(family, key);
        if !missing.is_empty() {
            return Err(format!("models-missing: {}", missing.join(", ")));
        }
        let cli = self
            .sd_cli
            .as_ref()
            .ok_or_else(|| "sd-cli is not available on this machine (Z_IMAGE_SD_CLI unset and no build found)".to_string())?;

        // Keep the graphs legal: sane bounds, family-specific alignment
        // (Z-Image 16 px, Qwen-Image 32 px).
        let align = family.align.max(1);
        let width = (req.width.clamp(256, 1536) / align) * align;
        let height = (req.height.clamp(256, 1536) / align) * align;

        let dir_out = req.home.join("Pictures").join("Generated");
        std::fs::create_dir_all(&dir_out)
            .map_err(|e| format!("cannot create {}: {e}", dir_out.display()))?;
        let name = output_name(&req.prompt);
        let out = dir_out.join(&name);

        let log_dir = PathBuf::from("data/imagegen-logs");
        std::fs::create_dir_all(&log_dir).ok();
        let log_path = log_dir.join(format!("{}.log", name.trim_end_matches(".png")));
        let log = File::create(&log_path).map_err(|e| format!("cannot open render log: {e}"))?;
        let log_err = log.try_clone().map_err(|e| format!("cannot open render log: {e}"))?;

        let mut cmd = Command::new(cli);
        cmd.arg("--diffusion-model").arg(dir.join(quant_file(family, quant)));
        match family.id {
            "qwen-image-2.1" => {
                // TAEQI2.1 replaces the full VAE for encode/decode (proven at
                // ~20× decode speed with no visible loss on the benchmark kit).
                cmd.arg("--llm").arg(dir.join(QWEN_ENCODER_FILE))
                    .arg("--taesd").arg(dir.join(QWEN_TAE_FILE))
                    .arg("--backend").arg("te=cpu")
                    .arg("--offload-to-cpu")
                    .arg("--diffusion-fa")
                    .arg("--max-vram").arg("3")
                    .arg("--model-args").arg("qwen_image_2_1_prefix_cache_type=q8_0")
                    .arg("--sampling-method").arg("euler")
                    .arg("--sigmas").arg(QWEN_TURBO_SIGMAS);
            }
            _ => {
                cmd.arg("--vae").arg(dir.join(Z_VAE_FILE))
                    .arg("--llm").arg(dir.join(Z_ENCODER_FILE))
                    .arg("--backend").arg("te=cpu")
                    .arg("--offload-to-cpu")
                    .arg("--diffusion-fa")
                    .arg("--max-vram").arg("2")
                    .arg("--vae-conv-direct");
            }
        }
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
            model = family.id, quality = %key, quant = %quant, width, height, steps = req.steps,
            seconds = start.elapsed().as_secs_f64(), "image generated"
        );
        Ok(GenerateResult {
            file: name.clone(),
            rel_path: format!("Pictures/Generated/{name}"),
            url: format!("/api/imagegen/images/{name}"),
            model: family.id.to_string(),
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
/// disables the cache). Shared by both families: 0.30 measured ~1.5× at 512²
/// (Z-Image) and ~2.4× on sampling (Qwen-Image, 4/8 steps skipped) with no
/// visible quality loss; `warmup=1` is required because the default warmup of
/// 8 never caches inside an 8-step turbo run.
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
        assert_eq!(resolve_quality(&Z_IMAGE, "fast").map(|(k, ..)| k), Some("fast"));
        assert_eq!(resolve_quality(&Z_IMAGE, "Q4_K").map(|(k, ..)| k), Some("balanced"));
        assert_eq!(resolve_quality(&Z_IMAGE, "q6k").map(|(k, ..)| k), Some("high"));
        assert!(resolve_quality(&Z_IMAGE, "ultra").is_none());
        assert_eq!(resolve_quality(&QWEN_IMAGE, "q3k").map(|(k, ..)| k), Some("fast"));
    }

    #[test]
    fn families_resolve() {
        assert_eq!(resolve_family("qwen").map(|f| f.id), Some("qwen-image-2.1"));
        assert_eq!(resolve_family("Qwen-Image-2.1").map(|f| f.id), Some("qwen-image-2.1"));
        assert_eq!(resolve_family("Z-Image").map(|f| f.id), Some("z-image"));
        assert!(resolve_family("midjourney").is_none());
    }

    #[test]
    fn quant_files_follow_each_pattern() {
        assert_eq!(quant_file(&Z_IMAGE, "Q3_K"), "z_image_turbo-Q3_K.gguf");
        assert_eq!(quant_file(&QWEN_IMAGE, "Q3_K"), "qwen-image-2.1-turbo-Q3_K.gguf");
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
