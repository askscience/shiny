use std::sync::Arc;
use async_trait::async_trait;
use shiny_plugin_sdk::{
    manifest::Manifest,
    plugin::Plugin,
    routes::{RouteHandler, RouteSpec},
    services::PluginCtx,
    tools::RegistryBuilder,
};

shiny_plugin_sdk::ctx_plugin!(ImagePlugin);

/// Persona fragment the agent system prompt sees when this plugin is active.
pub const PERSONA: &str =
    "a photo editor AI; tune pictures with adjustments, film looks and effects";

fn route_specs() -> Vec<RouteSpec> {
    vec![
        RouteSpec::get("/api/images", "image_list"),
        RouteSpec::post("/api/images", "image_create"),
        RouteSpec::get("/api/images/:id", "image_get"),
        RouteSpec::get("/api/images/:id/data", "image_data"),
        RouteSpec::post("/api/images/:id/apply", "image_apply"),
        RouteSpec::get("/api/images/:id/render", "image_render"),
        RouteSpec::get("/api/images/:id/looks", "image_looks"),
        RouteSpec::delete("/api/images/:id", "image_delete"),
    ]
}

#[async_trait]
impl Plugin for ImagePlugin {
    fn manifest(&self) -> &Manifest {
        static M: std::sync::OnceLock<Manifest> = std::sync::OnceLock::new();
        M.get_or_init(|| {
            Manifest::new("image")
                .description("Photo editor — tone and colour adjustments, blur/sharpen/vignette and one-tap film looks in the Image window")
                .summary("Photo editor: adjustments and film looks")
        })
    }

    fn register(&self, ctx: Arc<PluginCtx>, builder: &mut RegistryBuilder<'_>) {
        self.install_ctx(ctx);
        builder
            .persona(PERSONA)
            .skills(include_str!("../skills/image.md"))
            .context_line("Image: enabled — the Image window opens pictures and edits them from a bottom dock of adjustments and looks.");
        builder.routes(route_specs());
        builder.tools([
            Arc::new(crate::tools::ImageList) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::ImageGet) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::ImageEdit) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::ImageDelete) as Arc<dyn shiny_plugin_sdk::tools::Tool>,]);
    }

    fn route_handler(&self, tag: &str) -> Option<RouteHandler> {
        self.routes_for(tag)
    }
}

