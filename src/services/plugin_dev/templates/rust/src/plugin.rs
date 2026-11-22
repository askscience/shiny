use std::sync::Arc;

use async_trait::async_trait;
use shiny_plugin_sdk::{
    manifest::Manifest,
    plugin::Plugin,
    routes::{RouteHandler, RouteSpec},
    services::PluginCtx,
    tools::{RegistryBuilder, Tool},
};

// Declares `@@TYPE@@` with `new()`, keeps the plugin context, and resolves a
// route tag through `crate::routes::handle`.
shiny_plugin_sdk::ctx_plugin!(@@TYPE@@);

#[async_trait]
impl Plugin for @@TYPE@@ {
    fn manifest(&self) -> &Manifest {
        static M: std::sync::OnceLock<Manifest> = std::sync::OnceLock::new();
        // Keep in step with plugin.toml (name, description, summary).
        M.get_or_init(|| {
            Manifest::new("@@NAME@@")
                .description("@@DESCRIPTION@@")
                .summary("@@TITLE@@: @@DESCRIPTION@@")
                .author("@@AUTHOR@@")
        })
    }

    fn register(&self, ctx: Arc<PluginCtx>, builder: &mut RegistryBuilder<'_>) {
        self.install_ctx(ctx);
        builder
            // The skill is what tells the assistant which tools exist and how
            // to call them: every tool registered below must be named in it.
            .skills(include_str!("../skills/@@NAME@@.md"))
            // Every route has a path and a tag `routes::handle` resolves.
            .routes([
                RouteSpec::get("/api/@@NAME@@/items", "item_list"),
                RouteSpec::post("/api/@@NAME@@/items", "item_create"),
            ])
            // `tools([...])` wraps each tool so it runs on the plugin's runtime.
            .tools([
                Arc::new(crate::tools::AddItem) as Arc<dyn Tool>,
                Arc::new(crate::tools::ListItems) as Arc<dyn Tool>,
            ]);
    }

    fn route_handler(&self, tag: &str) -> Option<RouteHandler> {
        self.routes_for(tag)
    }
}
