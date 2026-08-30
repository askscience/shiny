use std::sync::{Arc, OnceLock};
use async_trait::async_trait;
use shiny_plugin_sdk::{
    manifest::Manifest,
    plugin::{Plugin, PLUGIN_ENTRY_SYMBOL},
    routes::{HttpMethod, RouteHandler, RouteSpec},
    services::PluginCtx,
    tools::RegistryBuilder,
};

pub struct ImpressPlugin {
    ctx: OnceLock<Arc<PluginCtx>>,
}

/// Persona fragment the agent system prompt sees when this plugin is active.
pub const PERSONA: &str =
    "a presentation designer; build modern, well-structured slide decks for the user";

fn route_specs() -> Vec<RouteSpec> {
    vec![
        RouteSpec { method: HttpMethod::Get, path: "/api/presentations".into(), auth: "auth".into(), handler_tag: "deck_list".into() },
        RouteSpec { method: HttpMethod::Post, path: "/api/presentations".into(), auth: "auth".into(), handler_tag: "deck_create".into() },
        RouteSpec { method: HttpMethod::Post, path: "/api/presentations/import".into(), auth: "auth".into(), handler_tag: "deck_import".into() },
        RouteSpec { method: HttpMethod::Get, path: "/api/presentations/:id".into(), auth: "auth".into(), handler_tag: "deck_get".into() },
        RouteSpec { method: HttpMethod::Put, path: "/api/presentations/:id".into(), auth: "auth".into(), handler_tag: "deck_save".into() },
        RouteSpec { method: HttpMethod::Delete, path: "/api/presentations/:id".into(), auth: "auth".into(), handler_tag: "deck_delete".into() },
        RouteSpec { method: HttpMethod::Get, path: "/api/presentations/:id/export".into(), auth: "auth".into(), handler_tag: "deck_export".into() },
    ]
}

#[async_trait]
impl Plugin for ImpressPlugin {
    fn manifest(&self) -> &Manifest {
        static M: std::sync::OnceLock<Manifest> = std::sync::OnceLock::new();
        M.get_or_init(|| Manifest {
            name: "impress".into(),
            version: semver::Version::new(0, 1, 0),
            api_level: 1,
            entry_symbol: PLUGIN_ENTRY_SYMBOL.into(),
            target_triple: None,
            description: Some(
                "Presentation builder — decks stored as slides, exported as open .odp files".into(),
            ),
            author: Some("shiny".into()),
            summary: Some("Impress: build, edit and present OpenDocument (.odp) slide decks".into()),
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
            .skills(include_str!("../skills/impress.md"))
            .context_line(
                "Impress: enabled — the Impress window edits slide decks stored as open .odp files.",
            );
        for spec in route_specs() {
            builder.route(spec);
        }
        for tool in [
            Arc::new(crate::tools::SlideCreate) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::SlideWrite) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::SlideEdit) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::SlideRead) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::SlideList) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::SlideDelete) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
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
    Box::into_raw(Box::new(ImpressPlugin { ctx: OnceLock::new() }))
}
