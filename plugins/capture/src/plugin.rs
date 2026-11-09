//! Capture plugin entry: manifest, routes, tools, and the C entry point.

use std::sync::Arc;

use async_trait::async_trait;
use shiny_plugin_sdk::{
    manifest::Manifest,
    plugin::Plugin,
    routes::RouteHandler,
    services::PluginCtx,
    tools::{bridged, RegistryBuilder},
};

shiny_plugin_sdk::ctx_plugin!(CapturePlugin);

pub const PERSONA: &str =
    "a video producer AI: record the screen, go live, and edit videos by proposals the user approves, with HTML graphics in a consistent style";

#[async_trait]
impl Plugin for CapturePlugin {
    fn manifest(&self) -> &Manifest {
        static M: std::sync::OnceLock<Manifest> = std::sync::OnceLock::new();
        M.get_or_init(|| {
            Manifest::new("capture")
                .description("Screen recording, live streaming and an AI video editor")
                .summary("Capture: record the screen or a window with a webcam bubble, go live, edit with AI")
        })
    }

    fn register(&self, ctx: Arc<PluginCtx>, builder: &mut RegistryBuilder<'_>) {
        crate::store::ensure_schema(&ctx);
        self.install_ctx(ctx);
        builder
            .persona(PERSONA)
            .skills(include_str!("../skills/capture.md"))
            .context_line("Capture: enabled — screen/window recording with webcam, live streaming (YouTube, Twitch, Kick, Facebook, RTMP), and a video editor the assistant can drive (capture_* tools).");
        builder.routes(crate::routes::specs());
        for t in crate::tools::all() {
            builder.tool_arc(bridged(t));
        }
        // Probe the GPU encoders off the request path: each probe runs ffmpeg.
        std::thread::Builder::new()
            .name("capture-probe".into())
            .spawn(|| {
                let _ = crate::session::va_nodes();
            })
            .ok();
    }

    async fn on_unload(&self, _ctx: Arc<PluginCtx>) {
        crate::session::stop();
        crate::meter::stop_all();
        crate::camera::release();
    }

    fn route_handler(&self, tag: &str) -> Option<RouteHandler> {
        self.routes_for(tag)
    }
}
