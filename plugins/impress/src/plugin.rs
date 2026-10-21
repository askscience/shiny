use std::sync::Arc;
use async_trait::async_trait;
use shiny_plugin_sdk::{
    manifest::Manifest,
    plugin::Plugin,
    routes::{RouteHandler, RouteSpec},
    services::PluginCtx,
    tools::RegistryBuilder,
};

shiny_plugin_sdk::ctx_plugin!(ImpressPlugin);

/// Persona fragment the agent system prompt sees when this plugin is active.
pub const PERSONA: &str =
    "a presentation designer; build modern, well-structured slide decks for the user";

fn route_specs() -> Vec<RouteSpec> {
    vec![
        RouteSpec::get("/api/presentations", "deck_list"),
        RouteSpec::post("/api/presentations", "deck_create"),
        RouteSpec::post("/api/presentations/import", "deck_import"),
        RouteSpec::get("/api/presentations/:id", "deck_get"),
        RouteSpec::put("/api/presentations/:id", "deck_save"),
        RouteSpec::delete("/api/presentations/:id", "deck_delete"),
        RouteSpec::get("/api/presentations/:id/export", "deck_export"),
    ]
}

#[async_trait]
impl Plugin for ImpressPlugin {
    fn manifest(&self) -> &Manifest {
        static M: std::sync::OnceLock<Manifest> = std::sync::OnceLock::new();
        M.get_or_init(|| {
            Manifest::new("impress")
                .description("Presentation builder — decks stored as slides, exported as open .odp files")
                .summary("Impress: build, edit and present OpenDocument (.odp) slide decks")
        })
    }

    fn register(&self, ctx: Arc<PluginCtx>, builder: &mut RegistryBuilder<'_>) {
        self.install_ctx(ctx);
        builder
            .persona(PERSONA)
            .skills(include_str!("../skills/impress.md"))
            .context_line(
                "Impress: enabled — the Impress window edits slide decks stored as open .odp files.",
            );
        builder.routes(route_specs());
        builder.tools([
            Arc::new(crate::tools::SlideCreate) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::SlideWrite) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::SlideEdit) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::SlideRead) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::SlideList) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::SlideDelete) as Arc<dyn shiny_plugin_sdk::tools::Tool>,]);
    }

    fn route_handler(&self, tag: &str) -> Option<RouteHandler> {
        self.routes_for(tag)
    }
}

