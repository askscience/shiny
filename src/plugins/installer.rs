//! Plugin installer — extracts `.zip` / `.tar.gz` archives into `data/plugins/<name>/`.
//! Used by the admin HTTP API.
//!
//! All installation attempts (success and failure) are appended to
//! `data/plugins/install.log` so admins can audit issues offline.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::manifest::Manifest;
use shiny_plugin_sdk::services::PluginCtx;

use crate::plugins::loader::find_cdylib;
use crate::plugins::manager::PluginManager;

/// Append a single line to `<plugins_dir>/install.log`.
pub(crate) fn log_event(plugins_dir: &Path, line: &str) {
    let log_path = plugins_dir.join("install.log");
    if let Some(parent) = log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    let entry = format!("[{ts}] {line}\n");
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&log_path) {
        let _ = f.write_all(entry.as_bytes());
    }
}

/// Recognized archive formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveFormat {
    Zip,
    TarGz,
    Tar,
}

impl ArchiveFormat {
    pub fn sniff(bytes: &[u8]) -> Option<Self> {
        if bytes.len() >= 4 && bytes[0..2] == [b'P', b'K'] {
            return Some(ArchiveFormat::Zip);
        }
        if bytes.len() >= 2 && bytes[0] == 0x1f && bytes[1] == 0x8b {
            return Some(ArchiveFormat::TarGz);
        }
        if bytes.len() >= 512 {
            // tar magic at offset 257 is "ustar"
            if &bytes[257..262] == b"ustar" {
                return Some(ArchiveFormat::Tar);
            }
        }
        None
    }
}

/// Install an uploaded plugin archive from in-memory bytes.
pub async fn install_archive(
    bytes: Vec<u8>,
    plugins_dir: &Path,
    manager: &PluginManager,
    base_ctx: Arc<PluginCtx>,
) -> Result<String, AppError> {
    log_event(plugins_dir, &format!("install-begin bytes={}", bytes.len()));
    let format = match ArchiveFormat::sniff(&bytes) {
        Some(f) => f,
        None => {
            log_event(plugins_dir, "reject-format unknown-bytes");
            return Err(AppError::BadRequest(
                "Unrecognised archive format (expected .zip or .tar.gz)".into(),
            ));
        }
    };
    log_event(plugins_dir, &format!(
        "format-detected format={:?}", format,
    ));

    // Stage the extraction in a tmp dir, then move to the canonical location.
    let staging = unique_staging_dir(plugins_dir);
    std::fs::create_dir_all(&staging)?;
    let _staging_cleanup = StagingCleanup(staging.clone());

    if let Err(e) = extract(&bytes, format, &staging) {
        log_event(plugins_dir, &format!("extract-failed: {e}"));
        let _ = std::fs::remove_dir_all(&staging);
        return Err(e);
    }

    // If the archive contains a single top-level dir, descend into it.
    let install_dir = find_install_root(&staging);

    let manifest_text = match std::fs::read_to_string(install_dir.join("plugin.toml")) {
        Ok(t) => t,
        Err(e) => {
            log_event(plugins_dir, &format!("manifest-read-failed: {e}"));
            let _ = std::fs::remove_dir_all(&staging);
            return Err(AppError::BadRequest("plugin.toml not found at archive root".into()));
        }
    };
    let manifest: Manifest = match toml::from_str(&manifest_text) {
        Ok(m) => m,
        Err(e) => {
            log_event(plugins_dir, &format!("manifest-parse-failed: {e}"));
            let _ = std::fs::remove_dir_all(&staging);
            return Err(AppError::BadRequest(format!("Invalid plugin.toml: {}", e)));
        }
    };
    // The manifest name is joined into `plugins_dir` below — reject anything
    // that is not a single safe path component (path traversal).
    if let Err(e) = crate::plugins::loader::validate_plugin_name(&manifest.name) {
        log_event(plugins_dir, &format!("reject-name name={:?}", manifest.name));
        let _ = std::fs::remove_dir_all(&staging);
        return Err(e);
    }
    log_event(plugins_dir, &format!(
        "manifest-ok name={} version={} api_level={}",
        manifest.name, manifest.version, manifest.api_level
    ));

    if manifest.api_level > shiny_plugin_sdk::CORE_API_LEVEL {
        log_event(plugins_dir, &format!(
            "reject api_level {} > core {}",
            manifest.api_level, shiny_plugin_sdk::CORE_API_LEVEL
        ));
        let _ = std::fs::remove_dir_all(&staging);
        return Err(AppError::BadRequest(format!(
            "Plugin api_level {} > core {}", manifest.api_level, shiny_plugin_sdk::CORE_API_LEVEL
        )));
    }

    // Validate a cdylib is present.
    if find_cdylib(&install_dir, &manifest.name).is_none() {
        log_event(plugins_dir, &format!(
            "cdylib-missing name={}", manifest.name
        ));
        let _ = std::fs::remove_dir_all(&staging);
        return Err(AppError::BadRequest(format!(
            "Archive does not contain a cdylib for plugin '{}'", manifest.name
        )));
    }

    // Installation lock — serializes concurrent installs. Two layers: an
    // in-process async mutex (same-server races) and an advisory file lock
    // (multi-process). Held until the end of `install_archive`.
    static INSTALL_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _process_guard = INSTALL_MUTEX.lock().await;
    let lockfile = plugins_dir.join(".install.lock");
    let _lock = acquire_lock(&lockfile).await?;

    let final_dir = plugins_dir.join(&manifest.name);
    let backup = plugins_dir.join(format!("{}.bak", manifest.name));
    let mut swap = InstallSwap::prepare(&install_dir, &final_dir, &backup)?;
    let _ = std::fs::remove_dir_all(&staging);

    // Disk changes remain provisional until the manager has prepared and
    // published the new registration. Cancellation also restores the files.
    let installed_name = match manager.install_dir_static(&final_dir, base_ctx).await {
        Ok(name) => name,
        Err(error) => {
            log_event(plugins_dir, &format!("register-failed name={}: {error}", manifest.name));
            if let Err(rollback) = swap.rollback() {
                log_event(plugins_dir, &format!("rollback-failed name={}: {rollback}", manifest.name));
                return Err(AppError::Internal(format!("Plugin install failed: {error}; restoring previous files failed: {rollback}")));
            }
            log_event(plugins_dir, &format!("files-restored name={}", manifest.name));
            return Err(error);
        }
    };
    swap.committed = true;
    log_event(plugins_dir, &format!(
        "install-ok name={}", installed_name
    ));
    Ok(installed_name)
}

