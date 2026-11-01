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
        RouteSpec::put("/api/documents/:id/design", "doc_design"),
        RouteSpec::get("/api/documents/:id/pages", "doc_pages"),
        RouteSpec::post("/api/documents/:id/opened", "doc_opened"),
        RouteSpec::get("/api/documents/:id/page/:n", "doc_page"),
        RouteSpec::get("/api/word-fonts/:name", "doc_font").public(),
    ]
}

#[async_trait]
impl Plugin for WordPlugin {
    fn manifest(&self) -> &Manifest {
        static M: std::sync::OnceLock<Manifest> = std::sync::OnceLock::new();
        M.get_or_init(|| {
            Manifest::new("word")
                .description("Word processor — open .odt documents with Word (.docx) and PDF export")
                .summary("Word processor: write .odt documents, import Word files, export polished .docx and PDF")
        })
    }

    fn register(&self, ctx: Arc<PluginCtx>, builder: &mut RegistryBuilder<'_>) {
        self.install_ctx(ctx);
        builder
            .persona(PERSONA)
            .skills(include_str!("../skills/word.md"))
            .context_line(
                "Word: enabled — the Word window edits documents stored as open .odt files; they export as .docx or PDF with a design (doc_design, doc_export).",
            );
        builder.routes(route_specs());
        builder.tools([
            Arc::new(crate::tools::DocCreate) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::DocWrite) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::DocEdit) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::DocAppend) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::DocRead) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::DocList) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::DocDelete) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::DocDesign) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::DocExport) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::DocInsertImage) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
        ]);
    }

    fn route_handler(&self, tag: &str) -> Option<RouteHandler> {
        self.routes_for(tag)
    }
}

