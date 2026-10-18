//! The Craft store — installs and updates the Craft App Suite's **web builds**
//! as `kind = "web"` plugins.
//!
//! Every app in the suite publishes a static web build as an asset of its
//! GitHub release (`<app>-web-<version>.zip`, next to the platform packages).
//! The store resolves each app's latest release, picks that asset, downloads
//! it, verifies it against the SHA-256 the release publishes, and installs it
//! through [`installer::install_web_app`]. The core then serves it at
//! `/plugins/<id>/` and the shell mounts it in a plugin window (see
//! `web/js/webAppSurface.js`).
//!
//! The catalog and the release/verification rules mirror ArtCraft Launcher
//! (<https://github.com/storytold/craft-launcher>, MIT OR Apache-2.0): its
//! `crates/core/src/catalog.rs` for the suite table and its `release.rs` for
//! picking an asset out of a release. We install the *web* asset and keep the
//! whole pipeline inside our own plugin system.

use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::Extension;
use axum::Json;
use serde::Deserialize;
use serde_json::json;
use sha2::Digest;

use crate::api::AppState;
use crate::errors::AppError;
use crate::models::Traveler;
use crate::plugins::installer::log_event;

/// How long one app's release lookup stays fresh. GitHub's unauthenticated
/// API allows 60 requests per hour per address; thirteen apps at one hour are
/// far inside that, and a cached entry is served when a refresh fails.
const RELEASE_TTL: Duration = Duration::from_secs(3600);
/// A larger download is not a Craft web build (they are 10–16 MB today).
const MAX_DOWNLOAD_BYTES: u64 = 256 * 1024 * 1024;
/// The CLI inside the Linux package (60–90 MB today); a bound on what one
/// archive entry may unpack to.
const MAX_CLI_BYTES: u64 = 256 * 1024 * 1024;
/// The app icons are small PNGs; anything else is not one.
const MAX_ICON_BYTES: usize = 512 * 1024;
/// Release assets are only ever fetched from GitHub itself, whatever a
/// (compromised or malformed) API response claims.
const ALLOWED_DOWNLOAD_HOSTS: [&str; 3] = [
    "github.com",
    "objects.githubusercontent.com",
    "release-assets.githubusercontent.com",
];
/// Icons come from the app's own repository, unmodified.
const ICON_HOST: &str = "raw.githubusercontent.com";
/// The GitHub account every Craft app lives under.
const GITHUB_OWNER: &str = "storytold";

// ── The catalog ─────────────────────────────────────────────────────────────

/// One app in the Craft App Suite.
pub struct CraftApp {
    /// Machine name: repository, release-asset stem and the plugin name the
    /// app is installed as.
    pub id: &'static str,
    /// Display name (product names stay as published, untranslated).
    pub name: &'static str,
    /// What it is for, in a few words.
    pub tagline: &'static str,
    /// Older release-asset stems (PdfCraft shipped as PrintCraft until 0.2.1).
    pub former_ids: &'static [&'static str],
}

impl CraftApp {
    /// Asset stems this app's releases may use, current first.
    fn stems(&self) -> impl Iterator<Item = &'static str> + '_ {
        std::iter::once(self.id).chain(self.former_ids.iter().copied())
    }

    /// The app's page on getartcraft.com.
    pub fn website(&self) -> String {
        format!("https://getartcraft.com/apps/{}", self.id)
    }

    pub fn by_id(id: &str) -> Option<&'static CraftApp> {
        SUITE.iter().find(|app| app.id == id)
    }
}

const fn app(id: &'static str, name: &'static str, tagline: &'static str) -> CraftApp {
    CraftApp { id, name, tagline, former_ids: &[] }
}

/// The suite, in display order (matches getartcraft.com).
pub const SUITE: [CraftApp; 13] = [
    app("photocraft", "PhotoCraft", "Image editing"),
    app("vectorcraft", "VectorCraft", "Vector illustration"),
    app("filmcraft", "FilmCraft", "Video editing, color and sound"),
    app("lightcraft", "LightCraft", "Photo library and raw development"),
    CraftApp {
        former_ids: &["printcraft"],
        ..app("pdfcraft", "PdfCraft", "Reading, organizing and protecting PDFs")
    },
    app("effectcraft", "EffectCraft", "Motion graphics and visual effects"),
    app("designcraft", "DesignCraft", "Page layout and publishing"),
    app("figcraft", "FigCraft", "Interface design and prototyping"),
    app("wordcraft", "WordCraft", "Writing and document design"),
    app("soundcraft", "SoundCraft", "Recording, editing and mixing audio"),
    app("deckcraft", "DeckCraft", "Presentations and slide shows"),
    app("cadcraft", "CADCraft", "Computer-aided design and drafting"),
    app("gridcraft", "GridCraft", "Spreadsheets and data"),
];

// ── Release resolution ──────────────────────────────────────────────────────

/// The subset of GitHub's release JSON the store reads.
#[derive(Debug, Deserialize)]
struct GitHubRelease {
    #[serde(default)]
    tag_name: String,
    #[serde(default)]
    assets: Vec<GitHubAsset>,
}

#[derive(Debug, Clone, Deserialize)]
struct GitHubAsset {
    name: String,
    #[serde(default)]
    browser_download_url: String,
    #[serde(default)]
    size: u64,
    /// `sha256:<hex>`, published by GitHub for release assets.
    #[serde(default)]
    digest: Option<String>,
}

