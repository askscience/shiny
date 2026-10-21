use std::sync::Arc;
use async_trait::async_trait;
use shiny_plugin_sdk::{
    manifest::Manifest,
    plugin::Plugin,
    routes::{RouteHandler, RouteSpec},
    services::PluginCtx,
    tools::RegistryBuilder,
};

shiny_plugin_sdk::ctx_plugin!(RadioPlugin);

/// Persona fragment the agent system prompt sees when this plugin is active.
pub const PERSONA: &str = "a radio tuner AI; offer stations by name or genre";

#[async_trait]
impl Plugin for RadioPlugin {
    fn manifest(&self) -> &Manifest {
        static M: std::sync::OnceLock<Manifest> = std::sync::OnceLock::new();
        M.get_or_init(|| {
            Manifest::new("radio")
                .description("Internet radio via Radio Browser — search stations and play them")
                .summary("Internet radio: search and play stations")
        })
    }

    fn register(&self, ctx: Arc<PluginCtx>, builder: &mut RegistryBuilder<'_>) {
        self.install_ctx(ctx);
        builder
            .persona(PERSONA)
            .skills(include_str!("../skills/radio.md"))
            .context_line("Radio: enabled — the Radio window can tune internet radio stations.");
        builder.route(RouteSpec::get("/api/radio/nowplaying", "nowplaying"));
        builder.tools([
            Arc::new(crate::tools::RadioSearch) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::RadioPlay) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::RadioStop) as Arc<dyn shiny_plugin_sdk::tools::Tool>,]);
    }

    fn route_handler(&self, tag: &str) -> Option<RouteHandler> {
        self.routes_for(tag)
    }
}

