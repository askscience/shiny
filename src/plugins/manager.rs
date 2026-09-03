//! Plugin manager — holds the registry, persona/skills fragments, and exposes
//! the per-user activation lookup (`user_plugin_states` table). All owner-level
//! state lives in the SQLite pool; the only thing kept in memory is the
//! per-plugin contributions + a manager reference for the dispatcher.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use parking_lot::RwLock;

use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::manifest::Manifest;
use shiny_plugin_sdk::routes::{RouteHandler, RouteSpec};
use shiny_plugin_sdk::services::PluginCtx;
use shiny_plugin_sdk::tools::RegistryBuilder;

use crate::plugins::loader::Loader;
use crate::plugins::registry::ToolRegistry;

/// Per-plugin contributions persisted after `register()`.
pub struct PluginContrib {
    pub manifest: Manifest,
    pub skills_md: String,
    pub persona: String,
    pub context_lines: Vec<String>,
    pub routes: Vec<(RouteSpec, RouteHandler)>,
}

/// Top-level plugin state, cloned cheaply (everything is `Arc`).
pub struct PluginManagerInner {
    pub tools: ToolRegistry,
    pub loader: Loader,
    pub contribs: RwLock<Vec<PluginContrib>>,
    pub plugins_dir: PathBuf,
    /// Shared SQLite pool used for the per-user activation state.
    pub pool: sqlx::SqlitePool,
}

#[derive(Clone)]
pub struct PluginManager {
    pub inner: Arc<PluginManagerInner>,
}

impl PluginManager {
    pub fn new(plugins_dir: PathBuf, pool: sqlx::SqlitePool) -> Self {
        Self {
            inner: Arc::new(PluginManagerInner {
                tools: ToolRegistry::new(),
                loader: Loader::new(),
                contribs: RwLock::new(Vec::new()),
                plugins_dir,
                pool,
            }),
        }
    }

    pub fn tools(&self) -> ToolRegistry { self.inner.tools.clone() }
    pub fn loader(&self) -> &Loader { &self.inner.loader }

    /// Synchronously read a single user's disabled plugin names.
    pub async fn disabled_for(&self, user_id: &str) -> BTreeSet<String> {
        let rows: Vec<String> = sqlx::query_scalar(
            "SELECT plugin_name FROM user_plugin_states WHERE user_id = ?1 AND enabled = 0",
        )
        .bind(user_id)
        .fetch_all(&self.inner.pool)
        .await
        .unwrap_or_default();
        rows.into_iter().collect()
    }

    /// Check whether `user_id` has `plugin_name` enabled. A plugin is enabled
    /// by default (no row = enabled).
    pub async fn is_enabled_for(&self, user_id: &str, plugin_name: &str) -> bool {
        let enabled: Option<i64> = sqlx::query_scalar(
            "SELECT enabled FROM user_plugin_states WHERE user_id = ?1 AND plugin_name = ?2",
        )
        .bind(user_id)
        .bind(plugin_name)
        .fetch_optional(&self.inner.pool)
        .await
        .ok()
        .flatten();
        enabled.map(|e| e == 1).unwrap_or(true)
    }

    pub async fn set_enabled_for(&self, user_id: &str, plugin_name: &str, enabled: bool) -> Result<(), AppError> {
        sqlx::query(
            "INSERT INTO user_plugin_states (user_id, plugin_name, enabled, updated_at) \
             VALUES (?1, ?2, ?3, datetime('now')) \
             ON CONFLICT(user_id, plugin_name) DO UPDATE SET \
               enabled = excluded.enabled, updated_at = datetime('now')",
        )
        .bind(user_id)
        .bind(plugin_name)
        .bind(if enabled { 1 } else { 0 })
        .execute(&self.inner.pool)
        .await?;
        Ok(())
    }

    /// Read the user's `session.remember` preference (default false). This one
    /// switch decides whether a sign-in resumes the saved workspace (plugins,
    /// windows, layout) or starts fresh (core assistant only, empty desktop).
    pub async fn session_remember(&self, user_id: &str) -> bool {
        let value: Option<String> = sqlx::query_scalar(
            "SELECT value FROM user_preferences WHERE user_id = ?1 AND key = 'session.remember'",
        )
        .bind(user_id)
        .fetch_optional(&self.inner.pool)
        .await
        .ok()
        .flatten();
        matches!(value.as_deref(), Some("true") | Some("1"))
    }

    /// Plugins active for `user_id` THIS session. Empty in fresh mode
    /// (`session.remember` off); otherwise the persisted enabled set
    /// (installed minus explicitly-disabled).
    pub async fn session_active_set(&self, user_id: &str) -> BTreeSet<String> {
        if !self.session_remember(user_id).await {
            return BTreeSet::new();
        }
        let disabled = self.disabled_for(user_id).await;
        self.list()
            .into_iter()
            .map(|m| m.name)
            .filter(|n| !disabled.contains(n))
            .collect()
    }

