//! Shared helpers for the local media generators (`image_gen`, `song_gen`):
//! the Files-sandbox home, filename slugs, byte formatting, and the RADV
//! workaround probe. Both services write into the same per-user home and run
//! next to the same TTS sidecar, so these live in one place.

use std::path::PathBuf;

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

/// Lowercase ASCII slug for a file name: `"A red, fox! Sleeping"` →
/// `"a-red-fox-sleeping"` (at most `max` chars, `fallback` when nothing survives).
pub fn slug(text: &str, fallback: &str, max: usize) -> String {
    let mut out = String::new();
    for c in text.chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
        if out.len() >= max {
            break;
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() { fallback.to_string() } else { trimmed.to_string() }
}

/// A collision-free file name for a generated asset:
/// `"<YYYYmmdd-HHMMSS>-<slug>.<ext>"` (same shape the image generator always
/// used, so nothing that parses those names changes).
pub fn stamped_name(base: &str, fallback: &str, ext: &str) -> String {
    format!(
        "{}-{}.{ext}",
        chrono::Local::now().format("%Y%m%d-%H%M%S"),
        slug(base, fallback, 40)
    )
}

pub fn human_bytes(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.1} GB", bytes as f64 / 1e9)
    } else {
        format!("{} MB", bytes / 1_000_000)
    }
}

/// True when an AMD GPU is present (vendor 0x1002 in DRM sysfs). The
/// integer-dot workaround only matters for RADV, but the flag is harmless.
pub fn amd_gpu_present() -> bool {
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
