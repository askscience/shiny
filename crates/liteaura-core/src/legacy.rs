//! One-time move of the files an install made under its former name, `peakd`.
//!
//! The shell used to be called `peakd`, so an existing install keeps its
//! settings in `~/.config/peakd`, its cache in `~/.cache/peakd`, and its web
//! profile (cookies, site logins, local storage) under
//! `~/.local/share/peakd/QtWebEngine/peakd`. Without this move a renamed shell
//! would start as a fresh install and sign the user out of every site.
//!
//! Each move is a plain `rename`: atomic, and skipped when the new location
//! already exists, so it runs once and never overwrites anything. Errors are
//! ignored: the old files stay where they were, and nothing worse than a
//! fresh profile follows.

use std::{
    env, fs,
    path::{Path, PathBuf},
};

/// The shell's former name.
pub const LEGACY_NAME: &str = "peakd";
/// The shell's name.
pub const NAME: &str = "liteaura";

/// Move every legacy `peakd` directory to its `liteaura` place. Call it first
/// thing in `main`, before anything reads or creates those directories.
pub fn migrate_legacy_dirs() {
    for (xdg, fallback) in [
        ("XDG_CONFIG_HOME", ".config"),
        ("XDG_CACHE_HOME", ".cache"),
        ("XDG_DATA_HOME", ".local/share"),
    ] {
        let Some(root) = xdg_root(xdg, fallback) else { continue };
        let dir = root.join(NAME);
        move_dir(&root.join(LEGACY_NAME), &dir);
        // QtWebEngine keeps the profile in a folder named after its storage
        // name, which was `peakd` too.
        move_dir(&dir.join("QtWebEngine").join(LEGACY_NAME), &dir.join("QtWebEngine").join(NAME));
    }

    // WKWebView keys its website data by bundle identifier.
    #[cfg(target_os = "macos")]
    if let Some(home) = env::var_os("HOME").map(PathBuf::from) {
        let library = home.join("Library");
        for sub in ["WebKit", "HTTPStorages", "Caches"] {
            move_dir(
                &library.join(sub).join("com.askscience.peakd"),
                &library.join(sub).join("com.askscience.liteaura"),
            );
        }
    }
}

/// Copy every `PEAKD_*` variable to its `LITEAURA_*` name when that is unset.
///
/// A system installed under the former name keeps its session script and
/// `/etc/environment`, which set `PEAKD_LANG`, `PEAKD_AI_DEFAULT` and so on.
/// Call it at the very start of `main`, before any thread is spawned.
pub fn adopt_legacy_env() {
    for (key, value) in env::vars_os() {
        let Some(rest) = key.to_str().and_then(|k| k.strip_prefix("PEAKD_")) else { continue };
        let current = format!("LITEAURA_{rest}");
        if env::var_os(&current).is_none() {
            env::set_var(current, value);
        }
    }
}

fn xdg_root(xdg: &str, fallback: &str) -> Option<PathBuf> {
    env::var_os(xdg)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(fallback)))
}

/// Rename `from` to `to` when `from` is a directory and `to` does not exist.
/// Returns whether it moved.
pub fn move_dir(from: &Path, to: &Path) -> bool {
    if !from.is_dir() || to.exists() {
        return false;
    }
    if let Some(parent) = to.parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::rename(from, to).is_ok()
}

#[cfg(test)]
mod tests {
    use super::{adopt_legacy_env, move_dir};

    #[test]
    fn legacy_env_fills_only_unset_names() {
        std::env::set_var("PEAKD_LEGACY_TEST_A", "old");
        std::env::set_var("PEAKD_LEGACY_TEST_B", "old");
        std::env::set_var("LITEAURA_LEGACY_TEST_B", "new");
        adopt_legacy_env();
        assert_eq!(std::env::var("LITEAURA_LEGACY_TEST_A").unwrap(), "old");
        assert_eq!(std::env::var("LITEAURA_LEGACY_TEST_B").unwrap(), "new");
    }

    #[test]
    fn moves_once_and_never_overwrites() {
        let root = std::env::temp_dir().join(format!("liteaura-legacy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let old = root.join("peakd");
        let new = root.join("liteaura");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("display.json"), "{}").unwrap();

        assert!(move_dir(&old, &new));
        assert!(new.join("display.json").is_file());
        assert!(!old.exists());

        // A second legacy folder appearing later must not replace the new one.
        std::fs::create_dir_all(&old).unwrap();
        assert!(!move_dir(&old, &new));
        assert!(old.exists() && new.join("display.json").is_file());

        // Nothing to move.
        assert!(!move_dir(&root.join("missing"), &root.join("other")));
        let _ = std::fs::remove_dir_all(&root);
    }
}
