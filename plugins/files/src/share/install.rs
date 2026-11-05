//! Locating and installing the `gokapi` / `gokapi-cli` binaries.
//!
//! Lookup order:
//! 1. the `GOKAPI_BIN` / `GOKAPI_CLI_BIN` overrides;
//! 2. the copies bundled in the liteaura package, [`BUNDLED_DIR`] (overridable
//!    with `SHINY_GOKAPI_BUNDLE_DIR`; see `packaging/gokapi.sh`);
//! 3. copies downloaded into [`super::bin_dir`] (development checkouts with no
//!    package);
//! 4. `PATH`.
//!
//! The pinned copies win over `PATH` because the headless bootstrap depends on
//! Gokapi's internal formats for the exact release in [`super::GOKAPI_VERSION`].
//!
//! The download fallback fetches the release zips, checks their SHA-256 against
//! the constants in `share/mod.rs` and extracts the single binary in each. It
//! runs in a spawned task (see the runtime note in `share/mod.rs`); Settings
//! polls [`state_json`] for progress.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{json, Value as Json};
use sha2::{Digest, Sha256};

use super::{asset_url, assets_for_arch, bin_dir, Asset};

pub const SERVER_NAME: &str = "gokapi";
pub const CLI_NAME: &str = "gokapi-cli";

/// Where `scripts/build-deb.sh` installs the bundled binaries.
pub const BUNDLED_DIR: &str = "/usr/libexec/shiny/gokapi";

