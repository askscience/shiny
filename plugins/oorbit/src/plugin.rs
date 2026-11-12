use std::sync::Arc;

use async_trait::async_trait;
use shiny_plugin_sdk::{
    manifest::Manifest,
    plugin::Plugin,
    routes::{RouteHandler, RouteSpec},
    services::PluginCtx,
    tools::RegistryBuilder,
};

shiny_plugin_sdk::ctx_plugin!(OorbitPlugin);

/// Persona fragment the agent system prompt sees when this plugin is active.
pub const PERSONA: &str =
    "the chronicler of the user's Oorbit planet, a tiny world whose society grows slowly in real time";

fn route_specs() -> Vec<RouteSpec> {
    vec![RouteSpec::get("/api/oorbit/ping", "ping")]
}

#[async_trait]
impl Plugin for OorbitPlugin {
    fn manifest(&self) -> &Manifest {
        static M: std::sync::OnceLock<Manifest> = std::sync::OnceLock::new();
        M.get_or_init(|| {
            Manifest::new("oorbit")
                .description("Oorbit: a tiny planet whose society grows from the stone age to space, in real time, beside your friends' planets")
                .summary("Oorbit: a slow planet game")
        })
    }

    fn register(&self, ctx: Arc<PluginCtx>, builder: &mut RegistryBuilder<'_>) {
        self.install_ctx(ctx);
        builder
            .persona(PERSONA)
            .skills(include_str!("../skills/oorbit.md"))
            .context_line("Oorbit: enabled. The Oorbit window shows the user's planet (early prototype, no game state yet).");
        builder.routes(route_specs());
    }

    fn route_handler(&self, tag: &str) -> Option<RouteHandler> {
        self.routes_for(tag)
    }
}
