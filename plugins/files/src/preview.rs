//! File-type classification, MIME mapping and image thumbnails.
//!
//! Pure helpers — no runtime state, no DB. The thumbnail encoder uses the
//! same `photon-rs` build the image plugin already links, so adding Files adds
//! no new tree to the workspace.

use std::path::Path;

use shiny_plugin_sdk::errors::AppError;

/// Lower-case extension without the dot (`""` when there is none).
pub fn ext_of(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// Content-Type for a path, by extension. A deliberately small table — the
/// browser only needs to know whether to paint or download it.
pub fn mime_for(path: &Path) -> &'static str {
    match ext_of(path).as_str() {
        // images
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        "tif" | "tiff" => "image/tiff",
        // video
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        "mov" => "video/quicktime",
        "ogv" => "video/ogg",
        // audio
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" | "oga" => "audio/ogg",
        "flac" => "audio/flac",
        "m4a" => "audio/mp4",
        "aac" => "audio/aac",
        // documents
        "pdf" => "application/pdf",
        "odt" => "application/vnd.oasis.opendocument.text",
        "ods" => "application/vnd.oasis.opendocument.spreadsheet",
        "odp" => "application/vnd.oasis.opendocument.presentation",
        // text / code
        "txt" | "md" | "log" | "rst" => "text/plain",
        "json" => "application/json",
        "xml" => "application/xml",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" | "mjs" | "cjs" | "ts" | "jsx" | "tsx" => "text/javascript",
        "rs" => "text/x-rust",
        "py" => "text/x-python",
        "sh" | "zsh" | "bash" => "text/x-sh",
        "toml" => "text/x-toml",
        "yaml" | "yml" => "text/x-yaml",
        "ini" | "conf" | "cfg" => "text/plain",
        "sql" => "text/x-sql",
        // archives
        "zip" => "application/zip",
        "tar" => "application/x-tar",
        "gz" | "tgz" => "application/gzip",
        "7z" => "application/x-7z-compressed",
        "rar" => "application/vnd.rar",
        _ => "application/octet-stream",
    }
}

/// Whether the browser can render the file as an image directly.
pub fn is_image(path: &Path) -> bool {
    matches!(
        ext_of(path).as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" | "avif" | "ico" | "tif" | "tiff"
    )
}

/// Whether photon-rs can decode it server-side for a thumbnail. SVG and AVIF
/// are decode-in-browser only.
pub fn is_thumbnailable_image(path: &Path) -> bool {
    matches!(
        ext_of(path).as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tif" | "tiff"
    )
}

pub fn is_text(path: &Path) -> bool {
    mime_for(path).starts_with("text/")
        || matches!(ext_of(path).as_str(), "json" | "xml" | "toml" | "yaml" | "yml")
}

pub fn is_office(path: &Path) -> bool {
    matches!(ext_of(path).as_str(), "odt" | "ods" | "odp")
}

pub fn is_pdf(path: &Path) -> bool {
    ext_of(path) == "pdf"
}

pub fn is_video(path: &Path) -> bool {
    mime_for(path).starts_with("video/")
}

pub fn is_audio(path: &Path) -> bool {
    mime_for(path).starts_with("audio/")
}

pub fn is_archive(path: &Path) -> bool {
    matches!(ext_of(path).as_str(), "zip" | "tar" | "gz" | "tgz" | "7z" | "rar")
}

/* ── Video via ffmpeg ───────────────────────────────────────────────────────
 *
 * Video frames are extracted **server-side with ffmpeg** rather than by a
 * `<video>` + `<canvas>` in the window. The browser route depends on the host
 * webview's own media stack — AVFoundation/QuickTime in the macOS shell,
 * GStreamer in WebKitGTK — so the same file yields a frame in Chrome and a
 * blank icon in the native browser, and codecs like MKV/HEVC/AV1 differ by
 * platform. ffmpeg is the one decoder every install shares, so the frame is
 * identical on macOS and Linux. The binaries can be overridden with
 * `FFMPEG_BIN` / `FFPROBE_BIN` for non-PATH installs.
 */

use std::process::Command;

fn tool(env_key: &str, default: &str) -> String {
    std::env::var(env_key)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| default.to_string())
}

pub fn ffmpeg_bin() -> String {
    tool("FFMPEG_BIN", "ffmpeg")
}

pub fn ffprobe_bin() -> String {
    tool("FFPROBE_BIN", "ffprobe")
}

/// Duration in seconds via `ffprobe` (`None` when unknown or ffprobe is absent).
pub fn video_duration_secs(path: &Path) -> Option<f64> {
    let out = Command::new(ffprobe_bin())
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(path)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    text.trim().parse::<f64>().ok().filter(|d| d.is_finite() && *d > 0.0)
}

