//! Plugin glue: manifest, contributions, entry symbol.

use std::sync::Arc;

use async_trait::async_trait;
use shiny_plugin_sdk::{
    manifest::Manifest,
    plugin::Plugin,
    routes::RouteHandler,
    services::PluginCtx,
    tools::{bridged, RegistryBuilder},
};

shiny_plugin_sdk::ctx_plugin!(UpdatesPlugin);

pub const PERSONA: &str =
    "an assistant that can check for and install Linux system and Ollama updates on the machine";

#[async_trait]
impl Plugin for UpdatesPlugin {
    fn manifest(&self) -> &Manifest {
        static M: std::sync::OnceLock<Manifest> = std::sync::OnceLock::new();
        M.get_or_init(|| {
            Manifest::new("updates")
                .description("Check and install system + Ollama updates across every major Linux distribution")
                .summary("Updates: one window (and HUD chip) for system and Ollama updates")
        })
    }

    fn register(&self, ctx: Arc<PluginCtx>, builder: &mut RegistryBuilder<'_>) {
        self.install_ctx(ctx);

        builder
            .persona(PERSONA)
            .skills(include_str!("../skills/updates.md"))
            .context_line(
                "Updates: the Updates window and the top-bar update chip show pending system and \
                 Ollama updates.",
            );

        builder.routes(crate::routes::route_specs());

        builder
            .tool_arc(bridged(Arc::new(crate::tools::UpdatesStatusTool)))
            .tool_arc(bridged(Arc::new(crate::tools::UpdatesDistroTool)))
            .tool_arc(bridged(Arc::new(crate::tools::UpdatesRefreshTool)))
            .tool_arc(bridged(Arc::new(crate::tools::UpdatesApplyTool)))
            .tool_arc(bridged(Arc::new(crate::tools::OllamaUpdateTool)));
    }

    fn route_handler(&self, tag: &str) -> Option<RouteHandler> {
        self.routes_for(tag)
    }
}