/// One resolved app release: the web build and what we know about it.
#[derive(Debug, Clone)]
pub struct Release {
    pub version: semver::Version,
    pub asset_name: String,
    pub asset_url: String,
    pub asset_size: u64,
    /// The release's own SHA-256 for this asset, when it publishes one.
    pub sha256: Option<String>,
    /// The Linux package that carries the app's command-line tool, which is
    /// also its MCP server. `None` when this release has no build for our
    /// architecture.
    pub cli: Option<CliAsset>,
}

/// One asset we may download (`<app>-cli` comes out of the Linux tarball).
#[derive(Debug, Clone)]
pub struct CliAsset {
    pub name: String,
    pub url: String,
    pub size: u64,
    pub sha256: Option<String>,
}

/// The architecture suffix the Craft releases use, when we can run it.
fn linux_arch() -> Option<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Some("x86_64"),
        "aarch64" => Some("aarch64"),
        "riscv64" => Some("riscv64"),
        _ => None,
    }
}

/// The version a release tag carries (`v0.10.0` → `0.10.0`).
fn version_from_tag(tag: &str) -> Option<semver::Version> {
    semver::Version::parse(tag.trim().trim_start_matches('v')).ok()
}

/// The web build inside a release: `<stem>-web-<version>.zip`, where `stem` is
/// the app's current or former asset stem.
fn web_asset<'a>(app: &CraftApp, release: &'a GitHubRelease) -> Option<&'a GitHubAsset> {
    release.assets.iter().find(|asset| {
        asset.name.ends_with(".zip")
            && app.stems().any(|stem| {
                asset
                    .name
                    .strip_prefix(stem)
                    .is_some_and(|rest| rest.starts_with("-web-"))
            })
    })
}

/// The SHA-256 a release publishes for one asset (`sha256:<hex>`).
fn asset_sha256(asset: &GitHubAsset) -> Option<String> {
    let digest = asset.digest.as_deref()?.trim();
    let hex = digest.strip_prefix("sha256:").unwrap_or(digest).trim();
    let hex = hex.to_ascii_lowercase();
    (hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit())).then_some(hex)
}

/// The Linux package that ships the app's command-line tool (its MCP server):
/// `<stem>-<version>-linux-<arch>.tar.gz`.
fn cli_asset(app: &CraftApp, release: &GitHubRelease) -> Option<CliAsset> {
    let arch = linux_arch()?;
    let suffix = format!("-linux-{arch}.tar.gz");
    release.assets.iter().find_map(|asset| {
        let rest = asset.name.strip_suffix(&suffix)?;
        app.stems()
            .find(|stem| rest.strip_prefix(stem).is_some_and(|mid| mid.starts_with('-')))
            .map(|_| CliAsset {
                name: asset.name.clone(),
                url: asset.browser_download_url.clone(),
                size: asset.size,
                sha256: asset_sha256(asset),
            })
    })
}

/// A release is only usable when it carries a web build we can name a version
/// for.
fn release_from_github(app: &CraftApp, release: &GitHubRelease) -> Result<Release, String> {
    let version = version_from_tag(&release.tag_name)
        .ok_or_else(|| format!("release tag '{}' is not a version", release.tag_name))?;
    let asset = web_asset(app, release)
        .ok_or_else(|| format!("release v{version} has no {}-web-*.zip asset", app.id))?;
    Ok(Release {
        version,
        asset_name: asset.name.clone(),
        asset_url: asset.browser_download_url.clone(),
        asset_size: asset.size,
        sha256: asset_sha256(asset),
        cli: cli_asset(app, release),
    })
}

/// Whether a URL may be downloaded from: GitHub and its asset CDN only.
fn download_host_allowed(url: &str) -> bool {
    let Some(rest) = url.split_once("://").map(|(_, rest)| rest) else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    // Strip userinfo and port: only the host itself is checked.
    let host = authority.rsplit('@').next().unwrap_or(authority);
    let host = host.split(':').next().unwrap_or(host).to_ascii_lowercase();
    ALLOWED_DOWNLOAD_HOSTS.contains(&host.as_str())
}

// ── Install status ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallState {
    Idle,
    Downloading,
    Installing,
    Failed,
}

#[derive(Debug, Clone)]
struct AppStatus {
    state: InstallState,
    /// Download progress, 0–100.
    progress: u8,
    error: Option<String>,
}

impl Default for AppStatus {
    fn default() -> Self {
        Self { state: InstallState::Idle, progress: 0, error: None }
    }
}

#[derive(Clone)]
struct CachedRelease {
    /// The latest release, or why the last lookup failed (kept so a transient
    /// failure still answers with the previous data when there is any).
    release: Result<Release, String>,
    fetched: Instant,
    etag: Option<String>,
}

// ── The store ───────────────────────────────────────────────────────────────

/// Shared Craft-store state: the release cache, per-app install status and the
/// HTTP client. Created once at startup and kept in `AppState`.
#[derive(Clone)]
pub struct Store {
    inner: Arc<StoreInner>,
}

struct StoreInner {
    client: reqwest::Client,
    /// Downloads (a web build, or the Linux package that carries the CLI) are
    /// tens of megabytes: they get their own, much longer request timeout.
    download_client: reqwest::Client,
    releases: parking_lot::Mutex<HashMap<String, CachedRelease>>,
    status: parking_lot::Mutex<HashMap<String, AppStatus>>,
    /// True while a background catalog refresh runs (deduplicates requests).
    refreshing: AtomicBool,
}

impl Default for Store {
    fn default() -> Self {
        Self::new()
    }
}

