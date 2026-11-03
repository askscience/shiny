//! Public share links backed by a Shiny-managed Gokapi server.
//!
//! Gokapi (<https://github.com/Forceu/Gokapi>, AGPL-3) is never linked: Shiny
//! runs the upstream release binaries as child processes, so nothing of its
//! code ships inside this GPL plugin. Its web UI is never used either:
//!
//! * [`install`] downloads the pinned `gokapi` + `gokapi-cli` release assets
//!   and verifies their SHA-256 against constants below.
//! * [`server`] bootstraps a headless instance (hand-written `config.json`,
//!   `--deployment-password` to create the super-admin, an API key inserted
//!   straight into Gokapi's SQLite), supervises the process and exposes it
//!   through Tailscale Funnel on :8443 or a user-supplied public URL.
//! * [`cli`] uploads through `gokapi-cli upload --json`.
//! * [`rest`] covers what the CLI cannot do: per-file metadata and delete.
//! * [`store`] records which Shiny account owns each Gokapi file id.
//!
//! **Plugin runtime rule:** every route of this plugin is serviced by one
//! serial worker (`shiny_plugin_sdk::rt`). Anything long (download, upload,
//! the server's lifetime) therefore runs in a `tokio::spawn`ed task, which the
//! worker's `current_thread` runtime polls between jobs, and routes only start
//! it and report its state.

pub mod cli;
pub mod e2e;
pub mod gate;
pub mod install;
pub mod rest;
pub mod routes;
pub mod server;
pub mod store;
pub mod theme;

use std::path::PathBuf;

/// Gokapi server release the bootstrap format below was verified against
/// (`ConfigVersion` 22, `ApiKeys` schema). Bump only after re-checking
/// `server::write_config` and `server::insert_api_key` against the new tag.
pub const GOKAPI_VERSION: &str = "2.2.4";
/// CLI release published alongside [`GOKAPI_VERSION`].
pub const GOKAPI_CLI_VERSION: &str = "1.1.3";

/// A release asset: file name on the GitHub release and its SHA-256.
pub struct Asset {
    pub name: &'static str,
    pub sha256: &'static str,
}

/// The server + CLI assets for this machine, or `None` on an architecture
/// Shiny does not install for (the user can still point `GOKAPI_BIN` /
/// `GOKAPI_CLI_BIN` at their own build).
pub fn assets_for_arch() -> Option<(Asset, Asset)> {
    match std::env::consts::ARCH {
        "x86_64" => Some((
            Asset {
                name: "gokapi-2.2.4_linux-amd64.zip",
                sha256: "51dbf724c94f8afa0b40fb1ef63a1fe418e7996e7b9536e1f0a5da695b8e18ef",
            },
            Asset {
                name: "gokapi-cli-1.1.3_linux-amd64.zip",
                sha256: "5bfa76c1dbfc9c9d4e4dcca535b7942db71672e3e06336bf6bdfebe654225f64",
            },
        )),
        "aarch64" => Some((
            Asset {
                name: "gokapi-2.2.4_linux-arm64.zip",
                sha256: "9f82cfb96fc48ac9bdfb94584dc6ba4648de1ac4fcd1276d2dd5a7e0f80398f7",
            },
            Asset {
                name: "gokapi-cli-1.1.3_linux-arm64.zip",
                sha256: "64cc187a55731bc3bde6d1a3b6c0c4517aa80437054f04e5abc3886f90344c86",
            },
        )),
        _ => None,
    }
}

/// Download URL of a release asset.
pub fn asset_url(asset: &Asset) -> String {
    format!(
        "https://github.com/Forceu/Gokapi/releases/download/v{GOKAPI_VERSION}/{}",
        asset.name
    )
}

/// Root of all Gokapi state: binaries, config, data, logs.
///
/// Host-wide (one server for every Shiny account), so it lives beside the
/// per-user homes rather than inside one. `SHINY_GOKAPI_DIR` overrides it.
pub fn state_dir() -> PathBuf {
    std::env::var_os("SHINY_GOKAPI_DIR")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| crate::fs_util::os_home().join(".shiny").join("gokapi"))
}

