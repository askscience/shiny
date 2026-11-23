//! Shared, serializable data shapes exchanged with the agent and the window.

use serde::{Deserialize, Serialize};

/// One package with a pending upgrade.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageUpdate {
    pub name: String,
    /// Installed version, when the package manager reports it.
    pub current: Option<String>,
    /// Version that would be installed.
    pub candidate: Option<String>,
    /// Repository / origin, when known.
    pub repo: Option<String>,
}

/// The detected distribution and the package manager selected for it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DistroInfo {
    /// `/etc/os-release` `ID` (e.g. `debian`, `fedora`).
    pub id: String,
    /// `/etc/os-release` `NAME` (e.g. `Debian GNU/Linux`).
    pub name: String,
    pub version: Option<String>,
    /// Raw `ID_LIKE` tokens (e.g. `["debian"]`).
    pub id_like: Vec<String>,
    /// Package-manager id (`apt`, `dnf`, `pacman`, `zypper`, `apk`, `xbps`,
    /// `emerge`, `nix`, `unknown`).
    pub manager: String,
    /// Human label (e.g. `APT (Debian/Ubuntu)`).
    pub manager_label: String,
    /// False when no dedicated module matches and no known binary is present.
    pub supported: bool,
}

/// Everything the status endpoint/tool returns.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateStatus {
    pub distro: DistroInfo,
    pub packages: Vec<PackageUpdate>,
    pub count: usize,
    /// Unix seconds when the list was collected.
    pub checked_at: i64,
    /// True when the cached answer is older than the freshness window
    /// (5 minutes); a background refresh is then already under way.
    pub stale: bool,
    /// True when the answer came from the in-memory cache rather than a
    /// query run for this call.
    #[serde(default)]
    pub cached: bool,
    /// Set when the query failed (unsupported distro, timeout, …).
    pub error: Option<String>,
    pub ollama: OllamaStatus,
    /// Bytes each package would download (package name → bytes), when the
    /// package manager can tell before downloading (apt's `--print-uris`).
    #[serde(default)]
    pub sizes: std::collections::BTreeMap<String, u64>,
    /// Everything the upgrade would download, new dependencies included.
    #[serde(default)]
    pub download_bytes: Option<u64>,
}

/// One file an upgrade downloads, known before it starts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DownloadItem {
    /// The package it belongs to (`bash`).
    pub name: String,
    /// The file the package manager stores it as (`bash_5.2_amd64.deb`).
    pub file: String,
    pub size: u64,
}

/// Ollama install/update state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaStatus {
    pub installed: bool,
    pub path: Option<String>,
    pub version: Option<String>,
    /// systemd unit that would be restarted after an update, if any.
    pub service: Option<String>,
    /// The exact command an update would run (shown to the user).
    pub update_command: String,
    pub can_update: bool,
}