impl Store {
    pub fn new() -> Self {
        let build = |timeout: Duration| {
            reqwest::Client::builder()
                .user_agent(concat!("Shiny/", env!("CARGO_PKG_VERSION"), " (Craft store)"))
                .timeout(timeout)
                .build()
                .unwrap_or_else(|_| reqwest::Client::new())
        };
        Self {
            inner: Arc::new(StoreInner {
                client: build(Duration::from_secs(30)),
                download_client: build(Duration::from_secs(600)),
                releases: parking_lot::Mutex::new(HashMap::new()),
                status: parking_lot::Mutex::new(HashMap::new()),
                refreshing: AtomicBool::new(false),
            }),
        }
    }

    /// The catalog with each app's installed/latest version and install state.
    /// Non-blocking: entries come from the release cache, and a background
    /// refresh is scheduled for whatever is missing or stale (the UI polls).
    pub fn snapshot(&self, state: &AppState) -> serde_json::Value {
        let installed: HashMap<String, String> = state
            .plugins
            .list()
            .into_iter()
            .map(|manifest| (manifest.name, manifest.version.to_string()))
            .collect();
        let mut stale = false;
        let apps: Vec<serde_json::Value> = SUITE
            .iter()
            .map(|app| {
                let cached = self.inner.releases.lock().get(app.id).cloned();
                let fresh = cached
                    .as_ref()
                    .is_some_and(|entry| entry.fetched.elapsed() < RELEASE_TTL);
                if !fresh {
                    stale = true;
                }
                let status = self
                    .inner
                    .status
                    .lock()
                    .get(app.id)
                    .cloned()
                    .unwrap_or_default();
                let installed_version = installed.get(app.id).cloned();
                let (latest, latest_error) = match cached.as_ref().map(|entry| &entry.release) {
                    Some(Ok(release)) => (Some(release.version.to_string()), None),
                    Some(Err(error)) => (None, Some(error.clone())),
                    None => (None, None),
                };
                let update_available = match (&installed_version, &latest) {
                    (Some(installed), Some(latest)) => {
                        semver::Version::parse(installed)
                            .ok()
                            .zip(semver::Version::parse(latest).ok())
                            .is_some_and(|(installed, latest)| latest > installed)
                    }
                    _ => false,
                };
                let asset_size = cached
                    .as_ref()
                    .and_then(|entry| entry.release.as_ref().ok())
                    .map(|release| release.asset_size)
                    .unwrap_or(0);
                // AI control: the app's CLI is installed on request (it is a
                // 60–90 MB package), and only while the app is focused do its
                // tools exist (see `craft_mcp`).
                let plugins_dir = std::path::Path::new(&state.config.plugins_dir);
                let mcp_installed =
                    crate::plugins::craft_mcp::cli_path(plugins_dir, app.id).is_file();
                let mcp_available = cached
                    .as_ref()
                    .and_then(|entry| entry.release.as_ref().ok())
                    .and_then(|release| release.cli.as_ref())
                    .is_some();
                let mcp_size = cached
                    .as_ref()
                    .and_then(|entry| entry.release.as_ref().ok())
                    .and_then(|release| release.cli.as_ref())
                    .map(|cli| cli.size)
                    .unwrap_or(0);
                let mcp_version = std::fs::read_to_string(
                    crate::plugins::craft_mcp::version_path(plugins_dir, app.id),
                )
                .ok()
                .map(|version| version.trim().to_string())
                .filter(|version| !version.is_empty());
                json!({
                    "id": app.id,
                    "name": app.name,
                    "tagline": app.tagline,
                    "website": app.website(),
                    "repo": format!("https://github.com/{GITHUB_OWNER}/{}", app.id),
                    "installed_version": installed_version,
                    "latest_version": latest,
                    "asset_size": asset_size,
                    "update_available": update_available,
                    "state": status.state,
                    "progress": status.progress,
                    "error": status.error.or(latest_error),
                    "mcp_available": mcp_available,
                    "mcp_installed": mcp_installed,
                    "mcp_version": mcp_version,
                    "mcp_size": mcp_size,
                })
            })
            .collect();
        if stale {
            self.spawn_refresh(false);
        }
        json!({
            "refreshing": self.inner.refreshing.load(Ordering::SeqCst),
            "apps": apps,
        })
    }

    /// Schedule a catalog refresh. `force` ignores the cache TTL; either way a
    /// refresh already in flight wins and this call returns immediately.
    pub fn spawn_refresh(self: &Store, force: bool) {
        if self.inner.refreshing.swap(true, Ordering::SeqCst) {
            return;
        }
        let store = self.clone();
        tokio::spawn(async move {
            store.refresh_all(force).await;
            store.inner.refreshing.store(false, Ordering::SeqCst);
        });
    }

    /// Look each app's latest release up, keeping the previous entry when a
    /// lookup fails (GitHub rate-limits unauthenticated callers, and being
    /// offline must not empty the store).
    async fn refresh_all(&self, force: bool) {
        for app in SUITE.iter() {
            let cached = self.inner.releases.lock().get(app.id).cloned();
            if !force {
                if let Some(entry) = &cached {
                    if entry.fetched.elapsed() < RELEASE_TTL {
                        continue;
                    }
                }
            }
            let etag = cached.as_ref().and_then(|entry| entry.etag.clone());
            let previous = cached.map(|entry| entry.release);
            let (release, etag) = match self.fetch_latest(app, etag.as_deref()).await {
                Ok(Latest::Fetched { release, etag }) => (release, etag),
                // 304: nothing changed, so the previous answer stands.
                Ok(Latest::Unchanged) => match previous {
                    Some(release) => (release, etag),
                    None => (Err("the release list did not change".into()), etag),
                },
                // Offline, or rate-limited: the last good answer — or the last
                // error, which the UI shows — beats an empty store.
                Err(error) => match previous {
                    Some(cached) => {
                        tracing::warn!("Craft store: keeping the cached release for {}: {error}", app.id);
                        (cached, etag)
                    }
                    None => (Err(error), etag),
                },
            };
            self.inner.releases.lock().insert(
                app.id.to_string(),
                CachedRelease { release, fetched: Instant::now(), etag },
            );
        }
    }

