use std::sync::Arc;
use async_trait::async_trait;
use shiny_plugin_sdk::{
    manifest::Manifest,
    plugin::Plugin,
    routes::{RouteHandler, RouteSpec},
    services::PluginCtx,
    tools::RegistryBuilder,
};

shiny_plugin_sdk::ctx_plugin!(YoutubePlugin);

/// Persona fragment the agent system prompt sees when this plugin is active.
pub const PERSONA: &str = "a video assistant; find and play YouTube videos";

#[async_trait]
impl Plugin for YoutubePlugin {
    fn manifest(&self) -> &Manifest {
        static M: std::sync::OnceLock<Manifest> = std::sync::OnceLock::new();
        M.get_or_init(|| {
            Manifest::new("youtube")
                .description("Watch YouTube videos in the YouTube window — search and play videos with the AI")
                .summary("YouTube: AI search and playback in the YouTube window")
        })
    }

    fn register(&self, ctx: Arc<PluginCtx>, builder: &mut RegistryBuilder<'_>) {
        self.install_ctx(ctx);
        builder
            .persona(PERSONA)
            .skills(include_str!("../skills/youtube.md"))
            .context_line("YouTube: enabled — the YouTube window plays videos the AI searches for.");
        builder.route(RouteSpec::get("/api/youtube/search", "yt_search"));
        builder.route(RouteSpec::get("/api/youtube/suggest", "yt_suggest"));
        builder.route(RouteSpec::get("/api/youtube/categories", "yt_categories"));
        builder.route(RouteSpec::get("/api/youtube/home", "yt_home"));
        builder.tools([
            Arc::new(crate::tools::YoutubeSearch) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::YoutubePlay) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::YoutubeSuggest) as Arc<dyn shiny_plugin_sdk::tools::Tool>,]);
    }

    fn route_handler(&self, tag: &str) -> Option<RouteHandler> {
        self.routes_for(tag)
    }
}

