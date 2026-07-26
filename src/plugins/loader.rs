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
    pub plugin: Box<dyn Plugin>,
    pub ctx: Arc<PluginCtx>,
    pub library: Library,
    pub install_dir: PathBuf,
}

unsafe impl Send for LoadedPlugin {}
unsafe impl Sync for LoadedPlugin {}

pub struct Loader {
    loaded: Arc<RwLock<Vec<LoadedPlugin>>>,
}

impl Loader {
    pub fn new() -> Self {
        Self { loaded: Arc::new(RwLock::new(Vec::new())) }
    }

    pub fn snapshot(&self) -> Vec<Manifest> {
        self.loaded.read().iter().map(|p| p.manifest.clone()).collect()
    }

    pub fn has(&self, name: &str) -> bool {
        self.loaded.read().iter().any(|p| p.manifest.name == name)
    }

    /// Load a plugin directory: parse `plugin.toml`, dlopen `<install_dir>/lib<name>.so`
    /// (or `.dylib` / `.dll`), call its entry, build the `PluginCtx`, call
    /// `register()` into a fresh `RegistryBuilder`, run migrations, and return
    /// the loaded plugin plus its contributions.
    pub async fn install_dir(
        &self,
        install_dir: &Path,
        base_ctx: Arc<PluginCtx>,
    ) -> Result<(Manifest, RegistryBuilder<'static>, Arc<PluginCtx>), AppError> {
        let manifest_path = install_dir.join("plugin.toml");
        let manifest_text = std::fs::read_to_string(&manifest_path)?;
        let mut manifest: Manifest = toml::from_str(&manifest_text)
            .map_err(|e| AppError::BadRequest(format!("Invalid plugin.toml: {}", e)))?;

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

        let entry: Symbol<PluginEntry> = unsafe { library.get(PLUGIN_ENTRY_SYMBOL.as_bytes()) }
            .map_err(|e| AppError::Internal(format!(
                "Missing symbol {PLUGIN_ENTRY_SYMBOL} in {}: {}", load_path.display(), e
            )))?;

        // SAFETY: transmute `*mut dyn Plugin` returned by the C symbol into a
        // `Box<dyn Plugin>`. We trust the plugin author's `shiny_plugin_entry`
        // to return a value allocated via `Box::into_raw(Box::new(...))`.
        let raw = unsafe { entry() };
        let plugin: Box<dyn Plugin> = if raw.is_null() {
            return Err(AppError::Internal("Plugin entry returned null".into()));
        } else {
            unsafe { Box::from_raw(raw) }
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

        // Run plugin migrations.
        let migrations_dir = install_dir.join(&manifest.migrations_dir);
        if migrations_dir.exists() {
            shiny_plugin_sdk::migrations::run_plugin_migrations(&base_ctx.pool, &manifest.name, &migrations_dir).await?;
        }

        // Register tools / routes / crons into a fresh builder.
        let mut builder = RegistryBuilder::new();
        plugin.register(base_ctx.clone(), &mut builder);

        // Build per-plugin ctx with the manifest's snapshot.
        let ctx = Arc::new(PluginCtx {
            pool: base_ctx.pool.clone(),
            ollama: base_ctx.ollama.clone(),
            search: base_ctx.search.clone(),
            supertonic: base_ctx.supertonic.clone(),
            config: base_ctx.config.clone(),
            manifest: manifest.clone(),
        });

        // Stash the loaded plugin (this keeps the library open).
        let loaded = LoadedPlugin {
            manifest: manifest.clone(),
            plugin,
            ctx: ctx.clone(),
            library,
            install_dir: install_dir.to_path_buf(),
        };

        // Be sure to unload any prior version of the same plugin name:
        let mut guard = self.loaded.write();
        guard.retain(|p| p.manifest.name != manifest.name);
        guard.push(loaded);

        Ok((plugin_manifest, builder, ctx))
    }

    /// Unload a plugin by name. The returned `_Guard` keeps the old Library
    /// alive until dropped — caller is responsible for waiting until no
    /// in-flight tool invocations reference it.
    pub fn unload(&self, name: &str) -> bool {
        let mut guard = self.loaded.write();
        let before = guard.len();
        guard.retain(|p| p.manifest.name != name);
        guard.len() != before
    }
}

fn find_cdylib(install_dir: &Path, name: &str) -> Option<PathBuf> {
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