    /// The latest release of one app, fetching it when no fresh entry exists.
    async fn release_for(&self, app: &CraftApp) -> Result<Release, AppError> {
        {
            let releases = self.inner.releases.lock();
            if let Some(entry) = releases.get(app.id) {
                if entry.fetched.elapsed() < RELEASE_TTL {
                    return entry
                        .release
                        .clone()
                        .map_err(|error| AppError::Internal(format!("{} has no usable release: {error}", app.name)));
                }
            }
        }
        let etag = self
            .inner
            .releases
            .lock()
            .get(app.id)
            .and_then(|entry| entry.etag.clone());
        match self.fetch_latest(app, etag.as_deref()).await {
            Ok(Latest::Fetched { release, etag }) => {
                self.inner.releases.lock().insert(
                    app.id.to_string(),
                    CachedRelease { release: release.clone(), fetched: Instant::now(), etag },
                );
                release.map_err(|error| AppError::Internal(format!("{} has no usable release: {error}", app.name)))
            }
            Ok(Latest::Unchanged) => self
                .inner
                .releases
                .lock()
                .get(app.id)
                .and_then(|entry| entry.release.clone().ok())
                .ok_or_else(|| AppError::Internal(format!("{}: release lookup did not return data", app.name))),
            // Offline, or GitHub rate-limited: a stale entry is still a real
            // release, and installing a known version is better than refusing.
            Err(error) => match self
                .inner
                .releases
                .lock()
                .get(app.id)
                .map(|entry| entry.release.clone())
            {
                Some(Ok(release)) => {
                    tracing::warn!("Craft store: using the cached release for {}: {error}", app.id);
                    Ok(release)
                }
                _ => Err(AppError::Internal(format!("could not check {}: {error}", app.name))),
            },
        }
    }

    async fn fetch_latest(&self, app: &CraftApp, etag: Option<&str>) -> Result<Latest, String> {
        let url = format!("https://api.github.com/repos/{GITHUB_OWNER}/{}/releases/latest", app.id);
        let mut request = self
            .inner
            .client
            .get(&url)
            .header("Accept", "application/vnd.github+json");
        if let Some(etag) = etag {
            request = request.header("If-None-Match", etag);
        }
        let response = request.send().await.map_err(|error| error.to_string())?;
        if response.status() == reqwest::StatusCode::NOT_MODIFIED {
            return Ok(Latest::Unchanged);
        }
        if !response.status().is_success() {
            return Err(format!("GitHub answered {} for {}", response.status(), url));
        }
        let etag = response
            .headers()
            .get("etag")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let body = response.json::<GitHubRelease>().await.map_err(|error| error.to_string())?;
        Ok(Latest::Fetched { release: release_from_github(app, &body), etag })
    }

    /// Start installing (or updating) one app and return immediately. The
    /// download and the install run in the background; the UI follows the
    /// store snapshot, and every step is appended to the plugin install log.
    pub async fn start_install(&self, state: &AppState, id: &str) -> Result<(), AppError> {
        let app = CraftApp::by_id(id)
            .ok_or_else(|| AppError::BadRequest(format!("unknown Craft app '{id}'")))?;
        let busy = {
            let status = self.inner.status.lock();
            matches!(
                status.get(app.id).map(|entry| entry.state),
                Some(InstallState::Downloading) | Some(InstallState::Installing)
            )
        };
        if busy {
            return Err(AppError::BadRequest(format!("{} is already being installed", app.name)));
        }
        // Resolve the release now so an unknown app fails the request instead
        // of a background task.
        let release = self.release_for(app).await?;
        self.set_status(app.id, InstallState::Downloading, 0, None);
        let store = self.clone();
        let state = state.clone();
        tokio::spawn(async move {
            if let Err(error) = store.install_release(&state, app, &release).await {
                let plugins_dir = PathBuf::from(&state.config.plugins_dir);
                crate::plugins::installer::log_event(
                    &plugins_dir,
                    &format!("craft-install-failed app={} version={} {error}", app.id, release.version),
                );
                store.set_status(app.id, InstallState::Failed, 0, Some(error.to_string()));
            }
        });
        Ok(())
    }

    async fn install_release(
        &self,
        state: &AppState,
        app: &CraftApp,
        release: &Release,
    ) -> Result<(), AppError> {
        let plugins_dir = PathBuf::from(&state.config.plugins_dir);
        crate::plugins::installer::log_event(
            &plugins_dir,
            &format!(
                "craft-download-begin app={} version={} asset={} bytes={}",
                app.id, release.version, release.asset_name, release.asset_size
            ),
        );
        let (bytes, verified) = self
            .download_asset(
                app.id,
                &release.asset_name,
                &release.asset_url,
                release.asset_size,
                release.sha256.as_deref(),
            )
            .await?;
        if verified {
            log_event(
                &plugins_dir,
                &format!(
                    "craft-verify-ok app={} sha256={}",
                    app.id,
                    release.sha256.as_deref().unwrap_or_default()
                ),
            );
        } else {
            // The release carries no digest for this asset: install it, but say
            // so — the log is the audit trail.
            log_event(
                &plugins_dir,
                &format!("craft-verify-skipped app={}: the release publishes no digest", app.id),
            );
        }
        self.set_status(app.id, InstallState::Installing, 100, None);
        let icon = self.fetch_icon(app).await;
        let manager = state.plugins.clone();
        let base_ctx = state.plugin_ctx();
        let installed = crate::plugins::installer::install_web_app(
            bytes,
            manifest_toml(app, release),
            icon,
            &plugins_dir,
            &manager,
            base_ctx,
        )
        .await?;
        if let Some(rebuild) = &state.router_rebuild {
            rebuild();
        }
        crate::plugins::installer::log_event(
            &plugins_dir,
            &format!("craft-install-ok app={} plugin={installed} version={}", app.id, release.version),
        );
        self.inner.status.lock().remove(app.id);
        Ok(())
    }

