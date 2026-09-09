//! Plugin installer — extracts `.zip` / `.tar.gz` archives into `data/plugins/<name>/`.
//! Used by the admin HTTP API.
//!
//! All installation attempts (success and failure) are appended to
//! `data/plugins/install.log` so admins can audit issues offline.

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
    if final_dir.exists() {
        // Back up the previous installation as `<name>.bak` so people can roll back.
        if backup.exists() {
            std::fs::remove_dir_all(&backup).ok();
        }
        std::fs::rename(&final_dir, &backup).ok();
    }
    if let Err(e) = std::fs::rename(&install_dir, &final_dir) {
        log_event(plugins_dir, &format!("rename-failed: {e}"));
        let _ = std::fs::remove_dir_all(&staging);
        // The previous install (if any) was moved to `.bak` — put it back.
        if backup.exists() {
            let _ = std::fs::rename(&backup, &final_dir);
        }
        return Err(AppError::Internal(format!("rename failed: {e}")));
    }
    // Clean staging
    let _ = std::fs::remove_dir_all(&staging);

    // Register via the manager (this dl's the cdylib + runs migrations).
    let installed_name = match manager.install_dir_static(&final_dir, base_ctx).await {
        Ok(n) => n,
        Err(e) => {
            log_event(plugins_dir, &format!("register-failed name={}: {e}", manifest.name));
            // Roll back the new install dir, then restore the `.bak` of the
            // previous working version so a failed upgrade doesn't leave the
            // plugin missing entirely.
            let _ = std::fs::remove_dir_all(&final_dir);
            if backup.exists() {
                let _ = std::fs::rename(&backup, &final_dir);
                log_event(plugins_dir, &format!("restored-backup name={}", manifest.name));
            }
            return Err(e);
        }
    };
    log_event(plugins_dir, &format!(
        "install-ok name={}", installed_name
    ));
    Ok(installed_name)
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

fn extract(bytes: &[u8], format: ArchiveFormat, dest: &Path) -> Result<(), AppError> {
    match format {
        ArchiveFormat::Zip => {
            let cursor = std::io::Cursor::new(bytes.to_vec());
            let mut archive = zip::ZipArchive::new(cursor)
                .map_err(|e| AppError::BadRequest(format!("Invalid zip: {}", e)))?;
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
                    std::io::copy(&mut entry, &mut out)?;
                }
            }
        }
        ArchiveFormat::TarGz => {
            let gz = flate2::read::GzDecoder::new(bytes);
            let mut archive = tar::Archive::new(gz);
            archive.unpack(dest)
                .map_err(|e| AppError::Internal(format!("Tar unpack error: {}", e)))?;
        }
        ArchiveFormat::Tar => {
            let mut archive = tar::Archive::new(bytes);
            archive.unpack(dest)
                .map_err(|e| AppError::Internal(format!("Tar unpack error: {}", e)))?;
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