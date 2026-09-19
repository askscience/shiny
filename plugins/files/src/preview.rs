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
