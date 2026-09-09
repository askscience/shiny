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
    /// `Arc` (not `Box`) so lifecycle hooks (`on_load`/`on_unload`) can be
    /// awaited without holding the loader lock.
    pub plugin: Arc<dyn Plugin>,
    pub ctx: Arc<PluginCtx>,
    pub library: Library,
    pub install_dir: PathBuf,
}

unsafe impl Send for LoadedPlugin {}
unsafe impl Sync for LoadedPlugin {}

/// Only the `category` key of a plugin.toml — parsed independently so the
/// category can be read from disk without touching the `Manifest` ABI.
#[derive(serde::Deserialize)]
struct ManifestCategory {
    #[serde(default)]
    category: Option<String>,
}

pub struct Loader {
    loaded: Arc<RwLock<Vec<LoadedPlugin>>>,
}

/// Libraries of uninstalled/replaced plugins, kept for the life of the
/// process. We intentionally never `dlclose` a plugin cdylib: `Arc<dyn Tool>`
/// objects (and their vtables) may still be referenced by in-flight agent
/// invocations, and unmapping the library under them is use-after-free. The
/// leak is bounded by the number of uninstall/reinstall events.
static LIBRARY_GRAVEYARD: parking_lot::Mutex<Vec<Library>> = parking_lot::Mutex::new(Vec::new());

/// Retire a loaded plugin: keep its library mapped (see `LIBRARY_GRAVEYARD`)
/// and drop the rest of the handle.
fn retire(loaded: LoadedPlugin) {
    LIBRARY_GRAVEYARD.lock().push(loaded.library);
    // `plugin` (Box<dyn Plugin>) and `ctx` drop here.
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

        // Category is a frontend grouping hint, read from the same toml but
        // kept out of `Manifest` to preserve the plugin ABI.
        let category = toml::from_str::<ManifestCategory>(&manifest_text)
            .ok()
            .and_then(|c| c.category)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

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

        // Locate the cdylib file.
        let lib_path = find_cdylib(install_dir, &manifest.name)
            .ok_or_else(|| AppError::BadRequest(format!(
                "No cdylib found in {} for plugin '{}'", install_dir.display(), manifest.name
            )))?;

        // On Windows the file may be locked by the previous load — copy to a
        // versioned path so we can re-install while the old Library is alive.
        let load_path = make_loadable_copy(&lib_path);

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
        let plugin: Arc<dyn Plugin> = if raw.is_null() {
            return Err(AppError::Internal("Plugin entry returned null".into()));
        } else {
            let boxed: Box<dyn Plugin> = unsafe { Box::from_raw(raw) };
            boxed.into()
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
        plugin.register(base_ctx.clone(), &mut builder);

        // Build per-plugin ctx with the manifest's snapshot. The plugin opens
        // its own SQLite pool lazily via `ctx.pool()` — never share ours.
        let ctx = base_ctx.with_manifest(manifest.clone());

        // Stash the loaded plugin (this keeps the library open).
        let loaded = LoadedPlugin {
            manifest: manifest.clone(),
            category,
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
        true
    }
}

pub(crate) fn find_cdylib(install_dir: &Path, name: &str) -> Option<PathBuf> {
    // Accept any `.so` / `.dylib` / `.dll` file at or below the install dir.
    // We prefer files whose stem contains `name`, but fall back to the first
    // cdylib we encounter — plugin authors can name the lib whatever they
    // want as long as there's exactly one cdylib per archive.
    let mut preferred: Option<PathBuf> = None;
    let mut fallback: Option<PathBuf> = None;
    let mut stack: Vec<PathBuf> = vec![install_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let fname = entry.file_name().to_string_lossy().to_lowercase();
                let is_cdylib = fname.ends_with(".so")
                    || fname.ends_with(".dylib")
                    || fname.ends_with(".dll");
                if !is_cdylib {
                    continue;
                }
                // `.so` files like `libshiny_hello_plugin.so` are fine.
                if fname.contains(name) {
                    preferred = Some(path);
                    return Some(preferred.unwrap());
                }
                if fallback.is_none() {
                    fallback = Some(path);
                }
            }
        }
    }
    preferred.or(fallback)
}

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