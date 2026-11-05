//! Whether the binary this process runs from is still the one on disk.
//!
//! A package upgrade replaces `/usr/bin/liteaura` while the shell keeps
//! running the old image. The shell does not relaunch at once — that would
//! take the screen away from the user — but behind the next locked screen,
//! where a one-second relaunch goes unseen. This module is the check.

use std::sync::OnceLock;
use std::time::SystemTime;

/// Size and modification time of the executable at startup.
static STARTED_FROM: OnceLock<Option<(u64, SystemTime)>> = OnceLock::new();

fn stat(path: &std::path::Path) -> Option<(u64, SystemTime)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.len(), meta.modified().ok()?))
}

fn exe() -> std::path::PathBuf {
    let current = std::env::current_exe().unwrap_or_default();
    let text = current.to_string_lossy();
    match text.strip_suffix(" (deleted)") {
        Some(clean) => std::path::PathBuf::from(clean),
        None => current,
    }
}

/// Take the startup snapshot. Call it first thing in `main`; a later first
/// call would snapshot an already replaced file and never notice it.
pub fn remember() {
    STARTED_FROM.get_or_init(|| stat(&exe()));
}

/// True when a different binary now sits at this program's path: the
/// kernel reports the running image as deleted, or the file's size or
/// modification time moved since [`remember`].
pub fn replaced() -> bool {
    let current = std::env::current_exe().unwrap_or_default();
    if current.to_string_lossy().ends_with(" (deleted)") {
        return true;
    }
    match STARTED_FROM.get_or_init(|| stat(&exe())) {
        Some(started) => stat(&exe()).is_some_and(|now| now != *started),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_running_binary_is_not_replaced() {
        super::remember();
        assert!(!super::replaced());
    }
}