    /// Whether `plugin_name` is usable for `user_id` this session. Fresh mode
    /// disables every plugin; remember mode defers to the persisted state.
    pub async fn session_active_plugin_enabled(&self, user_id: &str, plugin_name: &str) -> bool {
        if !self.session_remember(user_id).await {
            return false;
        }
        self.is_enabled_for(user_id, plugin_name).await
    }

    pub fn persona_concat_for(&self, active: &BTreeSet<String>) -> String {
        self.inner.contribs.read().iter()
            .filter(|c| active.contains(&c.manifest.name))
            .map(|c| c.persona.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn skills_markdown_for(&self, active: &BTreeSet<String>) -> String {
        let mut out = String::new();
        for contrib in self.inner.contribs.read().iter() {
            if !active.contains(&contrib.manifest.name) {
                continue;
            }
            if !contrib.skills_md.trim().is_empty() {
                out.push_str(&contrib.skills_md);
                out.push_str("\n\n---\n\n");
            }
        }
        out.push_str(&self.inner.tools.skills_markdown_for(active));
        out
    }

    /// One plugin's skills markdown (or empty). Used to inject a freshly
    /// activated plugin's tool docs into the running conversation.
    pub fn skills_for(&self, name: &str) -> String {
        self.inner.contribs.read().iter()
            .find(|c| c.manifest.name == name)
            .map(|c| c.skills_md.clone())
            .unwrap_or_default()
    }

    pub fn context_lines_for(&self, active: &BTreeSet<String>) -> Vec<String> {
        self.inner.contribs.read().iter()
            .filter(|c| active.contains(&c.manifest.name))
            .flat_map(|c| c.context_lines.clone())
            .collect()
    }

    /// List installed plugin manifests.
    pub fn list(&self) -> Vec<Manifest> {
        self.inner.loader.snapshot()
    }

    /// All installed plugins' routes: `(plugin_name, spec, handler)`.
    pub fn routes(&self) -> Vec<(String, RouteSpec, RouteHandler)> {
        let mut out = Vec::new();
        for contrib in self.inner.contribs.read().iter() {
            for (spec, handler) in &contrib.routes {
                out.push((contrib.manifest.name.clone(), spec.clone(), handler.clone()));
            }
        }
        out
    }

    /// Scan `plugins_dir` and install every directory containing `plugin.toml`.
    pub async fn discover_and_install(
        &self,
        base_ctx: Arc<PluginCtx>,
    ) -> Vec<String> {
        let mut installed: Vec<String> = Vec::new();
        let Some(entries) = std::fs::read_dir(&self.inner.plugins_dir).ok() else {
            return installed;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let manifest_path = path.join("plugin.toml");
            if !manifest_path.exists() {
                continue;
            }
            match self.install_dir_static(&path, base_ctx.clone()).await {
                Ok(n) => installed.push(n),
                Err(e) => tracing::warn!("Plugin discovery failed for {}: {}", path.display(), e),
            }
        }
        installed
    }

    pub async fn install_dir_static(
        &self,
        install_dir: &std::path::Path,
        base_ctx: Arc<PluginCtx>,
    ) -> Result<String, AppError> {
        let (manifest, builder, _ctx) = self.inner.loader.install_dir(install_dir, &self.inner.pool, base_ctx).await?;
        let plugin_name = manifest.name.clone();

        // Resolve every declared RouteSpec tag to a handler via the plugin.
        let mut routes: Vec<(RouteSpec, RouteHandler)> = Vec::new();
        for spec in &builder.routes {
            if let Some(h) = self.inner.loader.route_handler(&plugin_name, &spec.handler_tag) {
                routes.push((spec.clone(), h));
            }
        }

        {
            let mut contribs = self.inner.contribs.write();
            contribs.retain(|c| c.manifest.name != manifest.name);
            contribs.push(PluginContrib {
                manifest: manifest.clone(),
                skills_md: builder.skills_md.clone(),
                persona: builder.persona.clone(),
                context_lines: builder.context_lines.clone(),
                routes,
            });
        }
        for tool in builder.tools {
            self.inner.tools.install_owned(tool, &plugin_name);
        }
        self.inner.tools.attach_manager(self.clone());
        Ok(manifest.name)
    }

    pub fn uninstall(&self, name: &str) -> bool {
        self.inner.contribs.write().retain(|c| c.manifest.name != name);
        self.inner.loader.unload(name)
    }

    pub fn is_installed(&self, name: &str) -> bool {
        self.inner.loader.has(name)
    }

    /// Cached reference to the manager's pool — used by handlers that already
    /// operate on behalf of a user (`active_plugin_set` style helpers).
    pub fn pool(&self) -> &sqlx::SqlitePool {
        &self.inner.pool
    }
}