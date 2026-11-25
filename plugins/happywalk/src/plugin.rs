use std::sync::Arc;

use async_trait::async_trait;
use shiny_plugin_sdk::{
    manifest::Manifest,
    plugin::Plugin,
    routes::{RouteHandler, RouteSpec},
    services::PluginCtx,
    tools::RegistryBuilder,
};

shiny_plugin_sdk::ctx_plugin!(HappyWalkPlugin);

fn route_specs() -> Vec<RouteSpec> {
    vec![
        RouteSpec::get("/api/happywalk/ping", "ping"),
        RouteSpec::post("/api/happywalk/brain", "brain"),
    ]
}

#[async_trait]
impl Plugin for HappyWalkPlugin {
    fn manifest(&self) -> &Manifest {
        static M: std::sync::OnceLock<Manifest> = std::sync::OnceLock::new();
        M.get_or_init(|| {
            Manifest::new("happywalk")
                .description("Happy Walk: walk a character through a pastel voxel city using only your voice; it knows it is being steered, and says so")
                .summary("Happy Walk: a walk steered by voice")
        })
    }

    fn register(&self, ctx: Arc<PluginCtx>, builder: &mut RegistryBuilder<'_>) {
        self.install_ctx(ctx);
        // The game is played in its window, by voice: the assistant has
        // nothing to do in it.
        builder.persona("");
        builder.routes(route_specs());
    }

    fn route_handler(&self, tag: &str) -> Option<RouteHandler> {
        self.routes_for(tag)
    }
}
