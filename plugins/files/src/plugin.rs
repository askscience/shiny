//! Files plugin — a GNOME-style file browser over the user's real home.
//!
//! Self-contained: no database, no core edits beyond the documented
//! `on_user_registered` lifecycle hook. It ships the window surface, its REST
//! routes and its agent tools, and provisions each account's classic home
//! folders (`Desktop`, `Documents`, …) under `$HOME/.shiny/home/<user-id>/`.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use shiny_plugin_sdk::{
    manifest::Manifest,
    plugin::{Plugin, PLUGIN_ENTRY_SYMBOL},
    routes::{HttpMethod, RouteHandler, RouteSpec},
    services::PluginCtx,
    tools::RegistryBuilder,
};

use crate::tools::{
    FileCopy, FileDelete, FileInfo, FileList, FileMkdir, FileMove, FileRead, FileRestore,
    FileSearch, FileWrite,
};

pub struct FilesPlugin {
    ctx: OnceLock<Arc<PluginCtx>>,
}

/// Persona fragment the agent system prompt sees when this plugin is active.
pub const PERSONA: &str =
    "a file manager; browse, preview and organize the user's home folders";

fn route_specs() -> Vec<RouteSpec> {
    let auth = "auth".to_string();
    let spec = |method: HttpMethod, path: &str, tag: &str| RouteSpec {
        method,
        path: path.into(),
        auth: auth.clone(),
        handler_tag: tag.into(),
    };
    vec![
        spec(HttpMethod::Get, "/api/files/list", "files_list"),
        spec(HttpMethod::Get, "/api/files/home", "files_home"),
        spec(HttpMethod::Get, "/api/files/read", "files_read"),
        spec(HttpMethod::Get, "/api/files/text", "files_text"),
        spec(HttpMethod::Get, "/api/files/render", "files_render"),
        spec(HttpMethod::Get, "/api/files/raw", "files_raw"),
        spec(HttpMethod::Get, "/api/files/thumb", "files_thumb"),
        spec(HttpMethod::Get, "/api/files/download", "files_download"),
        spec(HttpMethod::Get, "/api/files/search", "files_search"),
        spec(HttpMethod::Post, "/api/files/upload", "files_upload"),
        spec(HttpMethod::Post, "/api/files/write", "files_write"),
        spec(HttpMethod::Post, "/api/files/mkdir", "files_mkdir"),
        spec(HttpMethod::Post, "/api/files/rename", "files_rename"),
        spec(HttpMethod::Post, "/api/files/delete", "files_delete"),
        spec(HttpMethod::Post, "/api/files/restore", "files_restore"),
        spec(HttpMethod::Post, "/api/files/empty-trash", "files_empty_trash"),
    ]
}

#[async_trait]
impl Plugin for FilesPlugin {
    fn manifest(&self) -> &Manifest {
        static M: std::sync::OnceLock<Manifest> = std::sync::OnceLock::new();
        M.get_or_init(|| Manifest {
            name: "files".into(),
            version: semver::Version::new(0, 1, 0),
            api_level: 1,
            entry_symbol: PLUGIN_ENTRY_SYMBOL.into(),
            target_triple: None,
            description: Some(
                "File browser — browse, preview and manage your home folders".into(),
            ),
            author: Some("shiny".into()),
            summary: Some(
                "GNOME-style file browser: grid/list, thumbnails, quick preview, upload and a trash"
                    .into(),
            ),
            migrations_dir: "migrations".into(),
            skills_dir: "skills".into(),
            web_dir: "web".into(),
            signature: None,
        })
    }

    fn register(&self, ctx: Arc<PluginCtx>, builder: &mut RegistryBuilder<'_>) {
        let _ = self.ctx.set(ctx);
        builder
            .persona(PERSONA)
            .skills(include_str!("../skills/files.md"))
            .context_line(
                "Files: enabled — browse the user's home folders (Desktop, Documents, \
                 Downloads, Music, Pictures, Public, Templates, Videos) with previews.",
            );
        for spec in route_specs() {
            builder.route(spec);
        }
        for tool in [
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
        ] {
            builder.tool_arc(shiny_plugin_sdk::tools::bridged(tool));
        }
    }

    fn route_handler(&self, tag: &str) -> Option<RouteHandler> {
        let ctx = self.ctx.get()?;
        crate::routes::handle(ctx, tag)
    }

    /// Provision this account's home folder tree. Runs on the plugin-owned
    /// runtime (via `rt::bridge`) so its `tokio::fs` work never touches the
    /// host runtime.
    async fn on_user_registered(&self, _ctx: Arc<PluginCtx>, user_id: &str) {
        let uid = user_id.to_string();
        let _ = shiny_plugin_sdk::rt::bridge(async move {
            crate::fs_util::ensure_home(&uid).await
        })
        .await;
    }
}

/// The C entry symbol the loader transmutes and calls.
#[no_mangle]
pub extern "C" fn shiny_plugin_entry() -> *mut dyn Plugin {
    Box::into_raw(Box::new(FilesPlugin { ctx: OnceLock::new() }))
}
