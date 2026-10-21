use std::sync::Arc;
use async_trait::async_trait;
use shiny_plugin_sdk::{
    manifest::Manifest,
    plugin::Plugin,
    routes::{RouteHandler, RouteSpec},
    services::PluginCtx,
    tools::RegistryBuilder,
};

shiny_plugin_sdk::ctx_plugin!(WordPlugin);

/// Persona fragment the agent system prompt sees when this plugin is active.
pub const PERSONA: &str =
    "a writer's assistant; draft, edit and manage the user's documents";

fn route_specs() -> Vec<RouteSpec> {
    vec![
        RouteSpec::get("/api/documents", "doc_list"),
        RouteSpec::post("/api/documents", "doc_create"),
        RouteSpec::post("/api/documents/import", "doc_import"),
        RouteSpec::get("/api/documents/:id", "doc_get"),
        RouteSpec::put("/api/documents/:id", "doc_save"),
        RouteSpec::delete("/api/documents/:id", "doc_delete"),
        RouteSpec::get("/api/documents/:id/export", "doc_export"),
    ]
}

#[async_trait]
impl Plugin for WordPlugin {
    fn manifest(&self) -> &Manifest {
        static M: std::sync::OnceLock<Manifest> = std::sync::OnceLock::new();
        M.get_or_init(|| {
            Manifest::new("word")
                .description("Simple word processor — documents stored as open .odt files")
                .summary("Word processor: create, edit and read .odt documents")
        })
    }

    fn register(&self, ctx: Arc<PluginCtx>, builder: &mut RegistryBuilder<'_>) {
        self.install_ctx(ctx);
        builder
            .persona(PERSONA)
            .skills(include_str!("../skills/word.md"))
            .context_line(
                "Word: enabled — the Word window edits documents stored as open .odt files.",
            );
        builder.routes(route_specs());
        builder.tools([
            Arc::new(crate::tools::DocCreate) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::DocWrite) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::DocEdit) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::DocAppend) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::DocRead) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::DocList) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::DocDelete) as Arc<dyn shiny_plugin_sdk::tools::Tool>,]);
    }

    fn route_handler(&self, tag: &str) -> Option<RouteHandler> {
        self.routes_for(tag)
    }
}

