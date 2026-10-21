//! Plugin loader. Uses `libloading` to dlopen cdylib plugins, then calls their
//! `shiny_plugin_entry` symbol to obtain a `Box<dyn Plugin>`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use libloading::{Library, Symbol};
use parking_lot::RwLock;

use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::manifest::Manifest;
use shiny_plugin_sdk::plugin::{Plugin, PluginEntry, PLUGIN_ENTRY_SYMBOL};
use shiny_plugin_sdk::services::PluginCtx;
use shiny_plugin_sdk::tools::RegistryBuilder;

pub struct LoadedPlugin {
    pub manifest: Manifest,
    /// Human-readable category read from `plugin.toml` (e.g. "Office", "Media").
    /// Kept separate from `Manifest` so adding it doesn't change the plugin
    /// ABI (the `manifest()` trait method returns `&Manifest` across dlopen).
    pub category: Option<String>,
    /// Capabilities declared in `plugin.toml` (e.g. "open-urls"), same ABI
    /// reason as `category`: the frontend resolves features like the link
    /// handler from these declarations instead of hardcoding plugin names.
    pub capabilities: Vec<String>,
    /// `web` for a plugin that is only a static web app (no cdylib). The shell
    /// mounts those in an iframe instead of importing a `web/plugin.js`.
    pub kind: PluginKind,
    /// `Arc` (not `Box`) so lifecycle hooks (`on_load`/`on_unload`) can be
    /// awaited without holding the loader lock.
    pub plugin: Arc<dyn Plugin>,
    pub ctx: Arc<PluginCtx>,
    library: Option<Library>,
    pub install_dir: PathBuf,
}

unsafe impl Send for LoadedPlugin {}
unsafe impl Sync for LoadedPlugin {}

/// What a plugin is made of. `Native` is a cdylib loaded with `dlopen` (the
/// historical shape); `Web` is a static web app served at `/plugins/<name>/`
/// and mounted by the shell in an iframe — it ships no code we execute in
/// this process, so it needs no cdylib and no entry symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PluginKind {
    #[default]
    Native,
    Web,
}

impl PluginKind {
    pub fn is_web(self) -> bool {
        matches!(self, PluginKind::Web)
    }
}

/// Host-side hint keys of a `plugin.toml`, read without touching the
/// `Manifest` ABI. They drive host behaviour only (`category` groups the tray,
/// `capabilities` feeds the link handler, `kind` selects the loader path).
#[derive(Debug, Clone, Default)]
pub(crate) struct HostExtras {
    pub category: Option<String>,
    pub capabilities: Vec<String>,
    pub kind: PluginKind,
}

/// Only the host-side hint keys of a plugin.toml — parsed independently so
/// they can be read from disk without touching the `Manifest` ABI.
#[derive(serde::Deserialize)]
struct ManifestExtras {
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    capabilities: Vec<String>,
    #[serde(default)]
    kind: Option<String>,
}

/// Optional diagnostics only. Legacy archives remain accepted without this
/// table; build metadata is evidence for support, never a binary-safety proof.
#[derive(serde::Deserialize)]
struct BuildMetadata {
    rustc: Option<String>,
    target: Option<String>,
    sdk_revision: Option<String>,
}

fn trace_build_metadata(manifest_text: &str) {
    #[derive(serde::Deserialize)]
    struct Extras { build: Option<BuildMetadata> }
    if let Ok(Extras { build: Some(build) }) = toml::from_str(manifest_text) {
        // Bound untrusted diagnostic strings and escape newlines via Debug.
        let bounded = |value: Option<String>| value.map(|value| value.chars().take(160).collect::<String>());
        tracing::debug!(rustc = ?bounded(build.rustc), target = ?bounded(build.target),
            sdk_revision = ?bounded(build.sdk_revision), "Plugin build metadata");
    }
}

