//! Content fingerprints for what the shell serves from disk.
//!
//! A digest answers "did these files change?" without reading them: every
//! regular file's relative path, size and modification time go through SHA-256
//! and the first 16 hex characters come out. A package upgrade, a `cp` of a
//! rebuilt plugin or an edited stylesheet all change at least one of those.
//!
//! Two digests exist per plugin because they trigger different work:
//! - **code** (`plugin.toml`, the cdylib, `migrations/`, `skills/`) means the
//!   server has to load the plugin again;
//! - **web** (`web/`) only means a window shows stale code until it reloads
//!   its module, which the page does on its own.
//!
//! The walk is cheap (a `stat` per file), so it can run every few seconds.

use sha2::{Digest, Sha256};
use std::path::Path;
use std::time::UNIX_EPOCH;

/// A plugin directory's fingerprints. `all` combines the two.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PluginDigest {
    pub code: String,
    pub web: String,
    pub all: String,
}

impl PluginDigest {
    pub fn of(dir: &Path) -> Self {
        let code = tree_digest(dir, &|rel| {
            // Only the plugin's own code: the web tree has its own digest and
            // backups (`<name>.bak`) are not inside the plugin directory.
            let top = rel.split('/').next().unwrap_or("");
            match top {
                "web" => false,
                "plugin.toml" | "migrations" | "skills" => true,
                _ => rel.ends_with(".so") || rel.ends_with(".dylib") || rel.ends_with(".dll"),
            }
        });
        let web = tree_digest(&dir.join("web"), &|_| true);
        Self::combine(code, web)
    }

    /// The combined fingerprint of a code digest and a web digest.
    pub fn combine(code: String, web: String) -> Self {
        let all = short_hex(&format!("{code}:{web}"));
        Self { code, web, all }
    }
}

/// Fingerprint of the core web tree. Backgrounds are user pictures, not code.
pub fn web_tree_digest(web_dir: &Path) -> String {
    tree_digest(web_dir, &|rel| !rel.starts_with("backgrounds/"))
}

/// Fingerprint of one file (the server binary): its size and mtime.
pub fn file_digest(path: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(path.to_string_lossy().as_bytes());
    if let Ok(meta) = std::fs::metadata(path) {
        hash_meta(&mut hasher, &meta);
    }
    short(hasher)
}

/// Walk `root` and hash every regular file `keep` accepts, in a stable order.
/// A missing root digests to the empty tree, so "absent" and "empty" compare
/// equal and a later appearance still counts as a change.
pub fn tree_digest(root: &Path, keep: &dyn Fn(&str) -> bool) -> String {
    let mut entries: Vec<(String, std::fs::Metadata)> = Vec::new();
    collect(root, root, keep, &mut entries);
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let mut hasher = Sha256::new();
    for (rel, meta) in &entries {
        hasher.update(rel.as_bytes());
        hasher.update([0]);
        hash_meta(&mut hasher, meta);
    }
    short(hasher)
}

fn collect(root: &Path, dir: &Path, keep: &dyn Fn(&str) -> bool, out: &mut Vec<(String, std::fs::Metadata)>) {
    let Ok(read) = std::fs::read_dir(dir) else { return };
    for entry in read.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || name == "node_modules" || name == "__pycache__" {
            continue;
        }
        let Ok(rel) = path.strip_prefix(root) else { continue };
        let rel = rel.to_string_lossy().replace('\\', "/");
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_dir() {
            collect(root, &path, keep, out);
        } else if meta.is_file() && keep(&rel) {
            out.push((rel, meta));
        }
    }
}

fn hash_meta(hasher: &mut Sha256, meta: &std::fs::Metadata) {
    hasher.update(meta.len().to_le_bytes());
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    hasher.update(mtime.to_le_bytes());
}

fn short(hasher: Sha256) -> String {
    hex::encode(hasher.finalize())[..16].to_string()
}

fn short_hex(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    short(hasher)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(path: &Path, bytes: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn code_and_web_change_independently() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        touch(&root.join("plugin.toml"), b"name = \"x\"");
        touch(&root.join("libshiny_x_plugin.so"), b"so");
        touch(&root.join("web/plugin.js"), b"export default {}");
        let first = PluginDigest::of(root);
        assert_eq!(first, PluginDigest::of(root), "stable across calls");

        // A bigger stylesheet: only the web digest moves.
        touch(&root.join("web/style.css"), b"body{}");
        let web_changed = PluginDigest::of(root);
        assert_eq!(web_changed.code, first.code);
        assert_ne!(web_changed.web, first.web);
        assert_ne!(web_changed.all, first.all);

        // A rebuilt library with a different size: the code digest moves.
        touch(&root.join("libshiny_x_plugin.so"), b"so-v2");
        let code_changed = PluginDigest::of(root);
        assert_ne!(code_changed.code, web_changed.code);
        assert_eq!(code_changed.web, web_changed.web);
    }

    #[test]
    fn hidden_files_and_backgrounds_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        touch(&root.join("js/app.js"), b"1");
        let base = web_tree_digest(root);
        touch(&root.join(".cache"), b"x");
        touch(&root.join("backgrounds/photo.png"), b"png");
        assert_eq!(web_tree_digest(root), base);
        touch(&root.join("js/app.js"), b"12");
        assert_ne!(web_tree_digest(root), base);
    }

    #[test]
    fn a_missing_tree_is_the_empty_tree() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope");
        assert_eq!(tree_digest(&missing, &|_| true), tree_digest(dir.path(), &|_| true));
    }
}