fn bundled_dir() -> PathBuf {
    std::env::var_os("SHINY_GOKAPI_BUNDLE_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(BUNDLED_DIR))
}

/// Resolve a binary: env override, bundled, downloaded, then `PATH`.
fn locate(env_key: &str, name: &str) -> Option<PathBuf> {
    if let Some(v) = std::env::var_os(env_key).filter(|v| !v.is_empty()) {
        let p = PathBuf::from(v);
        return p.is_file().then_some(p);
    }
    [bundled_dir().join(name), bin_dir().join(name)]
        .into_iter()
        .find(|p| p.is_file())
        .or_else(|| which(name))
}

/// True when the server binary in use is the one shipped with the package.
pub fn is_bundled() -> bool {
    server_bin().is_some_and(|p| p.starts_with(bundled_dir()))
}

/// Search `PATH` for a bare binary name (same approach as the updates
/// plugin's `exec::which`).
pub fn which(name: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .map(|dir| dir.join(name))
        .find(|cand| cand.is_file())
}

pub fn server_bin() -> Option<PathBuf> {
    locate("GOKAPI_BIN", SERVER_NAME)
}

pub fn cli_bin() -> Option<PathBuf> {
    locate("GOKAPI_CLI_BIN", CLI_NAME)
}

/* ── background install ───────────────────────────────────── */

#[derive(Clone, Debug, PartialEq)]
enum InstallState {
    Idle,
    Running(String),
    Done,
    Failed(String),
}

static STATE: Mutex<InstallState> = Mutex::new(InstallState::Idle);

fn set_state(s: InstallState) {
    *STATE.lock().unwrap_or_else(|e| e.into_inner()) = s;
}

fn get_state() -> InstallState {
    STATE.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// `{ "state": "idle|running|done|failed", "step"?: …, "error"?: … }`
pub fn state_json() -> Json {
    match get_state() {
        InstallState::Idle => json!({ "state": "idle" }),
        InstallState::Running(step) => json!({ "state": "running", "step": step }),
        InstallState::Done => json!({ "state": "done" }),
        InstallState::Failed(e) => json!({ "state": "failed", "error": e }),
    }
}

/// Start the install in the background. Must be called from the plugin
/// runtime (a bridged route). Returns `false` when one is already running.
pub fn start() -> Result<bool, String> {
    let Some((srv, cli)) = assets_for_arch() else {
        return Err(format!(
            "no Gokapi build is installed automatically for {}; set GOKAPI_BIN and GOKAPI_CLI_BIN",
            std::env::consts::ARCH
        ));
    };
    {
        let mut st = STATE.lock().unwrap_or_else(|e| e.into_inner());
        if matches!(*st, InstallState::Running(_)) {
            return Ok(false);
        }
        *st = InstallState::Running("starting".into());
    }
    tokio::spawn(async move {
        let result = async {
            install_one(srv, SERVER_NAME).await?;
            install_one(cli, CLI_NAME).await
        }
        .await;
        set_state(match result {
            Ok(()) => InstallState::Done,
            Err(e) => InstallState::Failed(e),
        });
    });
    Ok(true)
}

async fn install_one(asset: Asset, bin_name: &'static str) -> Result<(), String> {
    set_state(InstallState::Running(format!("downloading {}", asset.name)));
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(600))
        .user_agent("shiny-files")
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(asset_url(&asset))
        .send()
        .await
        .map_err(|e| format!("download {}: {e}", asset.name))?;
    if !resp.status().is_success() {
        return Err(format!("download {}: HTTP {}", asset.name, resp.status()));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("download {}: {e}", asset.name))?
        .to_vec();

    set_state(InstallState::Running(format!("verifying {}", asset.name)));
    let dest = bin_dir().join(bin_name);
    // Hashing and inflating ~50 MB is CPU work: keep it off the plugin's
    // single runtime thread so other Files routes stay responsive.
    tokio::task::spawn_blocking(move || verify_and_extract(&bytes, &asset, bin_name, &dest))
        .await
        .map_err(|e| format!("install task failed: {e}"))?
}

/// Check the zip's SHA-256, then extract `bin_name` atomically to `dest`.
fn verify_and_extract(
    bytes: &[u8],
    asset: &Asset,
    bin_name: &str,
    dest: &Path,
) -> Result<(), String> {
    let digest = sha256_hex(bytes);
    if digest != asset.sha256 {
        return Err(format!(
            "checksum mismatch for {}: expected {}, got {digest}",
            asset.name, asset.sha256
        ));
    }
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|e| format!("{}: not a zip: {e}", asset.name))?;
    let mut entry = archive
        .by_name(bin_name)
        .map_err(|_| format!("{} does not contain {bin_name}", asset.name))?;
    let mut data = Vec::with_capacity(entry.size() as usize);
    entry
        .read_to_end(&mut data)
        .map_err(|e| format!("extract {bin_name}: {e}"))?;

    let dir = dest.parent().ok_or("invalid install path")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let tmp = dest.with_extension("partial");
    std::fs::write(&tmp, &data).map_err(|e| format!("{}: {e}", tmp.display()))?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))
        .map_err(|e| format!("chmod {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    Ok(())
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_hex_matches_known_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn checksum_mismatch_is_rejected_before_extracting() {
        let asset = Asset {
            name: "x.zip",
            sha256: "00",
        };
        let dest = std::env::temp_dir().join("shiny-gokapi-never-written");
        let err = verify_and_extract(b"not a zip", &asset, "gokapi", &dest).unwrap_err();
        assert!(err.contains("checksum mismatch"), "{err}");
        assert!(!dest.exists());
    }

    #[test]
    fn extracts_named_binary_from_zip() {
        use std::io::Write;
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut buf);
            w.start_file("gokapi", zip::write::FileOptions::default())
                .unwrap();
            w.write_all(b"#!/bin/true\n").unwrap();
            w.finish().unwrap();
        }
        let bytes = buf.into_inner();
        let sum = sha256_hex(&bytes);
        let asset = Asset {
            name: "t.zip",
            sha256: Box::leak(sum.into_boxed_str()),
        };
        let dir = std::env::temp_dir().join(format!(
            "shiny-gokapi-test-{}",
            super::super::random_alnum(8)
        ));
        let dest = dir.join("gokapi");
        verify_and_extract(&bytes, &asset, "gokapi", &dest).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"#!/bin/true\n");
        let _ = std::fs::remove_dir_all(dir);
    }
}