/// Trimmed, empty-dropped host extras for one manifest. These keys are hints,
/// so a missing or malformed extras block yields nothing rather than an error.
pub(crate) fn parse_extras(manifest_text: &str) -> HostExtras {
    trace_build_metadata(manifest_text);
    let extras = toml::from_str::<ManifestExtras>(manifest_text).ok();
    let category = extras
        .as_ref()
        .and_then(|e| e.category.clone())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let kind = match extras.as_ref().and_then(|e| e.kind.as_deref()).map(str::trim) {
        Some("web") => PluginKind::Web,
        _ => PluginKind::Native,
    };
    let capabilities = extras
        .map(|e| e.capabilities)
        .unwrap_or_default()
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    HostExtras { category, capabilities, kind }
}

pub struct Loader {
    loaded: Arc<RwLock<Vec<LoadedPlugin>>>,
}

/// Libraries of uninstalled/replaced plugins, kept for the life of the
/// process. We intentionally never `dlclose` a plugin cdylib: `Arc<dyn Tool>`
/// objects (and their vtables) may still be referenced by in-flight agent
/// invocations, and unmapping the library under them is use-after-free.
///
/// The leak is bounded by the number of install/replace events. So that a user
/// cannot grow it without limit (each mapped library costs address space), the
/// loader refuses to load further plugins once this cap is reached; upgrades
/// stay safe, and a restart reclaims everything.
static LIBRARY_GRAVEYARD: parking_lot::Mutex<Vec<Library>> = parking_lot::Mutex::new(Vec::new());
const MAX_RETIRED_LIBRARIES: usize = 100;

/// Retire a loaded plugin: keep its library mapped (see `LIBRARY_GRAVEYARD`)
/// and drop the rest of the handle.
fn retire(loaded: LoadedPlugin) {
    drop(loaded);
}

impl Drop for LoadedPlugin {
    fn drop(&mut self) {
        // Also cover cancellation while awaiting on_unload: registry tools and
        // router snapshots may still reference this library's vtables then.
        if let Some(library) = self.library.take() {
            LIBRARY_GRAVEYARD.lock().push(library);
        }
    }
}

/// The `Plugin` handle for a web-app plugin (`kind = "web"`). Its whole
/// payload is the static `web/` directory the core already serves, so it
/// contributes no tools, routes, skills or persona — but keeping it behind
/// the same trait means install, activation, uninstall, `kind_for` and the
/// router all keep working unchanged.
struct WebAppPlugin {
    manifest: Manifest,
}

impl Plugin for WebAppPlugin {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn register(&self, _ctx: Arc<PluginCtx>, _builder: &mut RegistryBuilder<'_>) {}
}

/// Validate that a plugin name is a single safe path component. It comes from
/// `plugin.toml` (attacker-controlled on upload) and is joined into the
/// plugins directory, so it must not be able to escape it.
pub fn validate_plugin_name(name: &str) -> Result<(), AppError> {
    const MAX: usize = 64;
    let valid = !name.is_empty()
        && name.len() <= MAX
        && name != "."
        && name != ".."
        && !name.starts_with('.')
        && !name.starts_with('_')
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains('\0');
    if valid {
        Ok(())
    } else {
        Err(AppError::BadRequest(format!(
            "Invalid plugin name '{name}': must be 1..={MAX} chars, a single path component, \
             and must not start with '.' or '_'"
        )))
    }
}

/// Validate a manifest-declared path (`migrations_dir`, `web_dir`, `skills_dir`)
/// before it is joined onto an install directory.
///
/// These fields come from `plugin.toml`, which is attacker-controlled on
/// upload. `Path::join` with an absolute path **replaces** the base, so an
/// unvalidated `web_dir = "/etc"` would let a plugin serve the host filesystem
/// over HTTP, and `migrations_dir = "/tmp"` would execute arbitrary `.sql`
/// files with the host database pool. Only plain relative sub-paths are
/// allowed: no absolute paths, no `..`, no Windows separators.
pub fn validate_relative_path(field: &str, value: &str) -> Result<(), AppError> {
    const MAX: usize = 1024;
    if value.trim().is_empty() || value.len() > MAX || value.contains('\0') || value.contains('\\') {
        return Err(AppError::BadRequest(format!(
            "Invalid {field} '{value}': must be a non-empty relative path"
        )));
    }
    let path = Path::new(value);
    if path.is_absolute() {
        return Err(AppError::BadRequest(format!(
            "Invalid {field} '{value}': absolute paths are not allowed"
        )));
    }
    for component in path.components() {
        match component {
            std::path::Component::Normal(_) | std::path::Component::CurDir => {}
            std::path::Component::ParentDir
            | std::path::Component::RootDir
            | std::path::Component::Prefix(_) => {
                return Err(AppError::BadRequest(format!(
                    "Invalid {field} '{value}': must not escape the plugin directory"
                )));
            }
        }
    }
    Ok(())
}

impl Loader {
    pub fn new() -> Self {
        Self { loaded: Arc::new(RwLock::new(Vec::new())) }
    }

