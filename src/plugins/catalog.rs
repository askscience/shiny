//! The LiteAura plugin catalogue: process plugins published as archives.
//!
//! Every release publishes one `.tar.gz` per process plugin and a catalogue,
//! `index.json`, signed with the LiteAura archive key (`index.json.sig`, the
//! key APT already trusts). The Plugin manager lists what the catalogue offers,
//! installs and updates from it, and removes with the ordinary uninstall.
//!
//! Trust runs through the signature: the index is checked with `gpgv` against
//! the archive keyring before anything in it is used, and each archive must
//! match the SHA-256 the signed index gives for it. Only plugins whose
//! protocol range overlaps the server's and whose architecture matches are
//! offered, so a server is never handed a plugin it cannot talk to.
//!
//! In the background the catalogue also keeps the user's plugins in step:
//! plugins the system lists as defaults (`/usr/share/shiny/default-plugins`)
//! are installed once, and installed catalogue plugins are updated when the
//! catalogue has a newer version. A default the user removed is not brought
//! back.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::{Extension, Json};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::Digest;
use shiny_plugin_protocol as proto;
use shiny_plugin_sdk::errors::AppError;

use crate::api::AppState;
use crate::models::Traveler;
use crate::plugins::installer::log_event;

/// Where releases publish the catalogue (GitHub Pages, next to the APT repo).
pub const DEFAULT_INDEX_URL: &str = "https://askscience.github.io/shiny/plugins/index.json";
/// The archive key, as installed by `liteaura-archive-keyring`.
pub const DEFAULT_KEYRING: &str = "/usr/share/keyrings/liteaura-archive.gpg";
/// Plugins a new account gets, one name per line.
pub const DEFAULT_PLUGINS_FILE: &str = "/usr/share/shiny/default-plugins";

const ALLOWED_HOSTS: [&str; 4] = [
    "askscience.github.io",
    "github.com",
    "objects.githubusercontent.com",
    "release-assets.githubusercontent.com",
];
const INDEX_TTL: Duration = Duration::from_secs(6 * 3600);
const RETRY_TTL: Duration = Duration::from_secs(120);
const MAX_INDEX_BYTES: usize = 1024 * 1024;
const MAX_ARCHIVE_BYTES: u64 = 512 * 1024 * 1024;
/// Remembers which defaults were installed once (in the plugins directory).
const SEEDED_FILE: &str = ".catalog-seeded.json";

/// Where the catalogue comes from and how it is checked.
#[derive(Debug, Clone)]
pub struct Settings {
    pub index_url: String,
    /// The keyring `gpgv` checks the index signature against.
    pub keyring: PathBuf,
    /// Development and tests only: accept an unsigned index.
    pub allow_unsigned: bool,
    /// Development and tests only: accept `file://` URLs and any host.
    pub allow_any_host: bool,
    pub defaults_file: PathBuf,
    /// Install defaults and updates in the background
    /// (`SHINY_PLUGIN_AUTO_UPDATE=0` turns it off, e.g. on a developer's
    /// machine running its own builds).
    pub auto_maintain: bool,
}

impl Settings {
    /// Production defaults, with `SHINY_PLUGIN_INDEX_URL`,
    /// `SHINY_PLUGIN_KEYRING` and `SHINY_DEFAULT_PLUGINS_FILE` overrides. A
    /// local index (`file://`) is a developer's own build and needs no key.
    pub fn from_env() -> Self {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        let index_url = var("SHINY_PLUGIN_INDEX_URL").unwrap_or_else(|| DEFAULT_INDEX_URL.into());
        let local = index_url.starts_with("file://");
        Self {
            keyring: var("SHINY_PLUGIN_KEYRING")
                .map(PathBuf::from)
                .unwrap_or_else(|| DEFAULT_KEYRING.into()),
            allow_unsigned: local,
            allow_any_host: local,
            defaults_file: var("SHINY_DEFAULT_PLUGINS_FILE")
                .map(PathBuf::from)
                .unwrap_or_else(|| DEFAULT_PLUGINS_FILE.into()),
            auto_maintain: !matches!(
                var("SHINY_PLUGIN_AUTO_UPDATE").as_deref(),
                Some("0" | "false" | "off")
            ),
            index_url,
        }
    }
}

