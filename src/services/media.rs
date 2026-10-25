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

/// Remove the generated image/song markdown from an assistant reply.
///
/// The picture/track is rendered from the tool outcome as its own chat element,
/// and persisted as a `media` row, so keeping the markdown in the reply would
/// render it twice (and reopen the chat with a duplicate). Only generated media
/// URLs are removed; an image or link the user pasted stays.
pub fn strip_generated_media(text: &str) -> String {
    let mut out = text.to_string();
    for prefix in ["/api/imagegen/images/", "/api/songgen/tracks/"] {
        let mut cursor = 0;
        while let Some(found) = out[cursor..].find(prefix) {
            let pos = cursor + found;
            let Some(close) = out[pos..].find(')').map(|i| pos + i) else { break };
            let before = &out[..pos];
            let start = before
                .rfind("![")
                .filter(|start| before[*start..].contains("]("))
                .or_else(|| {
                    before
                        .rfind('[')
                        .filter(|start| before[*start..].contains("]("))
                });
            match start {
                Some(start) => {
                    out.replace_range(start..=close, "");
                    cursor = start;
                }
                None => {
                    cursor = pos + prefix.len();
                }
            }
        }
    }
    // Removing a media line on its own leaves a run of blank lines behind;
    // markdown collapses them anyway, so keep the stored reply tidy.
    while out.contains("\n\n\n") {
        out = out.replace("\n\n\n", "\n\n");
    }
    out.trim().to_string()
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

#[cfg(test)]
mod tests {
    use super::strip_generated_media;

    #[test]
    fn removes_only_generated_media_markdown() {
        let reply = "Here it is:\n\n![red fox](/api/imagegen/images/20260101-fox.png)\n\nEnjoy.";
        assert_eq!(strip_generated_media(reply), "Here it is:\n\nEnjoy.");

        let song = "Made this: [Night drive](/api/songgen/tracks/night.wav)";
        assert_eq!(strip_generated_media(song), "Made this:");

        let user_media = "![photo](/home/me/photo.png) and [site](https://example.com)";
        assert_eq!(strip_generated_media(user_media), user_media);
    }
}
