//! Plugin glue: manifest, live-only registration and route dispatch.

use std::sync::Arc;

use async_trait::async_trait;
use shiny_plugin_sdk::{
    manifest::Manifest, plugin::Plugin, routes::RouteHandler, services::PluginCtx,
    tools::RegistryBuilder,
};

shiny_plugin_sdk::ctx_plugin!(InstallerPlugin);

#[async_trait]
impl Plugin for InstallerPlugin {
    fn manifest(&self) -> &Manifest {
        static M: std::sync::OnceLock<Manifest> = std::sync::OnceLock::new();
        M.get_or_init(|| {
            Manifest::new("installer")
                .description("Install PEAK'D! to a disk from the live session")
                .summary("Live installer: choose a disk and profile, watch the install")
                .author("shiny")
        })
    }

    fn register(&self, ctx: Arc<PluginCtx>, builder: &mut RegistryBuilder<'_>) {
        self.install_ctx(ctx);
        if !live_mode() {
            return;
        }

        builder.routes(crate::routes::route_specs()).context_line(
            "Installer: the live PEAK'D! installer is open; it writes a disk only after \
                 the user confirms a typed plan and the privileged helper validates it.",
        );
    }

    fn route_handler(&self, tag: &str) -> Option<RouteHandler> {
        self.routes_for(tag)
    }
}

/// The installer is inert outside a live session.
fn live_mode() -> bool {
    std::env::var("SHINY_LIVE_INSTALLER")
        .map(|value| value == "1")
        .unwrap_or(false)
}
