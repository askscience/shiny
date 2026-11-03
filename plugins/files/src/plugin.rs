//! Files plugin — a GNOME-style file browser over the user's real home.
//!
//! Self-contained: no core edits beyond the documented lifecycle hooks. Its
//! only table is `file_shares` (link sharing, see `share/mod.rs`). It ships
//! the window surface, its REST routes and its agent tools, and provisions each account's classic home
//! folders (`Desktop`, `Documents`, …) under `$HOME/.shiny/home/<user-id>/`.

use std::sync::Arc;

use async_trait::async_trait;
use shiny_plugin_sdk::{
    manifest::Manifest,
    plugin::Plugin,
    routes::{RouteHandler, RouteSpec},
    services::PluginCtx,
    tools::RegistryBuilder,
};

use crate::tools::{
    FileCopy, FileDelete, FileInfo, FileList, FileMkdir, FileMove, FileRead, FileRestore,
    FileSearch, FileWrite,
};

shiny_plugin_sdk::ctx_plugin!(FilesPlugin);

/// Persona fragment the agent system prompt sees when this plugin is active.
pub const PERSONA: &str = "a file manager; browse, preview and organize the user's home folders";

fn route_specs() -> Vec<RouteSpec> {
    vec![
        RouteSpec::get("/api/files/list", "files_list"),
        RouteSpec::get("/api/files/home", "files_home"),
        RouteSpec::get("/api/files/read", "files_read"),
        RouteSpec::get("/api/files/text", "files_text"),
        RouteSpec::get("/api/files/render", "files_render"),
        RouteSpec::get("/api/files/raw", "files_raw"),
        RouteSpec::get("/api/files/thumb", "files_thumb"),
        RouteSpec::get("/api/files/video-info", "files_video_info"),
        RouteSpec::get("/api/files/video-frame", "files_video_frame"),
        RouteSpec::get("/api/files/download", "files_download"),
        RouteSpec::get("/api/files/search", "files_search"),
        RouteSpec::get("/api/files/recent", "files_recent"),
        RouteSpec::post("/api/files/upload", "files_upload"),
        RouteSpec::post("/api/files/write", "files_write"),
        RouteSpec::post("/api/files/mkdir", "files_mkdir"),
        RouteSpec::post("/api/files/rename", "files_rename"),
        RouteSpec::post("/api/files/delete", "files_delete"),
        RouteSpec::post("/api/files/restore", "files_restore"),
        RouteSpec::post("/api/files/empty-trash", "files_empty_trash"),
        // Link sharing through the managed Gokapi server (src/share).
        RouteSpec::get("/api/files/share/status", "files_share_status"),
        RouteSpec::post("/api/files/share/install", "files_share_install"),
        RouteSpec::post("/api/files/share/setup", "files_share_setup"),
        RouteSpec::post("/api/files/share/disable", "files_share_disable"),
        RouteSpec::post("/api/files/share", "files_share_create"),
        RouteSpec::get("/api/files/share/job", "files_share_job"),
        RouteSpec::get("/api/files/shares", "files_share_list"),
        RouteSpec::post("/api/files/share/revoke", "files_share_revoke"),
    ]
}

#[async_trait]
impl Plugin for FilesPlugin {
    fn manifest(&self) -> &Manifest {
        static M: std::sync::OnceLock<Manifest> = std::sync::OnceLock::new();
        M.get_or_init(|| {
            Manifest::new("files")
                .description("File browser — browse, preview and manage your home folders")
                .summary("GNOME-style file browser: grid/list, thumbnails, quick preview, upload and a trash")
        })
    }

    fn register(&self, ctx: Arc<PluginCtx>, builder: &mut RegistryBuilder<'_>) {
        self.install_ctx(ctx);
        builder
            .persona(PERSONA)
            .skills(include_str!("../skills/files.md"))
            .context_line(
                "Files: enabled — browse the user's home folders (Desktop, Documents, \
                 Downloads, Music, Pictures, Public, Templates, Videos) with previews.",
            );
        builder.routes(route_specs());
        builder.tools([
            Arc::new(FileList) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(FileRead) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(FileWrite) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(FileMkdir) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(FileMove) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(FileCopy) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(FileDelete) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(FileRestore) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(FileSearch) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(FileInfo) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
        ]);
    }

    /// Create `file_shares` if its migration has not run, and bring the
    /// Gokapi server and its public gate back up when sharing was left enabled.
    ///
    /// `on_load` runs on the host executor: the supervisor's `tokio::spawn`
    /// must happen inside an `rt::spawn` job on the plugin runtime.
    async fn on_load(&self, ctx: Arc<PluginCtx>) {
        crate::share::store::ensure_schema(ctx.db());
        if crate::share::server::load_instance()
            .map(|i| i.enabled)
            .unwrap_or(false)
        {
            // Spawned onto the plugin runtime, not awaited in the job, so the
            // Funnel call never holds up the plugin's serial route worker.
            shiny_plugin_sdk::rt::spawn(async {
                tokio::spawn(crate::share::server::resume());
            });
        }
    }

    /// Stop the Gokapi child with the plugin.
    async fn on_unload(&self, _ctx: Arc<PluginCtx>) {
        let _ = shiny_plugin_sdk::rt::bridge(crate::share::server::stop_server()).await;
    }

    fn route_handler(&self, tag: &str) -> Option<RouteHandler> {
        self.routes_for(tag)
    }

    /// Provision this account's home folder tree. Runs on the plugin-owned
    /// runtime (via `rt::bridge`) so its `tokio::fs` work never touches the
    /// host runtime.
    async fn on_user_registered(&self, _ctx: Arc<PluginCtx>, user_id: &str) {
        let uid = user_id.to_string();
        let _ =
            shiny_plugin_sdk::rt::bridge(async move { crate::fs_util::ensure_home(&uid).await })
                .await;
    }
}
