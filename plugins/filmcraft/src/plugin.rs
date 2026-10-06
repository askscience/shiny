//! Manifest, registration and the C entry symbol.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use shiny_plugin_sdk::{
    manifest::Manifest,
    plugin::{Plugin, PLUGIN_ENTRY_SYMBOL},
    routes::{HttpMethod, RouteHandler, RouteSpec},
    services::PluginCtx,
    tools::{bridged, RegistryBuilder},
};

/// Persona fragment the assistant sees while this plugin is active.
const PERSONA: &str = "a video editor: FilmCraft, with a non-linear timeline, colour grading, \
sound mixing, titles and export, reachable either through the open editor window or through a \
headless engine on this machine";

fn route_specs() -> Vec<RouteSpec> {
    vec![
        // The window's long-poll and answer routes.
        RouteSpec {
            method: HttpMethod::Get,
            path: "/api/filmcraft/next".into(),
            auth: "auth".into(),
            handler_tag: "relay_next".into(),
        },
        RouteSpec {
            method: HttpMethod::Post,
            path: "/api/filmcraft/result".into(),
            auth: "auth".into(),
            handler_tag: "relay_result".into(),
        },
        RouteSpec {
            method: HttpMethod::Get,
            path: "/api/filmcraft/status".into(),
            auth: "auth".into(),
            handler_tag: "relay_status".into(),
        },
        // FilmCraft's WebAudio worklet, which it loads from a document-relative
        // URL. Claimed at the site root so mounting the editor inside the
        // desktop shell does not lose audio. Public: addModule() cannot send
        // an Authorization header.
        RouteSpec {
            method: HttpMethod::Get,
            path: "/audio-worklet.js".into(),
            auth: "public".into(),
            handler_tag: "audio_worklet".into(),
        },
    ]
}

pub struct FilmCraftPlugin;

#[async_trait]
impl Plugin for FilmCraftPlugin {
    fn manifest(&self) -> &Manifest {
        static M: OnceLock<Manifest> = OnceLock::new();
        M.get_or_init(|| Manifest {
            name: "filmcraft".into(),
            version: semver::Version::new(0, 1, 0),
            api_level: 1,
            entry_symbol: PLUGIN_ENTRY_SYMBOL.into(),
            target_triple: None,
            description: Some(
                "FilmCraft — a non-linear video editor (cut, colour, sound, titles, captions, \
                 export) hosted as a window in the desktop"
                    .into(),
            ),
            author: Some("shiny".into()),
            summary: Some(
                "Video editor: the FilmCraft editor in a window, plus a headless engine for \
                 batch exports"
                    .into(),
            ),
            migrations_dir: "migrations".into(),
            skills_dir: "skills".into(),
            web_dir: "web".into(),
            signature: None,
        })
    }

    fn register(&self, _ctx: Arc<PluginCtx>, builder: &mut RegistryBuilder<'_>) {
        for spec in route_specs() {
            builder.route(spec);
        }
        builder
            .persona(PERSONA)
            .context_line(
                "FilmCraft: the editor window is a WebAssembly build of the FilmCraft app; the \
                 headless engine works on .fcproj files on this machine. They are separate sessions.",
            )
            .tool_arc(bridged(Arc::new(crate::tools::browser::FilmCommand)))
            .tool_arc(bridged(Arc::new(crate::tools::browser::FilmCommands)))
            .tool_arc(bridged(Arc::new(crate::tools::batch::FilmHeadless)))
            .tool_arc(bridged(Arc::new(crate::tools::batch::FilmExport)))
            .tool_arc(bridged(Arc::new(crate::tools::batch::FilmExportStatus)))
            .tool_arc(bridged(Arc::new(crate::tools::batch::FilmExportCancel)));
    }

    fn route_handler(&self, tag: &str) -> Option<RouteHandler> {
        crate::routes::handle(tag)
    }
}

/// C entry symbol the loader transmutes & calls.
#[no_mangle]
pub extern "C" fn shiny_plugin_entry() -> *mut dyn Plugin {
    Box::into_raw(Box::new(FilmCraftPlugin))
}