/// `index.json` as `scripts/build-plugin-archives.sh` writes it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Index {
    #[serde(default)]
    pub format: u32,
    #[serde(default)]
    pub release: String,
    #[serde(default)]
    pub plugins: Vec<Entry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub kind: String,
    pub arch: String,
    pub protocol: proto::VersionRange,
    pub url: String,
    pub size: u64,
    pub sha256: String,
}

impl Entry {
    /// Whether this server can run it.
    fn compatible(&self) -> bool {
        self.arch == host_arch()
            && proto::negotiate(self.protocol, proto::VersionRange::current()).is_some()
    }
}

/// The architecture names the archives use (Debian's).
pub fn host_arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        other => other,
    }
}

fn newer(candidate: &str, installed: &str) -> bool {
    match (
        semver::Version::parse(candidate),
        semver::Version::parse(installed),
    ) {
        (Ok(c), Ok(i)) => c > i,
        _ => false,
    }
}

#[derive(Debug, Clone, Default, Serialize)]
struct Status {
    /// `downloading`, `installing` or `failed`.
    state: String,
    progress: u8,
    error: Option<String>,
}

struct Cached {
    result: Result<Index, String>,
    fetched: Instant,
}

impl Cached {
    fn fresh(&self) -> bool {
        let ttl = if self.result.is_ok() {
            INDEX_TTL
        } else {
            RETRY_TTL
        };
        self.fetched.elapsed() < ttl
    }
}

/// The catalogue, shared through `AppState`.
#[derive(Clone)]
pub struct Catalog {
    inner: Arc<Inner>,
}

struct Inner {
    settings: Settings,
    client: reqwest::Client,
    cached: parking_lot::Mutex<Option<Cached>>,
    refreshing: AtomicBool,
    status: parking_lot::Mutex<HashMap<String, Status>>,
}

