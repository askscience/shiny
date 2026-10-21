use std::sync::Arc;
use async_trait::async_trait;
use shiny_plugin_sdk::{
    manifest::Manifest,
    plugin::Plugin,
    routes::{RouteHandler, RouteSpec},
    services::PluginCtx,
    tools::RegistryBuilder,
};

shiny_plugin_sdk::ctx_plugin!(CalcPlugin);

/// Persona fragment the agent system prompt sees when this plugin is active.
pub const PERSONA: &str =
    "a spreadsheet assistant; build, edit and compute with the user's spreadsheets";

fn route_specs() -> Vec<RouteSpec> {
    vec![
        RouteSpec::get("/api/spreadsheets", "sheet_list"),
        RouteSpec::post("/api/spreadsheets", "sheet_create"),
        RouteSpec::post("/api/spreadsheets/import", "sheet_import"),
        RouteSpec::get("/api/spreadsheets/:id", "sheet_get"),
        RouteSpec::put("/api/spreadsheets/:id", "sheet_save"),
        RouteSpec::delete("/api/spreadsheets/:id", "sheet_delete"),
        RouteSpec::get("/api/spreadsheets/:id/export", "sheet_export"),
    ]
}

#[async_trait]
impl Plugin for CalcPlugin {
    fn manifest(&self) -> &Manifest {
        static M: std::sync::OnceLock<Manifest> = std::sync::OnceLock::new();
        M.get_or_init(|| {
            Manifest::new("calc")
                .description("Spreadsheet calculator — sheets stored as open cell grids, editable in the Calc window")
                .summary("Calc: create, edit and compute with spreadsheets")
        })
    }

    fn register(&self, ctx: Arc<PluginCtx>, builder: &mut RegistryBuilder<'_>) {
        self.install_ctx(ctx);
        builder
            .persona(PERSONA)
            .skills(include_str!("../skills/calc.md"))
            .context_line(
                "Calc: enabled — the Calc window edits spreadsheets stored as open cell grids.",
            );
        builder.routes(route_specs());
        builder.tools([
            Arc::new(crate::tools::CalcCreate) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::CalcWrite) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::CalcClear) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::CalcRead) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::CalcList) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::CalcDelete) as Arc<dyn shiny_plugin_sdk::tools::Tool>,]);
    }

    fn route_handler(&self, tag: &str) -> Option<RouteHandler> {
        self.routes_for(tag)
    }
}