    pub fn snapshot(&self) -> Vec<Manifest> {
        self.loaded.read().iter().map(|p| p.manifest.clone()).collect()
    }

    /// Category for one loaded plugin, if its `plugin.toml` declares one.
    pub fn category_for(&self, name: &str) -> Option<String> {
        self.loaded.read().iter()
            .find(|p| p.manifest.name == name)
            .and_then(|p| p.category.clone())
    }

    /// Capabilities declared in one loaded plugin's `plugin.toml` (empty when
    /// it declares none).
    pub fn capabilities_for(&self, name: &str) -> Vec<String> {
        self.loaded.read().iter()
            .find(|p| p.manifest.name == name)
            .map(|p| p.capabilities.clone())
            .unwrap_or_default()
    }

    /// What kind of plugin this is (native cdylib or web app).
    pub fn kind_for(&self, name: &str) -> PluginKind {
        self.loaded.read().iter()
            .find(|p| p.manifest.name == name)
            .map(|p| p.kind)
            .unwrap_or_default()
    }

    pub fn has(&self, name: &str) -> bool {
        self.loaded.read().iter().any(|p| p.manifest.name == name)
    }

    /// Resolve a plugin's `handler_tag` to a route handler.
    pub fn route_handler(&self, name: &str, tag: &str) -> Option<shiny_plugin_sdk::routes::RouteHandler> {
        self.loaded.read().iter()
            .find(|p| p.manifest.name == name)
            .and_then(|p| p.plugin.route_handler(tag))
    }

