use std::sync::Arc;
use async_trait::async_trait;
use shiny_plugin_sdk::{
    manifest::Manifest,
    plugin::Plugin,
    routes::{RouteHandler, RouteSpec},
    services::PluginCtx,
    tools::RegistryBuilder,
};

shiny_plugin_sdk::ctx_plugin!(CalculatorPlugin);

/// Persona fragment the agent system prompt sees when this plugin is active.
pub const PERSONA: &str =
    "a calculator AI; evaluate arithmetic and scientific math expressions for the user";

fn route_specs() -> Vec<RouteSpec> {
    vec![
        RouteSpec::post("/api/calculator/eval", "eval"),
        RouteSpec::get("/api/calculator/history", "history_list"),
        RouteSpec::delete("/api/calculator/history", "history_clear"),
    ]
}

#[async_trait]
impl Plugin for CalculatorPlugin {
    fn manifest(&self) -> &Manifest {
        static M: std::sync::OnceLock<Manifest> = std::sync::OnceLock::new();
        M.get_or_init(|| {
            Manifest::new("calculator")
                .description("Scientific calculator — evaluate arithmetic and scientific expressions in the Calculator window")
                .summary("Calculator: basic and scientific math")
        })
    }

    fn register(&self, ctx: Arc<PluginCtx>, builder: &mut RegistryBuilder<'_>) {
        self.install_ctx(ctx);
        builder
            .persona(PERSONA)
            .skills(include_str!("../skills/calculator.md"))
            .context_line("Calculator: enabled — the Calculator window evaluates basic and scientific expressions.");
        builder.routes(route_specs());
        builder.tools([
            Arc::new(crate::tools::CalculatorEval) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::CalculatorHistory) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::CalculatorClearHistory) as Arc<dyn shiny_plugin_sdk::tools::Tool>,]);
    }

    fn route_handler(&self, tag: &str) -> Option<RouteHandler> {
        self.routes_for(tag)
    }
}