    /// Start downloading an app's command-line tool (its MCP server) so the AI
    /// can drive that app while its window is focused. Returns immediately;
    /// progress and failures land in the store snapshot and the install log.
    pub async fn start_mcp_install(&self, state: &AppState, id: &str) -> Result<(), AppError> {
        let app = CraftApp::by_id(id)
            .ok_or_else(|| AppError::BadRequest(format!("unknown Craft app '{id}'")))?;
        if self.is_busy(app.id) {
            return Err(AppError::BadRequest(format!("{} is busy", app.name)));
        }
        let release = self.release_for(app).await?;
        let asset = release.cli.clone().ok_or_else(|| {
            AppError::BadRequest(format!(
                "{} does not publish a Linux command-line tool for this machine in v{}",
                app.name, release.version
            ))
        })?;
        self.set_status(app.id, InstallState::Downloading, 0, None);
        let store = self.clone();
        let state = state.clone();
        tokio::spawn(async move {
            if let Err(error) = store.install_cli(&state, app, &release, &asset).await {
                let plugins_dir = PathBuf::from(&state.config.plugins_dir);
                crate::plugins::installer::log_event(
                    &plugins_dir,
                    &format!("craft-mcp-failed app={} {error}", app.id),
                );
                store.set_status(app.id, InstallState::Failed, 0, Some(error.to_string()));
            }
        });
        Ok(())
    }

    /// Download the Linux package, pull `<app>-cli` out of it and install it as
    /// `PLUGINS_DIR/.craft-mcp/<id>/bin/<id>-cli` (mode 0755).
    async fn install_cli(
        &self,
        state: &AppState,
        app: &'static CraftApp,
        release: &Release,
        asset: &CliAsset,
    ) -> Result<(), AppError> {
        let plugins_dir = PathBuf::from(&state.config.plugins_dir);
        crate::plugins::installer::log_event(
            &plugins_dir,
            &format!(
                "craft-mcp-begin app={} version={} asset={} bytes={}",
                app.id, release.version, asset.name, asset.size
            ),
        );
        let (bytes, verified) = self
            .download_asset(app.id, &asset.name, &asset.url, asset.size, asset.sha256.as_deref())
            .await?;
        crate::plugins::installer::log_event(
            &plugins_dir,
            &format!(
                "craft-mcp-verify-{} app={}",
                if verified { "ok" } else { "skipped" },
                app.id
            ),
        );
        self.set_status(app.id, InstallState::Installing, 100, None);
        let binary = extract_cli(&bytes, app.id)?;
        let target = crate::plugins::craft_mcp::cli_path(&plugins_dir, app.id);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&target, &binary)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(&target)?.permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&target, permissions)?;
        }
        std::fs::write(
            crate::plugins::craft_mcp::version_path(&plugins_dir, app.id),
            release.version.to_string(),
        )?;
        crate::plugins::installer::log_event(
            &plugins_dir,
            &format!(
                "craft-mcp-ok app={} version={} bytes={}",
                app.id,
                release.version,
                binary.len()
            ),
        );
        self.inner.status.lock().remove(app.id);
        Ok(())
    }

    /// Remove an app's AI-control bundle ("AI control" off).
    pub fn remove_mcp(&self, state: &AppState, id: &str) -> Result<(), AppError> {
        let app = CraftApp::by_id(id)
            .ok_or_else(|| AppError::BadRequest(format!("unknown Craft app '{id}'")))?;
        let plugins_dir = PathBuf::from(&state.config.plugins_dir);
        let dir = plugins_dir.join(".craft-mcp").join(app.id);
        if dir.exists() {
            std::fs::remove_dir_all(&dir).map_err(|error| {
                AppError::Internal(format!("could not remove {} AI control: {error}", app.name))
            })?;
        }
        crate::plugins::installer::log_event(
            &plugins_dir,
            &format!("craft-mcp-removed app={}", app.id),
        );
        Ok(())
    }

    fn is_busy(&self, id: &str) -> bool {
        matches!(
            self.inner.status.lock().get(id).map(|entry| entry.state),
            Some(InstallState::Downloading) | Some(InstallState::Installing)
        )
    }

    /// Stream one asset into memory, reporting progress as it goes, and verify
    /// it against the release's digest when one is published. The boolean says
    /// whether a digest was checked.
    async fn download_asset(
        &self,
        app_id: &str,
        asset_name: &str,
        url: &str,
        size: u64,
        sha256: Option<&str>,
    ) -> Result<(Vec<u8>, bool), AppError> {
        if !download_host_allowed(url) {
            return Err(AppError::BadRequest(format!(
                "refusing to download {url} from a host that is not GitHub"
            )));
        }
        let response = self
            .inner
            .download_client
            .get(url)
            .send()
            .await
            .map_err(|error| AppError::Internal(format!("download failed: {error}")))?;
        if !response.status().is_success() {
            return Err(AppError::Internal(format!(
                "download failed: {asset_name} answered {}",
                response.status()
            )));
        }
        let total = response.content_length().unwrap_or(size);
        if total > MAX_DOWNLOAD_BYTES {
            return Err(AppError::BadRequest(format!(
                "{asset_name} is larger than the {MAX_DOWNLOAD_BYTES}-byte limit"
            )));
        }
        let mut bytes: Vec<u8> = Vec::with_capacity(total as usize);
        let mut stream = response.bytes_stream();
        while let Some(chunk) = futures::StreamExt::next(&mut stream).await {
            let chunk = chunk.map_err(|error| AppError::Internal(format!("download failed: {error}")))?;
            bytes.extend_from_slice(&chunk);
            if bytes.len() as u64 > MAX_DOWNLOAD_BYTES {
                return Err(AppError::BadRequest(format!(
                    "{asset_name} grew past the {MAX_DOWNLOAD_BYTES}-byte limit"
                )));
            }
            let progress = (bytes.len() as u64)
                .saturating_mul(100)
                .checked_div(total)
                .unwrap_or(0)
                .min(99) as u8;
            self.set_status(app_id, InstallState::Downloading, progress, None);
        }
        if let Some(expected) = sha256 {
            let actual = hex::encode(sha2::Sha256::digest(&bytes));
            if !actual.eq_ignore_ascii_case(expected) {
                return Err(AppError::BadRequest(format!(
                    "checksum mismatch for {asset_name}: expected {expected}, downloaded {actual}"
                )));
            }
            return Ok((bytes, true));
        }
        Ok((bytes, false))
    }

    /// The app's own icon, copied unmodified from its repository (best effort:
    /// no network, or no icon, must not fail an install).
    async fn fetch_icon(&self, app: &CraftApp) -> Option<Vec<u8>> {
        let url = format!(
            "https://{ICON_HOST}/{GITHUB_OWNER}/{}/main/assets/app-icon/hicolor/128x128/apps/ai.storyteller.{}.png",
            app.id, app.id
        );
        let response = self.inner.client.get(&url).send().await.ok()?;
        if !response.status().is_success() {
            return None;
        }
        let bytes = response.bytes().await.ok()?;
        if bytes.len() > MAX_ICON_BYTES || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            return None;
        }
        Some(bytes.to_vec())
    }

    fn set_status(&self, id: &str, state: InstallState, progress: u8, error: Option<String>) {
        self.inner
            .status
            .lock()
            .insert(id.to_string(), AppStatus { state, progress, error });
    }
}