    /// Load a plugin directory: parse `plugin.toml`, dlopen `<install_dir>/lib<name>.so`
    /// (or `.dylib` / `.dll`), call its entry, build the `PluginCtx`, call
    /// `register()` into a fresh `RegistryBuilder`, run migrations, and return
    /// the loaded plugin plus its contributions.
    pub async fn install_dir(
        &self,
        install_dir: &Path,
        pool: &sqlx::SqlitePool,
        base_ctx: Arc<PluginCtx>,
    ) -> Result<(Manifest, RegistryBuilder<'static>, Arc<PluginCtx>), AppError> {
        let manifest_path = install_dir.join("plugin.toml");
        let manifest_text = std::fs::read_to_string(&manifest_path)?;
        let manifest: Manifest = toml::from_str(&manifest_text)
            .map_err(|e| AppError::BadRequest(format!("Invalid plugin.toml: {}", e)))?;
        validate_plugin_name(&manifest.name)?;

        // Category, capabilities and kind are host-side hints, read from the
        // same toml but kept out of `Manifest` to preserve the plugin ABI.
        let extras = parse_extras(&manifest_text);

        if manifest.api_level > shiny_plugin_sdk::CORE_API_LEVEL {
            return Err(AppError::BadRequest(format!(
                "Plugin '{}' requires api_level {} but core is at {}",
                manifest.name, manifest.api_level, shiny_plugin_sdk::CORE_API_LEVEL
            )));
        }

        if let Some(want) = &manifest.target_triple {
            let host = current_target_triple();
            if want != &host {
                return Err(AppError::BadRequest(format!(
                    "Plugin '{}' built for '{want}' but host is '{host}'",
                    manifest.name
                )));
            }
        }

        // Manifest-declared directories are attacker-controlled on upload and
        // are joined onto the install directory below; refuse anything that is
        // not a plain relative sub-path (e.g. `web_dir = "/etc"`).
        validate_relative_path("migrations_dir", &manifest.migrations_dir)?;
        validate_relative_path("skills_dir", &manifest.skills_dir)?;
        validate_relative_path("web_dir", &manifest.web_dir)?;

        let (plugin, library): (Arc<dyn Plugin>, Option<Library>) = if extras.kind.is_web() {
            // A web-app plugin is a static site with no code to dlopen. Refuse
            // it early when the entry document is missing, so a broken
            // download fails the install instead of opening an empty window.
            let web_root = install_dir.join(&manifest.web_dir);
            if !web_root.join("index.html").is_file() {
                return Err(AppError::BadRequest(format!(
                    "Web-app plugin '{}' has no {}/index.html to serve",
                    manifest.name, manifest.web_dir
                )));
            }
            (Arc::new(WebAppPlugin { manifest: manifest.clone() }), None)
        } else {
            // Never grow the retired-library graveyard without bound: each
            // entry is a mapped cdylib we cannot safely dlclose. Fail the load
            // instead of letting an install/uninstall loop exhaust the process.
            if LIBRARY_GRAVEYARD.lock().len() >= MAX_RETIRED_LIBRARIES {
                return Err(AppError::Internal(format!(
                    "Too many plugin replacements this session (max {MAX_RETIRED_LIBRARIES}); restart Shiny to load more."
                )));
            }

            // Locate the cdylib file.
            let lib_path = find_cdylib(install_dir, &manifest.name)
                .ok_or_else(|| AppError::BadRequest(format!(
                    "No cdylib found in {} for plugin '{}'", install_dir.display(), manifest.name
                )))?;

            // On Windows the file may be locked by the previous load — copy to a
            // versioned path so we can re-install while the old Library is alive.
            //
            // On Unix that workaround is unnecessary *and* expensive: the original
            // can be `dlopen`ed directly, and it can be overwritten or unlinked
            // while it is mapped (the kernel keeps the inode alive), so a re-install
            // still works. Copying every cdylib was several GiB of disk I/O on every
            // startup — the plugin loaders' real cost — for no benefit here.
            #[cfg(windows)]
            let load_path = make_loadable_copy(&lib_path);
            #[cfg(not(windows))]
            let load_path = lib_path;

            // SAFETY: cdylib plugins must be `Send + Sync` and free of statics
            // accessible after `dlclose`. We retain `Library` in `LoadedPlugin` so
            // it lives as long as the plugin is registered.
            let library = unsafe { Library::new(&load_path) }
                .map_err(|e| AppError::Internal(format!(
                    "Failed to dlopen {}: {}", load_path.display(), e
                )))?;

            // Honor the manifest's declared entry symbol (defaulting to the
            // standard `shiny_plugin_entry`) so the field is actually read.
            let entry_symbol = if manifest.entry_symbol.trim().is_empty() {
                PLUGIN_ENTRY_SYMBOL
            } else {
                manifest.entry_symbol.as_str()
            };
            let entry: Symbol<PluginEntry> = unsafe { library.get(entry_symbol.as_bytes()) }
                .map_err(|e| AppError::Internal(format!(
                    "Missing symbol {entry_symbol} in {}: {}", load_path.display(), e
                )))?;

            // SAFETY: transmute `*mut dyn Plugin` returned by the C symbol into a
            // `Box<dyn Plugin>`. We trust the plugin author's `shiny_plugin_entry`
            // to return a value allocated via `Box::into_raw(Box::new(...))`.
            let raw = unsafe { entry() };
            if raw.is_null() {
                return Err(AppError::Internal("Plugin entry returned null".into()));
            }
            let boxed: Box<dyn Plugin> = unsafe { Box::from_raw(raw) };
            (boxed.into(), Some(library))
        };

        // Sanity: the plugin's manifest matches what's on disk.
        let plugin_manifest = plugin.manifest().clone();
        if plugin_manifest.name != manifest.name {
            return Err(AppError::BadRequest(format!(
                "Plugin manifest name mismatch: {} on disk, {} in code",
                manifest.name, plugin_manifest.name
            )));
        }
        if plugin_manifest.api_level > shiny_plugin_sdk::CORE_API_LEVEL {
            return Err(AppError::BadRequest(format!(
                "Plugin '{}' code api_level {} > core {}", manifest.name, plugin_manifest.api_level, shiny_plugin_sdk::CORE_API_LEVEL
            )));
        }

        // Run plugin migrations — with the HOST pool, in the host process
        // (migrations run core-side by design).
        let migrations_dir = install_dir.join(&manifest.migrations_dir);
        if migrations_dir.exists() {
            shiny_plugin_sdk::migrations::run_plugin_migrations(pool, &manifest.name, &migrations_dir).await?;
        }

        // Register tools / routes / crons into a fresh builder.
        let mut builder = RegistryBuilder::new();
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            plugin.register(base_ctx.clone(), &mut builder);
        })).map_err(|_| AppError::Internal(format!("Plugin '{}' registration panicked", manifest.name)))?;
        for route in &builder.routes {
            if plugin.route_handler(&route.handler_tag).is_none() {
                return Err(AppError::BadRequest(format!(
                    "Plugin '{}' declares an unresolved route handler '{}'", manifest.name, route.handler_tag
                )));
            }
        }

        // Build per-plugin ctx with the manifest's snapshot. The plugin opens
        // its own SQLite pool lazily via `ctx.pool()` — never share ours.
        let ctx = base_ctx.with_manifest(manifest.clone());

        // Stash the loaded plugin (this keeps the library open).
        let loaded = LoadedPlugin {
            manifest: manifest.clone(),
            category: extras.category,
            capabilities: extras.capabilities,
            kind: extras.kind,
            plugin,
            ctx: ctx.clone(),
            library,
            install_dir: install_dir.to_path_buf(),
        };

        // Be sure to unload any prior version of the same plugin name. The
        // lifecycle hook runs and the library is retired (never dlclosed,
        // see `LIBRARY_GRAVEYARD`) without holding the lock across the await.
        let replaced: Vec<LoadedPlugin> = {
            let mut guard = self.loaded.write();
            let mut removed = Vec::new();
            let mut i = 0;
            while i < guard.len() {
                if guard[i].manifest.name == manifest.name {
                    removed.push(guard.remove(i));
                } else {
                    i += 1;
                }
            }
            removed
        };
        for old in replaced {
            old.plugin.on_unload(old.ctx.clone()).await;
            retire(old);
        }
        self.loaded.write().push(loaded);
        // Localized metadata may have changed with this install/replacement.
        crate::i18n::clear_plugin_meta_cache();

        Ok((plugin_manifest, builder, ctx))
    }

    /// Invoke the plugin's `on_load` lifecycle hook (the documented place for
    /// plugins to start background/cron work). Runs after the plugin is
    /// registered and its tools are live. No-op when not loaded.
    pub async fn call_on_load(&self, name: &str) {
        let found = {
            let guard = self.loaded.read();
            guard
                .iter()
                .find(|p| p.manifest.name == name)
                .map(|p| (p.plugin.clone(), p.ctx.clone()))
        };
        if let Some((plugin, ctx)) = found {
            plugin.on_load(ctx).await;
        }
    }

    /// Invoke the plugin's `on_user_registered` lifecycle hook for a newly
    /// registered user. Mirrors `call_on_load`; runs for every loaded plugin so
    /// one can provision per-user state (e.g. Files creates the user's home
    /// folders). Failures are logged and never abort registration.
    pub async fn call_on_user_registered(&self, user_id: &str) {
        let plugins: Vec<(Arc<dyn Plugin>, Arc<PluginCtx>)> = {
            let guard = self.loaded.read();
            guard
                .iter()
                .map(|p| (p.plugin.clone(), p.ctx.clone()))
                .collect()
        };
        for (plugin, ctx) in plugins {
            let name = plugin.manifest().name.clone();
            let hook = std::panic::AssertUnwindSafe(plugin.on_user_registered(ctx, user_id));
            if let Err(panic) = futures::FutureExt::catch_unwind(hook).await {
                let msg = panic
                    .downcast_ref::<&str>()
                    .map(|s| s.to_string())
                    .or_else(|| panic.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "unknown panic".into());
                tracing::warn!("plugin '{name}' on_user_registered panicked: {msg}");
            }
        }
    }

    /// Unload a plugin by name: runs its `on_unload` hook, then retires it.
    /// Its tools must already have been removed from the `ToolRegistry`
    /// (`uninstall_plugin`) before this is called. The cdylib is *not*
    /// dlclosed — it is kept in `LIBRARY_GRAVEYARD` for the rest of the
    /// process lifetime so any late reference stays valid.
    pub async fn unload(&self, name: &str) -> bool {
        let removed: Vec<LoadedPlugin> = {
            let mut guard = self.loaded.write();
            let mut removed = Vec::new();
            let mut i = 0;
            while i < guard.len() {
                if guard[i].manifest.name == name {
                    removed.push(guard.remove(i));
                } else {
                    i += 1;
                }
            }
            removed
        };
        if removed.is_empty() {
            return false;
        }
        for old in removed {
            old.plugin.on_unload(old.ctx.clone()).await;
            retire(old);
        }
        crate::i18n::clear_plugin_meta_cache();
        true
    }
}

