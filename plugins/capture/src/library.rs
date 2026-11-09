//! Where recordings live and what is known about them.
//!
//! Recordings are ordinary files in the user's Videos folder
//! (`$XDG_VIDEOS_DIR/LiteAura`), so Files and every player see them. What the
//! editor and the agent need beyond the file (markers, the event log, track
//! names) sits in a hidden JSON sidecar next to it: deleting or moving the
//! video in Files never leaves the library inconsistent.

use std::path::{Path, PathBuf};

/// `$XDG_VIDEOS_DIR` from `~/.config/user-dirs.dirs`, else `~/Videos`.
pub fn videos_dir(home: &Path) -> PathBuf {
    let conf = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute() && p.starts_with(home))
        .unwrap_or_else(|| home.join(".config"));
    if let Ok(s) = std::fs::read_to_string(conf.join("user-dirs.dirs")) {
        for line in s.lines() {
            if let Some(v) = line.trim().strip_prefix("XDG_VIDEOS_DIR=") {
                let v = v.trim().trim_matches('"');
                let p = v.replace("$HOME", &home.display().to_string());
                let p = PathBuf::from(p);
                if p.is_absolute() && p != home {
                    return p;
                }
            }
        }
    }
    home.join("Videos")
}

/// The folder Capture records into.
pub fn capture_dir(home: &Path) -> PathBuf {
    videos_dir(home).join("LiteAura")
}

/// The hidden metadata file of a recording.
pub fn sidecar(video: &Path) -> PathBuf {
    let name = video
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    video.with_file_name(format!(".{name}.capture.json"))
}

pub fn is_video(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        matches!(
            e.to_ascii_lowercase().as_str(),
            "mp4" | "mkv" | "webm" | "mov" | "m4v"
        )
    })
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct Recording {
    /// The file name: unique inside the folder, stable across restarts.
    pub id: String,
    pub path: PathBuf,
    pub bytes: u64,
    pub modified: u64,
    pub meta: serde_json::Value,
}

/// Every video in `dir`, newest first.
pub fn list(dir: &Path) -> Vec<Recording> {
    let mut out: Vec<Recording> = std::fs::read_dir(dir)
        .map(|d| {
            d.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| is_video(p))
                .filter(|p| {
                    !p.file_name()
                        .is_some_and(|n| n.to_string_lossy().starts_with('.'))
                })
                .filter_map(|p| {
                    let md = std::fs::metadata(&p).ok()?;
                    let modified = md
                        .modified()
                        .ok()?
                        .duration_since(std::time::UNIX_EPOCH)
                        .ok()?
                        .as_millis() as u64;
                    let meta = std::fs::read(sidecar(&p))
                        .ok()
                        .and_then(|b| serde_json::from_slice(&b).ok())
                        .unwrap_or(serde_json::Value::Null);
                    Some(Recording {
                        id: p.file_name()?.to_string_lossy().to_string(),
                        path: p,
                        bytes: md.len(),
                        modified,
                        meta,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by_key(|r| std::cmp::Reverse(r.modified));
    out
}

/// Resolve a recording id (a bare file name) inside `dir`; `None` for
/// anything that would leave it.
pub fn resolve(dir: &Path, id: &str) -> Option<PathBuf> {
    if id.is_empty() || id.contains('/') || id.contains('\\') || id.starts_with('.') {
        return None;
    }
    let p = dir.join(id);
    (p.is_file() && is_video(&p)).then_some(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_refuses_escapes() {
        let d = Path::new("/tmp");
        assert!(resolve(d, "../etc/passwd").is_none());
        assert!(resolve(d, ".hidden.mp4").is_none());
        assert!(resolve(d, "").is_none());
    }

    #[test]
    fn sidecar_is_hidden() {
        assert_eq!(
            sidecar(Path::new("/v/Capture 1.mp4")),
            PathBuf::from("/v/.Capture 1.mp4.capture.json")
        );
    }
}