enum Latest {
    Unchanged,
    Fetched { release: Result<Release, String>, etag: Option<String> },
}

/// Pull `<id>-cli` out of its Linux release tarball without extracting the
/// rest of the archive — it carries the whole app (two ~60 MB binaries). The
/// binary lives at `<package>/bin/<id>-cli`.
fn extract_cli(bytes: &[u8], id: &str) -> Result<Vec<u8>, AppError> {
    let reader = flate2::read::GzDecoder::new(bytes);
    let mut archive = tar::Archive::new(reader);
    let wanted = format!("{id}-cli");
    let entries = archive
        .entries()
        .map_err(|error| AppError::BadRequest(format!("invalid app archive: {error}")))?;
    for entry in entries {
        let mut entry =
            entry.map_err(|error| AppError::BadRequest(format!("invalid app archive: {error}")))?;
        let path = entry
            .path()
            .map_err(|error| AppError::BadRequest(format!("invalid app archive path: {error}")))?
            .into_owned();
        if !path.components().any(|part| part.as_os_str() == "bin") {
            continue;
        }
        if path.file_name().and_then(|name| name.to_str()) != Some(wanted.as_str()) {
            continue;
        }
        let size = entry.size();
        if size > MAX_CLI_BYTES {
            return Err(AppError::BadRequest(format!(
                "{wanted} is larger than the {MAX_CLI_BYTES}-byte limit"
            )));
        }
        let mut binary = Vec::with_capacity(size as usize);
        entry
            .read_to_end(&mut binary)
            .map_err(|error| AppError::BadRequest(format!("could not read {wanted}: {error}")))?;
        if binary.len() as u64 > MAX_CLI_BYTES {
            return Err(AppError::BadRequest(format!(
                "{wanted} grew past the {MAX_CLI_BYTES}-byte limit"
            )));
        }
        return Ok(binary);
    }
    Err(AppError::BadRequest(format!(
        "the release archive has no bin/{wanted}"
    )))
}

/// The manifest a store-installed app gets. Rewritten on every install, so it
/// never carries anything a hand edit should keep.
fn manifest_toml(app: &CraftApp, release: &Release) -> String {
    let escape = |value: &str| value.replace('\\', "\\\\").replace('"', "\\\"");
    format!(
        "# Generated by the Craft store on every install/update — do not edit.\n\
         name = \"{id}\"\n\
         version = \"{version}\"\n\
         api_level = 1\n\
         kind = \"web\"\n\
         description = \"{tagline}\"\n\
         summary = \"{name} — the app's own web build, installed by the Craft store\"\n\
         author = \"ArtCraft Team\"\n\
         web_dir = \"web\"\n\
         category = \"Craft\"\n",
        id = escape(app.id),
        version = release.version,
        name = escape(app.name),
        tagline = escape(app.tagline),
    )
}

// ── HTTP handlers ───────────────────────────────────────────────────────────

/// GET /api/plugins/store — the catalog with installed/latest versions,
/// install state and progress. Cheap: cached data plus a background refresh of
/// whatever is stale, which the caller sees on its next poll.
pub async fn store_list(
    State(state): State<AppState>,
    Extension(_traveler): Extension<Traveler>,
) -> Result<Json<serde_json::Value>, AppError> {
    Ok(Json(json!({ "success": true, "data": state.store.snapshot(&state) })))
}