impl Catalog {
    pub fn new(settings: Settings) -> Self {
        let client = reqwest::Client::builder()
            .user_agent(concat!(
                "Shiny/",
                env!("CARGO_PKG_VERSION"),
                " (plugin catalogue)"
            ))
            .timeout(Duration::from_secs(600))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            inner: Arc::new(Inner {
                settings,
                client,
                cached: parking_lot::Mutex::new(None),
                refreshing: AtomicBool::new(false),
                status: parking_lot::Mutex::new(HashMap::new()),
            }),
        }
    }

    pub fn settings(&self) -> &Settings {
        &self.inner.settings
    }

    fn host_allowed(&self, url: &str) -> bool {
        if self.inner.settings.allow_any_host {
            return true;
        }
        let Some(rest) = url.strip_prefix("https://") else {
            return false;
        };
        let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
        let host = authority.rsplit('@').next().unwrap_or(authority);
        let host = host.split(':').next().unwrap_or(host).to_ascii_lowercase();
        ALLOWED_HOSTS.contains(&host.as_str())
    }

    /// Bytes behind a URL: `https://` from an allowed host, or a local
    /// `file://` in development.
    async fn fetch(&self, url: &str, max: u64, progress: Option<&str>) -> Result<Vec<u8>, String> {
        if !self.host_allowed(url) {
            return Err(format!(
                "refusing to download {url}: not a LiteAura release host"
            ));
        }
        if let Some(path) = url.strip_prefix("file://") {
            let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
            if bytes.len() as u64 > max {
                return Err(format!("{path} is larger than {max} bytes"));
            }
            return Ok(bytes);
        }
        let response = self
            .inner
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !response.status().is_success() {
            return Err(format!("{url} answered {}", response.status()));
        }
        let total = response.content_length().unwrap_or(0);
        if total > max {
            return Err(format!("{url} is larger than {max} bytes"));
        }
        let mut bytes = Vec::with_capacity(total as usize);
        let mut stream = response.bytes_stream();
        while let Some(chunk) = futures::StreamExt::next(&mut stream).await {
            let chunk = chunk.map_err(|e| e.to_string())?;
            bytes.extend_from_slice(&chunk);
            if bytes.len() as u64 > max {
                return Err(format!("{url} grew past {max} bytes"));
            }
            if let (Some(name), true) = (progress, total > 0) {
                let pct = ((bytes.len() as u64 * 100) / total).min(99) as u8;
                self.set_status(name, "downloading", pct, None);
            }
        }
        Ok(bytes)
    }

    /// Fetch the index and check its signature.
    async fn load_index(&self) -> Result<Index, String> {
        let settings = &self.inner.settings;
        let body = self
            .fetch(&settings.index_url, MAX_INDEX_BYTES as u64, None)
            .await?;
        match self
            .fetch(&format!("{}.sig", settings.index_url), 64 * 1024, None)
            .await
        {
            Ok(signature) => verify_signature(&settings.keyring, &body, &signature).await?,
            Err(error) if !settings.allow_unsigned => {
                return Err(format!("the catalogue signature is missing: {error}"))
            }
            Err(_) => {}
        }
        serde_json::from_slice(&body).map_err(|e| format!("unreadable catalogue: {e}"))
    }

    /// The cached index, refreshed now when stale (or forced).
    pub async fn index(&self, force: bool) -> Result<Index, String> {
        if !force {
            if let Some(cached) = self.inner.cached.lock().as_ref().filter(|c| c.fresh()) {
                return cached.result.clone();
            }
        }
        let result = self.load_index().await;
        if let Err(error) = &result {
            tracing::warn!("plugin catalogue: {error}");
        }
        *self.inner.cached.lock() = Some(Cached {
            result: result.clone(),
            fetched: Instant::now(),
        });
        result
    }

    fn spawn_refresh(&self, force: bool) {
        if self.inner.refreshing.swap(true, Ordering::SeqCst) {
            return;
        }
        let catalog = self.clone();
        tokio::spawn(async move {
            let _ = catalog.index(force).await;
            catalog.inner.refreshing.store(false, Ordering::SeqCst);
        });
    }

    fn set_status(&self, name: &str, state: &str, progress: u8, error: Option<String>) {
        self.inner.status.lock().insert(
            name.to_string(),
            Status {
                state: state.into(),
                progress,
                error,
            },
        );
    }

    fn busy(&self, name: &str) -> bool {
        self.inner
            .status
            .lock()
            .get(name)
            .is_some_and(|s| s.state == "downloading" || s.state == "installing")
    }

    /// What the Plugin manager shows. Never waits on the network: a stale or
    /// missing index is refreshed in the background (the window polls).
    pub fn snapshot(&self, state: &AppState) -> serde_json::Value {
        let cached = self
            .inner
            .cached
            .lock()
            .as_ref()
            .map(|c| (c.result.clone(), c.fresh()));
        if !matches!(cached, Some((_, true))) {
            self.spawn_refresh(false);
        }
        let installed: HashMap<String, String> = state
            .plugins
            .list()
            .into_iter()
            .map(|m| (m.name, m.version.to_string()))
            .collect();
        let status = self.inner.status.lock().clone();
        let (index, error) = match cached.map(|(r, _)| r) {
            Some(Ok(index)) => (Some(index), None),
            Some(Err(error)) => (None, Some(error)),
            None => (None, None),
        };
        let plugins: Vec<serde_json::Value> = index
            .as_ref()
            .map(|index| {
                index
                    .plugins
                    .iter()
                    .filter(|e| e.compatible())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
            .into_iter()
            .map(|entry| {
                let installed_version = installed.get(&entry.name).cloned();
                let update_available = installed_version
                    .as_deref()
                    .is_some_and(|v| newer(&entry.version, v));
                let system = state.plugins.is_system_only(&entry.name);
                let s = status.get(&entry.name).cloned().unwrap_or_default();
                json!({
                    "name": entry.name,
                    "version": entry.version,
                    "description": entry.description,
                    "summary": entry.summary,
                    "category": entry.category,
                    "size": entry.size,
                    "installed_version": installed_version,
                    "update_available": update_available,
                    "system": system,
                    "state": if s.state.is_empty() { "idle".to_string() } else { s.state },
                    "progress": s.progress,
                    "error": s.error,
                })
            })
            .collect();
        json!({
            "refreshing": self.inner.refreshing.load(Ordering::SeqCst),
            "release": index.as_ref().map(|i| i.release.clone()),
            // No network is a wait; anything else (not published yet, a bad
            // signature) is the catalogue being unavailable.
            "offline": error.as_deref().is_some_and(network_error),
            "error": error,
            "plugins": plugins,
        })
    }

    /// Start installing (or updating) one plugin; the snapshot follows it.
    pub async fn start_install(&self, state: &AppState, name: &str) -> Result<(), AppError> {
        if self.busy(name) {
            return Err(AppError::BadRequest(format!(
                "{name} is already being installed"
            )));
        }
        let index = self.index(false).await.map_err(AppError::Internal)?;
        let entry = index
            .plugins
            .into_iter()
            .find(|e| e.name == name && e.compatible())
            .ok_or_else(|| {
                AppError::NotFound(format!("the catalogue offers no '{name}' for this system"))
            })?;
        self.set_status(name, "downloading", 0, None);
        let catalog = self.clone();
        let state = state.clone();
        tokio::spawn(async move {
            if let Err(error) = catalog.install(&state, &entry).await {
                let plugins_dir = PathBuf::from(&state.config.plugins_dir);
                log_event(
                    &plugins_dir,
                    &format!(
                        "catalog-install-failed name={} version={} {error}",
                        entry.name, entry.version
                    ),
                );
                catalog.set_status(&entry.name, "failed", 0, Some(error.to_string()));
            }
        });
        Ok(())
    }

    /// Download, verify and install one entry. Returns the installed name.
    pub async fn install(&self, state: &AppState, entry: &Entry) -> Result<String, AppError> {
        let plugins_dir = PathBuf::from(&state.config.plugins_dir);
        log_event(
            &plugins_dir,
            &format!(
                "catalog-download-begin name={} version={} bytes={}",
                entry.name, entry.version, entry.size
            ),
        );
        self.set_status(&entry.name, "downloading", 0, None);
        let bytes = self
            .fetch(&entry.url, MAX_ARCHIVE_BYTES, Some(&entry.name))
            .await
            .map_err(|e| AppError::Internal(format!("download failed: {e}")))?;
        let actual = hex::encode(sha2::Sha256::digest(&bytes));
        if !actual.eq_ignore_ascii_case(&entry.sha256) {
            return Err(AppError::BadRequest(format!(
                "checksum mismatch for {}: the catalogue says {}, the download is {actual}",
                entry.name, entry.sha256
            )));
        }
        log_event(
            &plugins_dir,
            &format!("catalog-verify-ok name={} sha256={actual}", entry.name),
        );
        self.set_status(&entry.name, "installing", 100, None);
        let installed = crate::plugins::installer::install_archive(
            bytes,
            &plugins_dir,
            &state.plugins,
            state.plugin_ctx(),
        )
        .await?;
        if installed != entry.name {
            log_event(
                &plugins_dir,
                &format!(
                    "catalog-name-mismatch expected={} installed={installed}",
                    entry.name
                ),
            );
        }
        if let Some(rebuild) = &state.router_rebuild {
            rebuild();
        }
        log_event(
            &plugins_dir,
            &format!(
                "catalog-install-ok name={installed} version={}",
                entry.version
            ),
        );
        self.inner.status.lock().remove(&entry.name);
        Ok(installed)
    }

    /// Install missing defaults once and update what is behind. Returns what
    /// it installed or updated.
    pub async fn maintain(&self, state: &AppState) -> Result<Vec<String>, String> {
        let index = self.index(false).await?;
        let plugins_dir = PathBuf::from(&state.config.plugins_dir);
        let mut seeded = read_seeded(&plugins_dir);
        let defaults = read_defaults(&self.inner.settings.defaults_file);
        let installed: HashMap<String, String> = state
            .plugins
            .list()
            .into_iter()
            .map(|m| (m.name, m.version.to_string()))
            .collect();
        let mut done = Vec::new();
        for entry in index.plugins.iter().filter(|e| e.compatible()) {
            if self.busy(&entry.name) {
                continue;
            }
            let wanted = match installed.get(&entry.name) {
                // Installed: keep it current (a system copy is the package's
                // business, but a newer catalogue build may override it).
                Some(version) => newer(&entry.version, version),
                // A default this account never had: once.
                None => defaults.contains(&entry.name) && !seeded.contains(&entry.name),
            };
            if !wanted {
                continue;
            }
            match self.install(state, entry).await {
                Ok(name) => {
                    seeded.insert(name.clone());
                    done.push(name);
                }
                Err(error) => {
                    log_event(
                        &plugins_dir,
                        &format!("catalog-maintain-failed name={} {error}", entry.name),
                    );
                    self.set_status(&entry.name, "failed", 0, Some(error.to_string()));
                }
            }
        }
        // Defaults already present (a live system, an upgrade) count as had.
        for name in &defaults {
            if installed.contains_key(name) {
                seeded.insert(name.clone());
            }
        }
        write_seeded(&plugins_dir, &seeded);
        Ok(done)
    }

    /// Run [`Catalog::maintain`] shortly after start-up and then periodically.
    pub fn spawn_maintenance(&self, state: AppState) {
        if !self.inner.settings.auto_maintain {
            return;
        }
        let catalog = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(20)).await;
            loop {
                let wait = match catalog.maintain(&state).await {
                    Ok(done) => {
                        if !done.is_empty() {
                            tracing::info!(
                                "plugin catalogue installed or updated: {}",
                                done.join(", ")
                            );
                        }
                        INDEX_TTL
                    }
                    Err(error) => {
                        tracing::info!("plugin catalogue not reachable yet: {error}");
                        RETRY_TTL
                    }
                };
                tokio::time::sleep(wait).await;
            }
        });
    }
}

