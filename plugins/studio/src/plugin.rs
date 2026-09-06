//! Studio plugin entry: manifest, tool/route registration, and the C entry point.

use std::sync::{Arc, OnceLock};
use async_trait::async_trait;
use shiny_plugin_sdk::{
    manifest::Manifest,
    plugin::{Plugin, PLUGIN_ENTRY_SYMBOL},
    routes::{HttpMethod, RouteHandler, RouteSpec},
    services::PluginCtx,
    tools::RegistryBuilder,
};

pub struct StudioPlugin {
    ctx: OnceLock<Arc<PluginCtx>>,
}

/// Persona fragment the agent system prompt sees when this plugin is active.
pub const PERSONA: &str =
    "a music studio AI; compose rhythmic patterns with exact (Euclidean) rhythms and multiple instruments, and render them to audio";

fn route_specs() -> Vec<RouteSpec> {
    vec![
        RouteSpec { method: HttpMethod::Get, path: "/api/studio".into(), auth: "auth".into(), handler_tag: "studio_list".into() },
        RouteSpec { method: HttpMethod::Post, path: "/api/studio".into(), auth: "auth".into(), handler_tag: "studio_create".into() },
        RouteSpec { method: HttpMethod::Get, path: "/api/studio/:id".into(), auth: "auth".into(), handler_tag: "studio_get".into() },
        RouteSpec { method: HttpMethod::Get, path: "/api/studio/:id/audio".into(), auth: "auth".into(), handler_tag: "studio_audio".into() },
        RouteSpec { method: HttpMethod::Put, path: "/api/studio/:id".into(), auth: "auth".into(), handler_tag: "studio_update".into() },
        RouteSpec { method: HttpMethod::Post, path: "/api/studio/:id/render".into(), auth: "auth".into(), handler_tag: "studio_render".into() },
        RouteSpec { method: HttpMethod::Post, path: "/api/studio/arrangement/render".into(), auth: "auth".into(), handler_tag: "studio_arrangement_render".into() },
        RouteSpec { method: HttpMethod::Post, path: "/api/studio/preview".into(), auth: "auth".into(), handler_tag: "studio_preview".into() },
        RouteSpec { method: HttpMethod::Post, path: "/api/studio/waveform".into(), auth: "auth".into(), handler_tag: "studio_waveform".into() },
        RouteSpec { method: HttpMethod::Get, path: "/api/studio/arrangement".into(), auth: "auth".into(), handler_tag: "studio_arrangement_list".into() },
        RouteSpec { method: HttpMethod::Post, path: "/api/studio/arrangement".into(), auth: "auth".into(), handler_tag: "studio_arrangement_save".into() },
        RouteSpec { method: HttpMethod::Get, path: "/api/studio/arrangement/:id".into(), auth: "auth".into(), handler_tag: "studio_arrangement_get".into() },
        RouteSpec { method: HttpMethod::Put, path: "/api/studio/arrangement/:id".into(), auth: "auth".into(), handler_tag: "studio_arrangement_update".into() },
        RouteSpec { method: HttpMethod::Delete, path: "/api/studio/arrangement/:id".into(), auth: "auth".into(), handler_tag: "studio_arrangement_delete".into() },
        RouteSpec { method: HttpMethod::Get, path: "/api/studio/presets".into(), auth: "auth".into(), handler_tag: "studio_preset_list".into() },
        RouteSpec { method: HttpMethod::Post, path: "/api/studio/presets".into(), auth: "auth".into(), handler_tag: "studio_preset_save".into() },
        RouteSpec { method: HttpMethod::Delete, path: "/api/studio/presets/:id".into(), auth: "auth".into(), handler_tag: "studio_preset_delete".into() },
        RouteSpec { method: HttpMethod::Delete, path: "/api/studio/:id".into(), auth: "auth".into(), handler_tag: "studio_delete".into() },
    ]
}

#[async_trait]
impl Plugin for StudioPlugin {
    fn manifest(&self) -> &Manifest {
        static M: std::sync::OnceLock<Manifest> = std::sync::OnceLock::new();
        M.get_or_init(|| Manifest {
            name: "studio".into(),
            version: semver::Version::new(0, 1, 0),
            api_level: 1,
            entry_symbol: PLUGIN_ENTRY_SYMBOL.into(),
            target_triple: None,
            description: Some(
                "Music studio — a trem-powered step sequencer and synth: compose patterns (Euclidean fills, explicit rhythms) and render them to audio in the Studio window".into(),
            ),
            author: Some("shiny".into()),
            summary: Some("Music studio: pattern sequencer + synth renderer (trem engine)".into()),
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
            .skills(include_str!("../skills/studio.md"))
            .context_line("Studio: enabled — compose and render patterns (Euclidean rhythms, multiple instruments) to audio in the Studio window.");
        for spec in route_specs() {
            builder.route(spec);
        }
        for tool in [
            Arc::new(crate::tools::StudioList) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::StudioCreate) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::StudioGet) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::StudioRender) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::StudioDelete) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::StudioPresetList) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::StudioPresetSave) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::StudioPresetDelete) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::StudioArrangementList) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::StudioArrangementSave) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::StudioArrangementGet) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::StudioArrangementDelete) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
        ] {
            builder.tool_arc(shiny_plugin_sdk::tools::bridged(tool));
        }
    }

    fn route_handler(&self, tag: &str) -> Option<RouteHandler> {
        let ctx = self.ctx.get()?;
        crate::routes::handle(ctx, tag)
    }
}

/// The C entry symbol the loader transmutes and calls.
#[no_mangle]
pub extern "C" fn shiny_plugin_entry() -> *mut dyn Plugin {
    Box::into_raw(Box::new(StudioPlugin { ctx: OnceLock::new() }))
}