pub fn bin_dir() -> PathBuf {
    state_dir().join("bin")
}
pub fn config_dir() -> PathBuf {
    state_dir().join("config")
}
pub fn data_dir() -> PathBuf {
    state_dir().join("data")
}
pub fn tmp_dir() -> PathBuf {
    state_dir().join("tmp")
}
pub fn gokapi_config_path() -> PathBuf {
    config_dir().join("config.json")
}
pub fn gokapi_db_path() -> PathBuf {
    data_dir().join("gokapi.sqlite")
}
pub fn cli_config_path() -> PathBuf {
    state_dir().join("gokapi-cli.json")
}
/// Shiny's own record of the instance (port, API key, public URL).
pub fn shiny_state_path() -> PathBuf {
    state_dir().join("shiny.json")
}
pub fn log_path() -> PathBuf {
    state_dir().join("gokapi.log")
}

/// `len` random characters from `[A-Za-z0-9]`, from the OS CSPRNG.
///
/// Rejection sampling keeps the distribution uniform (248 = 4 × 62).
pub fn random_alnum(len: usize) -> String {
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut out = String::with_capacity(len);
    let mut buf = [0u8; 64];
    while out.len() < len {
        getrandom::getrandom(&mut buf).expect("OS random source unavailable");
        for b in buf {
            if out.len() == len {
                break;
            }
            if b < 248 {
                out.push(CHARSET[(b % 62) as usize] as char);
            }
        }
    }
    out
}

/// Write `contents` to `path` readable only by the owner (API keys and the
/// admin password live in these files).
pub fn write_private(path: &std::path::Path, contents: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(contents)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_alnum_has_requested_length_and_charset() {
        let s = random_alnum(35);
        assert_eq!(s.len(), 35);
        assert!(s.chars().all(|c| c.is_ascii_alphanumeric()));
        assert_ne!(random_alnum(30), random_alnum(30));
    }

    #[test]
    fn asset_url_points_at_pinned_release() {
        let a = Asset {
            name: "x.zip",
            sha256: "",
        };
        assert_eq!(
            asset_url(&a),
            "https://github.com/Forceu/Gokapi/releases/download/v2.2.4/x.zip"
        );
    }

    /// `packaging/gokapi.sh` (what the .deb bundles) and the constants here
    /// (the download fallback) must name the same release.
    #[test]
    fn pins_match_packaging_manifest() {
        let manifest = include_str!("../../../../packaging/gokapi.sh");
        let value = |key: &str| {
            manifest
                .lines()
                .find_map(|l| l.strip_prefix(&format!("{key}=")))
                .unwrap_or_else(|| panic!("{key} missing from packaging/gokapi.sh"))
                .trim()
                .to_string()
        };
        assert_eq!(value("GOKAPI_VERSION"), GOKAPI_VERSION);
        assert_eq!(value("GOKAPI_CLI_VERSION"), GOKAPI_CLI_VERSION);
        let amd64 = [
            (
                "gokapi-2.2.4_linux-amd64.zip",
                value("GOKAPI_SERVER_SHA256"),
            ),
            (
                "gokapi-cli-1.1.3_linux-amd64.zip",
                value("GOKAPI_CLI_SHA256"),
            ),
        ];
        assert_eq!(value("GOKAPI_SERVER_ZIP"), amd64[0].0);
        assert_eq!(value("GOKAPI_CLI_ZIP"), amd64[1].0);
        if std::env::consts::ARCH == "x86_64" {
            let (srv, cli) = assets_for_arch().unwrap();
            assert_eq!((srv.name, srv.sha256), (amd64[0].0, amd64[0].1.as_str()));
            assert_eq!((cli.name, cli.sha256), (amd64[1].0, amd64[1].1.as_str()));
        }
    }
}