struct StagingCleanup(PathBuf);
impl Drop for StagingCleanup {
    fn drop(&mut self) {
        if self.0.exists() {
            if let Err(error) = std::fs::remove_dir_all(&self.0) {
                tracing::warn!("Could not clean plugin staging directory {}: {error}", self.0.display());
            }
        }
    }
}

/// Filesystem half of an installation. Database migrations are independently
/// committed; restoring these files never claims to undo a committed schema.
struct InstallSwap {
    destination: PathBuf,
    backup: PathBuf,
    previous: bool,
    installed: bool,
    committed: bool,
}

impl InstallSwap {
    fn prepare(staged: &Path, destination: &Path, backup: &Path) -> Result<Self, AppError> {
        let previous = destination.exists();
        if previous {
            if backup.exists() { std::fs::remove_dir_all(backup)?; }
            std::fs::rename(destination, backup)?;
        }
        let mut swap = Self { destination: destination.into(), backup: backup.into(), previous,
            installed: false, committed: false };
        if let Err(error) = std::fs::rename(staged, destination) {
            if let Err(rollback) = swap.rollback() {
                return Err(AppError::Internal(format!("Plugin rename failed: {error}; restore failed: {rollback}")));
            }
            return Err(error.into());
        }
        swap.installed = true;
        Ok(swap)
    }

    fn rollback(&mut self) -> Result<(), AppError> {
        if self.committed { return Ok(()); }
        if self.installed && self.destination.exists() {
            std::fs::remove_dir_all(&self.destination)?;
        }
        self.installed = false;
        if self.previous { std::fs::rename(&self.backup, &self.destination)?; }
        self.committed = true;
        Ok(())
    }
}

impl Drop for InstallSwap {
    fn drop(&mut self) {
        if let Err(error) = self.rollback() {
            tracing::error!(destination = %self.destination.display(), "Plugin file rollback failed: {error}");
        }
    }
}

/// Open `path` and take an exclusive advisory lock on it (blocking, but run
/// on a blocking thread so the async worker isn't stalled). The lock lives as
/// long as the returned `File`.
async fn acquire_lock(path: &Path) -> Result<std::fs::File, AppError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(path)?;
    let file = tokio::task::spawn_blocking(move || -> Result<std::fs::File, AppError> {
        file.lock().map_err(|e| {
            AppError::Internal(format!("failed to acquire the plugin install lock: {e}"))
        })?;
        Ok(file)
    })
    .await
    .map_err(|e| AppError::Internal(format!("install lock task failed: {e}")))??;
    Ok(file)
}

/// Hard cap on the total bytes an archive may write to disk, and on its entry
/// count. The upload is limited to 64 MiB, but a compression bomb expands far
/// past that; without a cap a 1 KiB archive could fill the disk.
const MAX_EXTRACTED_BYTES: u64 = 512 * 1024 * 1024;
const MAX_ENTRIES: usize = 10_000;

/// A `Read` adapter that fails once more than `max` bytes have been produced.
/// Used to bound a decompressed tar stream, where entries cannot be inspected
/// before they are read.
struct CapReader<R> {
    inner: R,
    read: u64,
    max: u64,
}