/// One frame as PNG, at `at` seconds (or the first frame when `None`).
fn run_frame(path: &Path, at: Option<f64>, max: u32) -> Result<Vec<u8>, AppError> {
    let mut cmd = Command::new(ffmpeg_bin());
    cmd.args(["-hide_banner", "-loglevel", "error", "-an", "-sn"]);
    if let Some(t) = at {
        cmd.args(["-ss", &format!("{t:.3}")]);
    }
    cmd.arg("-i")
        .arg(path)
        .args([
            "-frames:v",
            "1",
            "-vf",
            // `-2` keeps the width even; `min(iw,…)` never upscales.
            &format!("scale='min(iw,{max})':-2"),
            "-f",
            "image2pipe",
            "-vcodec",
            "png",
            "pipe:1",
        ]);
    let out = cmd.output().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => {
            AppError::Internal("ffmpeg is not installed; cannot read video frames".into())
        }
        _ => AppError::Internal(format!("ffmpeg failed to start: {e}")),
    })?;
    if !out.status.success() || out.stdout.is_empty() {
        return Err(AppError::BadRequest("could not extract a video frame".into()));
    }
    Ok(out.stdout)
}

/// A representative frame near 10% of the video, falling back to the first
/// frame for clips too short to seek into.
pub fn video_thumbnail_png(path: &Path, max: u32) -> Result<Vec<u8>, AppError> {
    let max = max.clamp(32, 1024);
    let at = video_duration_secs(path)
        .map(|d| (d * 0.1).min(5.0))
        .filter(|t| *t > 0.05);
    match run_frame(path, at, max) {
        Ok(png) => Ok(png),
        Err(_) => run_frame(path, None, max),
    }
}

/// One frame at an explicit timestamp (used for poster/scrub requests).
pub fn video_frame_png(path: &Path, at: f64, max: u32) -> Result<Vec<u8>, AppError> {
    run_frame(path, Some(at.max(0.0)), max.clamp(32, 1024))
}

/// `ffprobe` metadata as JSON: duration, dimensions and stream codecs.
pub fn video_info_json(path: &Path) -> Option<serde_json::Value> {
    let out = Command::new(ffprobe_bin())
        .args([
            "-v",
            "error",
            "-show_format",
            "-show_streams",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let parsed: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    let duration = parsed
        .get("format")
        .and_then(|f| f.get("duration"))
        .and_then(|d| d.as_str())
        .and_then(|d| d.parse::<f64>().ok())
        .filter(|d| d.is_finite())
        .unwrap_or(0.0);
    let streams = parsed.get("streams").and_then(|s| s.as_array()).cloned().unwrap_or_default();
    let mut video_codec = None;
    let mut audio_codec = None;
    let mut width = 0u64;
    let mut height = 0u64;
    for s in &streams {
        match s.get("codec_type").and_then(|t| t.as_str()) {
            Some("video") if video_codec.is_none() => {
                video_codec = s.get("codec_name").and_then(|c| c.as_str()).map(str::to_string);
                width = s.get("width").and_then(|w| w.as_u64()).unwrap_or(0);
                height = s.get("height").and_then(|h| h.as_u64()).unwrap_or(0);
            }
            Some("audio") if audio_codec.is_none() => {
                audio_codec = s.get("codec_name").and_then(|c| c.as_str()).map(str::to_string);
            }
            _ => {}
        }
    }
    Some(serde_json::json!({
        "duration": duration,
        "width": width,
        "height": height,
        "video_codec": video_codec,
        "audio_codec": audio_codec,
    }))
}

/// Downscale image bytes to a PNG no larger than `max` on its long edge.
/// Returns `Err` when the bytes are not a decodable raster image.
pub fn thumbnail_png(bytes: &[u8], max: u32) -> Result<Vec<u8>, AppError> {
    let max = max.clamp(32, 1024);
    let mut img = photon_rs::native::open_image_from_bytes(bytes)
        .map_err(|e| AppError::BadRequest(format!("cannot decode image: {e}")))?;
    let (w, h) = (img.get_width(), img.get_height());
    if w == 0 || h == 0 {
        return Err(AppError::BadRequest("empty image".into()));
    }
    if w.max(h) > max {
        let scale = max as f32 / w.max(h) as f32;
        let nw = ((w as f32 * scale).round() as u32).max(1);
        let nh = ((h as f32 * scale).round() as u32).max(1);
        img = photon_rs::transform::resize(
            &img,
            nw,
            nh,
            photon_rs::transform::SamplingFilter::Lanczos3,
        );
    }
    Ok(img.get_bytes())
}