/// POST /api/plugins/store/refresh — re-check every app's latest release,
/// ignoring the cache. Returns as soon as the check is scheduled.
pub async fn store_refresh(
    State(state): State<AppState>,
    Extension(_traveler): Extension<Traveler>,
) -> Result<Json<serde_json::Value>, AppError> {
    state.store.spawn_refresh(true);
    Ok(Json(json!({ "success": true, "data": { "refreshing": true } })))
}

/// POST /api/plugins/store/install  body {"id":"photocraft"} — download,
/// verify and install (or update) one app. The work runs in the background;
/// progress and failures are reported by the store snapshot and the plugin
/// install log.
pub async fn store_install(
    State(state): State<AppState>,
    Extension(_traveler): Extension<Traveler>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, AppError> {
    let id = body
        .get("id")
        .and_then(|value| value.as_str())
        .ok_or_else(|| AppError::BadRequest("id required".into()))?;
    state.store.start_install(&state, id).await?;
    Ok(Json(json!({ "success": true, "data": { "installing": id } })))
}

/// POST /api/plugins/store/mcp  body {"id":"photocraft","enable":true|false} —
/// install (or remove) the app's command-line tool, which is also its MCP
/// server. The agent can use it only while that app's window is in focus
/// mode; that rule is enforced by the MCP host, not by prompt wording.
pub async fn store_mcp(
    State(state): State<AppState>,
    Extension(_traveler): Extension<Traveler>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, AppError> {
    let id = body
        .get("id")
        .and_then(|value| value.as_str())
        .ok_or_else(|| AppError::BadRequest("id required".into()))?;
    let enable = body
        .get("enable")
        .and_then(|value| value.as_bool())
        .unwrap_or(true);
    if enable {
        state.store.start_mcp_install(&state, id).await?;
    } else {
        state.store.remove_mcp(&state, id)?;
    }
    Ok(Json(json!({
        "success": true,
        "data": { "id": id, "mcp": enable },
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A trimmed real release: PhotoCraft v0.10.0 as GitHub answers it. The
    /// web asset's size and digest are the published ones (its archive wraps
    /// the site in a single `photocraft-web-0.10.0/` folder).
    const PHOTOCRAFT_RELEASE: &str = r#"{
        "tag_name": "v0.10.0",
        "assets": [
            {
                "name": "photocraft-0.10.0-linux-x86_64.AppImage",
                "browser_download_url": "https://github.com/storytold/photocraft/releases/download/v0.10.0/photocraft-0.10.0-linux-x86_64.AppImage",
                "size": 78852915,
                "digest": "sha256:cd1a33c979e8a5da94ded81d7648e779cb8069b77e4aca743c8d177ca5270d77"
            },
            {
                "name": "photocraft-web-0.10.0.zip",
                "browser_download_url": "https://github.com/storytold/photocraft/releases/download/v0.10.0/photocraft-web-0.10.0.zip",
                "size": 11428841,
                "digest": "sha256:b121472d1cdc2f24530b8239c4f32843afdbc1da994244e98a2ce4835b410590"
            },
            {
                "name": "photocraft-0.10.0-linux-x86_64.tar.gz",
                "browser_download_url": "https://github.com/storytold/photocraft/releases/download/v0.10.0/photocraft-0.10.0-linux-x86_64.tar.gz",
                "size": 83037328,
                "digest": "sha256:c5d455785d9c2eaf695bf8e6327c4d6d23a39ddb956a6114f0c5b64326f984e1"
            },
            {
                "name": "SHA256SUMS.txt",
                "browser_download_url": "https://github.com/storytold/photocraft/releases/download/v0.10.0/SHA256SUMS.txt",
                "size": 2554
            }
        ]
    }"#;

    fn parse(json: &str) -> GitHubRelease {
        serde_json::from_str(json).expect("fixture parses")
    }

    #[test]
    fn suite_ids_are_unique_repo_stems() {
        assert_eq!(SUITE.len(), 13);
        let mut ids: Vec<_> = SUITE.iter().map(|app| app.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), SUITE.len());
        for app in &SUITE {
            assert_eq!(app.name.to_lowercase(), app.id);
            assert!(app.website().ends_with(app.id));
        }
    }

    /// The platform packages come first in a real release; the web build is
    /// what we install, and only for the app's own stems.
    #[test]
    fn the_web_build_is_picked_from_a_release() {
        let release = parse(PHOTOCRAFT_RELEASE);
        let app = CraftApp::by_id("photocraft").expect("known app");
        let picked = release_from_github(app, &release).expect("web asset found");
        assert_eq!(picked.version, semver::Version::new(0, 10, 0));
        assert_eq!(picked.asset_name, "photocraft-web-0.10.0.zip");
        assert_eq!(picked.asset_size, 11_428_841);
        assert_eq!(
            picked.sha256.as_deref(),
            Some("b121472d1cdc2f24530b8239c4f32843afdbc1da994244e98a2ce4835b410590")
        );
        // The same release carries the Linux package the CLI (MCP server) comes
        // from; its digest is the published one.
        let cli = picked.cli.as_ref().expect("the Linux CLI package is picked");
        assert_eq!(cli.name, "photocraft-0.10.0-linux-x86_64.tar.gz");
        assert_eq!(cli.size, 83_037_328);
        assert_eq!(
            cli.sha256.as_deref(),
            Some("c5d455785d9c2eaf695bf8e6327c4d6d23a39ddb956a6114f0c5b64326f984e1")
        );

        // Another app's release must not satisfy this one.
        let other = CraftApp::by_id("filmcraft").expect("known app");
        assert!(release_from_github(other, &release).is_err());
    }

    /// PdfCraft shipped as `printcraft` until 0.2.1; older releases must
    /// still resolve.
    #[test]
    fn former_asset_stems_are_accepted() {
        let release = parse(
            r#"{"tag_name": "v0.2.0", "assets": [{ "name": "printcraft-web-0.2.0.zip",
                "browser_download_url": "https://github.com/storytold/pdfcraft/releases/download/v0.2.0/printcraft-web-0.2.0.zip",
                "size": 1 }]}"#,
        );
        let pdf = CraftApp::by_id("pdfcraft").expect("known app");
        let picked = release_from_github(pdf, &release).expect("web asset found");
        assert_eq!(picked.version, semver::Version::new(0, 2, 0));
        assert_eq!(picked.sha256, None);
    }

    /// The tag carries the version; a release without a web build is reported,
    /// not installed.
    #[test]
    fn releases_without_a_web_build_are_rejected() {
        let release = parse(r#"{"tag_name": "v1.2.3", "assets": []}"#);
        let app = CraftApp::by_id("wordcraft").expect("known app");
        let error = release_from_github(app, &release).expect_err("no web asset");
        assert!(error.contains("wordcraft-web"), "{error}");

        let release = parse(r#"{"tag_name": "nightly", "assets": []}"#);
        assert!(release_from_github(app, &release).is_err());
    }

    #[test]
    fn digests_are_only_accepted_as_sha256_hex() {
        let asset = |digest: Option<&str>| GitHubAsset {
            name: "x.zip".into(),
            browser_download_url: String::new(),
            size: 0,
            digest: digest.map(str::to_string),
        };
        assert_eq!(
            asset_sha256(&asset(Some("sha256:ABCDEF"))),
            None,
            "a short digest is not a checksum"
        );
        assert_eq!(asset_sha256(&asset(None)), None);
        let hex = "a".repeat(64);
        assert_eq!(asset_sha256(&asset(Some(&format!("sha256:{hex}")))), Some(hex.clone()));
        // A bare hex digest (no scheme) is accepted too.
        assert_eq!(asset_sha256(&asset(Some(&hex))), Some(hex));
        assert_eq!(asset_sha256(&asset(Some("sha512:deadbeef"))), None);
    }

    /// Downloads are GitHub-only, whatever a (hostile) API response claims.
    #[test]
    fn downloads_are_restricted_to_github() {
        assert!(download_host_allowed(
            "https://github.com/storytold/photocraft/releases/download/v0.10.0/photocraft-web-0.10.0.zip"
        ));
        assert!(download_host_allowed("https://objects.githubusercontent.com/x"));
        assert!(download_host_allowed("https://release-assets.githubusercontent.com/x"));
        for bad in [
            "https://example.com/photocraft-web-0.10.0.zip",
            "https://github.com.evil.example/x.zip",
            "https://evil.example/github.com/x.zip",
            "http://127.0.0.1:8080/x.zip",
            "file:///etc/passwd",
            "not a url",
        ] {
            assert!(!download_host_allowed(bad), "{bad} must be refused");
        }
    }

    /// The Linux package carries the whole app; only its `bin/<app>-cli` is
    /// extracted. The archive nests it under the package directory
    /// (`photocraft-0.10.0-linux-x86_64/bin/`).
    #[test]
    fn the_cli_is_pulled_out_of_the_linux_package() {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use std::io::Write as _;

        let encoder = GzEncoder::new(Vec::new(), Compression::default());
        let mut builder = tar::Builder::new(encoder);
        for (name, body) in [
            ("photocraft-0.10.0-linux-x86_64/bin/photocraft", b"app".as_slice()),
            ("photocraft-0.10.0-linux-x86_64/bin/photocraft-cli", b"cli-binary".as_slice()),
        ] {
            let mut header = tar::Header::new_gnu();
            header.set_size(body.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder
                .append_data(&mut header, name, body)
                .expect("append entry");
        }
        let bytes = builder.into_inner().expect("tar").finish().expect("gzip");
        let _ = bytes.len();

        let cli = extract_cli(&bytes, "photocraft").expect("the CLI is in the package");
        assert_eq!(cli, b"cli-binary");
        // Another app's package does not satisfy this one.
        assert!(extract_cli(&bytes, "filmcraft").is_err());
    }

    /// The generated manifest has to load as an ordinary web-app plugin: the
    /// loader's host extras are what select the no-cdylib path.
    #[test]
    fn the_generated_manifest_loads_as_a_web_app() {
        let app = CraftApp::by_id("photocraft").expect("known app");
        let release = Release {
            version: semver::Version::new(0, 10, 0),
            asset_name: "photocraft-web-0.10.0.zip".into(),
            asset_url: String::new(),
            asset_size: 0,
            sha256: None,
            cli: None,
        };
        let toml_text = manifest_toml(app, &release);
        let manifest: shiny_plugin_sdk::manifest::Manifest =
            toml::from_str(&toml_text).expect("generated manifest parses");
        assert_eq!(manifest.name, "photocraft");
        assert_eq!(manifest.version, semver::Version::new(0, 10, 0));
        assert_eq!(manifest.entry_symbol, "", "a web app has no entry symbol");
        let extras = crate::plugins::loader::parse_extras(&toml_text);
        assert!(extras.kind.is_web());
        assert_eq!(extras.category.as_deref(), Some("Craft"));
        // Quotes in names/taglines must not break the document.
        let odd = CraftApp { former_ids: &[], ..*app };
        let escaped = manifest_toml(&CraftApp { tagline: "a \"quoted\" tagline", ..odd }, &release);
        assert!(toml::from_str::<shiny_plugin_sdk::manifest::Manifest>(&escaped).is_ok());
    }
}