impl<R: std::io::Read> std::io::Read for CapReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.read += n as u64;
        if self.read > self.max {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "archive expands beyond the extraction limit",
            ));
        }
        Ok(n)
    }
}

fn extract(bytes: &[u8], format: ArchiveFormat, dest: &Path) -> Result<(), AppError> {
    match format {
        ArchiveFormat::Zip => {
            let cursor = std::io::Cursor::new(bytes.to_vec());
            let mut archive = zip::ZipArchive::new(cursor)
                .map_err(|e| AppError::BadRequest(format!("Invalid zip: {}", e)))?;
            if archive.len() > MAX_ENTRIES {
                return Err(AppError::BadRequest("archive has too many entries".into()));
            }
            let mut total: u64 = 0;
            for i in 0..archive.len() {
                let mut entry = archive.by_index(i)
                    .map_err(|e| AppError::Internal(format!("Zip read error: {}", e)))?;
                let entry_name = entry.enclosed_name().ok_or_else(|| {
                    AppError::BadRequest("Zip contains unsafe path".into())
                })?.to_owned();
                let out_path = dest.join(&entry_name);
                if entry.is_dir() {
                    std::fs::create_dir_all(&out_path)?;
                } else {
                    if let Some(parent) = out_path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    let mut out = std::fs::File::create(&out_path)?;
                    // Bound this entry by the remaining budget, +1 so crossing
                    // the limit is detected rather than truncated silently.
                    let remaining = MAX_EXTRACTED_BYTES.saturating_sub(total);
                    let copied = std::io::copy(&mut entry.by_ref().take(remaining + 1), &mut out)?;
                    total += copied;
                    if total > MAX_EXTRACTED_BYTES {
                        return Err(AppError::BadRequest(
                            "archive expands beyond the extraction limit".into(),
                        ));
                    }
                }
            }
        }
        ArchiveFormat::TarGz => {
            let gz = flate2::read::GzDecoder::new(bytes);
            let capped = CapReader { inner: gz, read: 0, max: MAX_EXTRACTED_BYTES };
            let mut archive = tar::Archive::new(capped);
            archive.unpack(dest)
                .map_err(|e| AppError::BadRequest(format!("Tar unpack error: {e}")))?;
        }
        ArchiveFormat::Tar => {
            let capped = CapReader { inner: bytes, read: 0, max: MAX_EXTRACTED_BYTES };
            let mut archive = tar::Archive::new(capped);
            archive.unpack(dest)
                .map_err(|e| AppError::BadRequest(format!("Tar unpack error: {e}")))?;
        }
    }
    Ok(())
}

fn find_install_root(staging: &Path) -> PathBuf {
    // If staging has exactly one subdirectory, descend into it (WP-style
    // archives often wrap everything in a single top-level folder name).
    if let Ok(entries) = std::fs::read_dir(staging) {
        let entries: Vec<_> = entries.flatten().collect();
        if entries.len() == 1 && entries[0].path().is_dir() {
            let p = entries[0].path();
            if p.join("plugin.toml").exists() {
                return p;
            }
        }
    }
    staging.to_path_buf()
}

fn unique_staging_dir(plugins_dir: &Path) -> PathBuf {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    plugins_dir.join(format!("_staging-{}-{}", pid, ts))
}
#[cfg(test)]
mod swap_tests {
    use super::*;
    #[test]
    fn dropped_upgrade_restores_previous_files() {
        let root = tempfile::tempdir().unwrap();
        let staged = root.path().join("staged");
        let destination = root.path().join("plugin");
        let backup = root.path().join("plugin.bak");
        for path in [&staged, &destination] { std::fs::create_dir(path).unwrap(); }
        std::fs::write(destination.join("version"), "old").unwrap();
        std::fs::write(staged.join("version"), "new").unwrap();
        {
            let _swap = InstallSwap::prepare(&staged, &destination, &backup).unwrap();
            assert_eq!(std::fs::read_to_string(destination.join("version")).unwrap(), "new");
        }
        assert_eq!(std::fs::read_to_string(destination.join("version")).unwrap(), "old");
    }
    #[test]
    fn failed_move_restores_previous_files_and_does_not_use_a_stale_backup() {
        let root = tempfile::tempdir().unwrap();
        let destination = root.path().join("plugin");
        let backup = root.path().join("plugin.bak");
        std::fs::create_dir(&destination).unwrap();
        std::fs::write(destination.join("version"), "old").unwrap();
        assert!(InstallSwap::prepare(&root.path().join("missing"), &destination, &backup).is_err());
        assert_eq!(std::fs::read_to_string(destination.join("version")).unwrap(), "old");
        // A first install may have an unrelated historical backup. It must not
        // silently activate that archive on failure.
        std::fs::rename(&destination, &backup).unwrap();
        assert!(InstallSwap::prepare(&root.path().join("missing"), &destination, &backup).is_err());
        assert!(!destination.exists());
        assert!(backup.exists());
    }
}