pub(crate) fn find_cdylib(install_dir: &Path, name: &str) -> Option<PathBuf> {
    // Accept any regular `.so` / `.dylib` / `.dll` file at or below the install
    // dir. Selection is deterministic so an archive cannot smuggle in a second
    // library and win the race by `read_dir` order:
    //   1. an exact `lib<name>.<ext>` / `<name>.<ext>` match wins;
    //   2. otherwise the lexicographically first cdylib.
    // Symlinks are never followed: a tar archive can contain one, and we must
    // not `dlopen` a file that was not part of the archive's own tree.
    let mut candidates: Vec<PathBuf> = Vec::new();
    let mut stack: Vec<PathBuf> = vec![install_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        paths.sort();
        for path in paths {
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                stack.push(path);
                continue;
            }
            if !meta.is_file() {
                continue;
            }
            let fname = path
                .file_name()
                .map(|f| f.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            if fname.ends_with(".so") || fname.ends_with(".dylib") || fname.ends_with(".dll") {
                candidates.push(path);
            }
        }
    }
    candidates.sort();
    let wanted = name.to_lowercase();
    let selected = candidates
        .iter()
        .find(|p| {
            let stem = p
                .file_stem()
                .map(|s| s.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            stem == wanted || stem == format!("lib{wanted}")
        })
        .cloned()
        .or_else(|| candidates.first().cloned());
    if let Some(path) = &selected {
        warn_if_stale(path);
    }
    selected
}

/// Security floor for plugin binaries. Anything built before this timestamp
/// predates commit `42e484c` (2026-10-04 00:27:28 UTC), which fixed the files
/// sandbox escape, the calculator stack-overflow crash, the terminal remote
/// gate/caps and the ODF/upload limits. Bump the floor when the next security
/// wave lands; set `SHINY_MIN_PLUGIN_TS=0` to silence the check.
const SECURITY_FLOOR_TS: u64 = 1_791_073_648;

/// Warn when a plugin cdylib predates the security floor: a stale install
/// silently undoes fixes that are present in the source tree.
fn warn_if_stale(path: &Path) {
    let floor = std::env::var("SHINY_MIN_PLUGIN_TS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(SECURITY_FLOOR_TS);
    if floor == 0 {
        return;
    }
    let Ok(modified) = std::fs::metadata(path).and_then(|m| m.modified()) else {
        return;
    };
    let Ok(ts) = modified.duration_since(std::time::UNIX_EPOCH) else {
        return;
    };
    if ts.as_secs() < floor {
        tracing::warn!(
            "plugin {} was built before the security floor ({} < {}) — rebuild and reinstall it to pick up the security fixes",
            path.display(),
            ts.as_secs(),
            floor
        );
    }
}

/// Windows-only: copy a cdylib to a timestamped sibling so a re-install can
/// overwrite the original while the previous `Library` still has it locked.
/// Unix `dlopen`s the original in place — see the call site.
#[cfg(windows)]
fn make_loadable_copy(path: &Path) -> PathBuf {
    // Copy to a timestamped sibling so re-installs can overwrite the original
    // even on Windows where the in-use `.dll` is locked.
    if let (Some(dir), Some(file)) = (path.parent(), path.file_name()) {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let copy = dir.join(format!(".{}.{}", file.to_string_lossy(), ts));
        if std::fs::copy(path, &copy).is_ok() {
            return copy;
        }
    }
    path.to_path_buf()
}

fn current_target_triple() -> String {
    // Best static guess — we don't pull in `target-lexicon` for one call.
    let arch = if cfg!(target_arch = "x86_64") { "x86_64" }
        else if cfg!(target_arch = "aarch64") { "aarch64" }
        else { "unknown" };
    let os = if cfg!(target_os = "linux") { "unknown-linux-gnu" }
        else if cfg!(target_os = "macos") { "apple-darwin" }
        else if cfg!(target_os = "windows") { "pc-windows-msvc" }
        else { "unknown" };
    format!("{arch}-{os}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_names_are_single_safe_components() {
        for good in ["hello", "files", "my-plugin", "plugin_1"] {
            assert!(validate_plugin_name(good).is_ok(), "{good} should be valid");
        }
        for bad in ["", ".", "..", ".hidden", "_internal", "a/b", "a\\b", "a\0b"] {
            assert!(validate_plugin_name(bad).is_err(), "{bad:?} should be rejected");
        }
        assert!(validate_plugin_name(&"a".repeat(65)).is_err());
    }

    /// `capabilities` is a host-side hint parsed from the same toml as
    /// `category`; absent or blank entries must never reach the API. `kind`
    /// selects the loader path and defaults to `native`.
    #[test]
    fn host_extras_carry_capabilities_and_kind() {
        let extras = parse_extras(
            "name = \"alt-browser\"\ncapabilities = [\"open-urls\", \"  \", \" media \"]\ncategory = \" Web \"\n",
        );
        assert_eq!(extras.category.as_deref(), Some("Web"));
        assert_eq!(extras.capabilities, vec!["open-urls".to_string(), "media".to_string()]);
        assert_eq!(extras.kind, PluginKind::Native);

        let extras = parse_extras("name = \"legacy\"\n");
        assert!(extras.category.is_none());
        assert!(extras.capabilities.is_empty());
        assert_eq!(extras.kind, PluginKind::Native);

        // An unknown kind is a hint, not an error: it loads as a native plugin.
        assert_eq!(parse_extras("kind = \"plugin\"\n").kind, PluginKind::Native);

        let extras = parse_extras("name = \"photocraft\"\nkind = \" web \"\n");
        assert_eq!(extras.kind, PluginKind::Web);
        assert!(extras.kind.is_web());
    }

    /// Regression: `Path::join` with an absolute path *replaces* the base, so
    /// an unvalidated `web_dir = "/etc"` let a plugin serve the host's
    /// filesystem and `migrations_dir = "/tmp"` ran arbitrary SQL.
    #[test]
    fn manifest_paths_must_stay_relative() {
        for good in ["web", "migrations", "assets/js", "./web"] {
            assert!(
                validate_relative_path("web_dir", good).is_ok(),
                "{good} should be valid"
            );
        }
        for bad in [
            "",
            "/etc",
            "/",
            "../other-plugin",
            "web/../../etc",
            "C:\\Windows",
            "a\\b",
            "a\0b",
        ] {
            assert!(
                validate_relative_path("web_dir", bad).is_err(),
                "{bad:?} should be rejected"
            );
        }
    }
}