/// Whether a fetch failed for want of a network rather than because of the
/// answer (a 404, a bad signature).
fn network_error(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    [
        "error sending request",
        "dns error",
        "failed to lookup address",
        "connection refused",
        "connection reset",
        "network is unreachable",
        "no route to host",
        "temporary failure",
        "timed out",
    ]
    .iter()
    .any(|needle| error.contains(needle))
}

/// Check a detached OpenPGP signature with `gpgv` (part of every Debian
/// system: APT uses it for the same job).
async fn verify_signature(keyring: &Path, data: &[u8], signature: &[u8]) -> Result<(), String> {
    if !keyring.is_file() {
        return Err(format!("no archive keyring at {}", keyring.display()));
    }
    let dir = std::env::temp_dir().join(format!("shiny-catalog-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let data_path = dir.join("index.json");
    let sig_path = dir.join("index.json.sig");
    let written =
        std::fs::write(&data_path, data).and_then(|_| std::fs::write(&sig_path, signature));
    let output = match written {
        Ok(()) => tokio::process::Command::new("gpgv")
            .arg("--keyring")
            .arg(keyring)
            .arg(&sig_path)
            .arg(&data_path)
            .output()
            .await
            .map_err(|e| format!("gpgv: {e}")),
        Err(e) => Err(e.to_string()),
    };
    let _ = std::fs::remove_dir_all(&dir);
    let output = output?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "the catalogue signature does not verify: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn read_defaults(path: &Path) -> BTreeSet<String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|l| l.split('#').next().unwrap_or("").trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

fn read_seeded(plugins_dir: &Path) -> BTreeSet<String> {
    std::fs::read(plugins_dir.join(SEEDED_FILE))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn write_seeded(plugins_dir: &Path, seeded: &BTreeSet<String>) {
    if let Ok(json) = serde_json::to_vec_pretty(seeded) {
        let tmp = plugins_dir.join(format!("{SEEDED_FILE}.new"));
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(tmp, plugins_dir.join(SEEDED_FILE));
        }
    }
}

// ── HTTP ────────────────────────────────────────────────────────────────────

/// GET /api/plugins/catalog — the catalogue with each plugin's state here.
pub async fn catalog_list(
    State(state): State<AppState>,
    Extension(_traveler): Extension<Traveler>,
) -> Result<Json<serde_json::Value>, AppError> {
    Ok(Json(
        json!({ "success": true, "data": state.catalog.snapshot(&state) }),
    ))
}

/// POST /api/plugins/catalog/refresh — look at the catalogue again now.
pub async fn catalog_refresh(
    State(state): State<AppState>,
    Extension(_traveler): Extension<Traveler>,
) -> Result<Json<serde_json::Value>, AppError> {
    state
        .catalog
        .index(true)
        .await
        .map_err(AppError::Internal)?;
    Ok(Json(
        json!({ "success": true, "data": state.catalog.snapshot(&state) }),
    ))
}

/// POST /api/plugins/catalog/install  body {"name":"word"} — install or update.
pub async fn catalog_install(
    State(state): State<AppState>,
    Extension(_traveler): Extension<Traveler>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, AppError> {
    let name = body
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::BadRequest("name required".into()))?;
    crate::plugins::loader::validate_plugin_name(name)?;
    state.catalog.start_install(&state, name).await?;
    Ok(Json(
        json!({ "success": true, "data": { "installing": name } }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> Catalog {
        Catalog::new(Settings {
            index_url: DEFAULT_INDEX_URL.into(),
            keyring: DEFAULT_KEYRING.into(),
            allow_unsigned: false,
            allow_any_host: false,
            defaults_file: DEFAULT_PLUGINS_FILE.into(),
            auto_maintain: false,
        })
    }

    #[test]
    fn only_release_hosts_over_https_are_allowed() {
        let c = catalog();
        assert!(
            c.host_allowed("https://github.com/askscience/shiny/releases/download/v1/word.tar.gz")
        );
        assert!(c.host_allowed("https://askscience.github.io/shiny/plugins/index.json"));
        assert!(!c.host_allowed("http://github.com/x"));
        assert!(!c.host_allowed("https://github.com.evil.example/x"));
        assert!(!c.host_allowed("https://evil.example@github.com.evil.example/x"));
        assert!(!c.host_allowed("file:///etc/passwd"));
    }

    #[test]
    fn compatibility_needs_the_arch_and_a_shared_protocol() {
        let entry = |arch: &str, min, max| Entry {
            name: "x".into(),
            version: "1.0.0".into(),
            description: String::new(),
            summary: String::new(),
            category: String::new(),
            kind: String::new(),
            arch: arch.into(),
            protocol: proto::VersionRange { min, max },
            url: String::new(),
            size: 0,
            sha256: String::new(),
        };
        assert!(entry(host_arch(), proto::PROTOCOL_MIN, proto::PROTOCOL_MAX).compatible());
        assert!(!entry("sparc", proto::PROTOCOL_MIN, proto::PROTOCOL_MAX).compatible());
        assert!(!entry(
            host_arch(),
            proto::PROTOCOL_MAX + 1,
            proto::PROTOCOL_MAX + 2
        )
        .compatible());
    }

    #[test]
    fn a_missing_catalogue_is_not_a_missing_network() {
        assert!(network_error("error sending request for url (https://x): dns error"));
        assert!(network_error("operation timed out"));
        assert!(!network_error("https://askscience.github.io/shiny/plugins/index.json answered 404 Not Found"));
        assert!(!network_error("the catalogue signature does not verify: BAD signature"));
    }

    #[test]
    fn versions_compare_as_semver() {
        assert!(newer("0.1.10", "0.1.9"));
        assert!(!newer("0.1.2", "0.1.2"));
        assert!(!newer("garbage", "0.1.0"));
    }
}
