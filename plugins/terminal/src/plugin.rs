use std::sync::Arc;

use async_trait::async_trait;
use shiny_plugin_sdk::{
    manifest::Manifest,
    plugin::Plugin,
    routes::{RouteHandler, RouteSpec},
    services::PluginCtx,
    tools::{bridged, RegistryBuilder},
};

use crate::tool::TerminalExec;

shiny_plugin_sdk::ctx_plugin!(TerminalPlugin);

/// Persona line: the model must know the window exists *and* that it can run
/// commands there itself (`terminal_exec`), so "run ls" is answered with the
/// tool instead of an invented action.
pub const PERSONA: &str =
    "an assistant with a real Terminal window on the desktop, where it can run shell commands for the user and the user can type commands themselves";

fn route_specs() -> Vec<RouteSpec> {
    vec![
        RouteSpec::post("/api/terminal/sessions", "sessions"),
        RouteSpec::get("/api/terminal/stream", "stream"),
        RouteSpec::get("/api/terminal/events", "events"),
        RouteSpec::post("/api/terminal/input", "input"),
        RouteSpec::post("/api/terminal/resize", "resize"),
        RouteSpec::post("/api/terminal/close", "close"),
    ]
}

#[async_trait]
impl Plugin for TerminalPlugin {
    fn manifest(&self) -> &Manifest {
        static M: std::sync::OnceLock<Manifest> = std::sync::OnceLock::new();
        M.get_or_init(|| {
            Manifest::new("terminal")
                .description("A real Linux terminal — a PTY-backed login shell in its own window")
                .summary("Terminal: a real shell over a PTY, rendered with xterm.js")
        })
    }

    fn register(&self, ctx: Arc<PluginCtx>, builder: &mut RegistryBuilder<'_>) {
        self.install_ctx(ctx);
        builder
            .persona(PERSONA)
            .skills(include_str!("../skills/terminal.md"))
            .context_line(
                "Terminal: enabled — the assistant runs shell commands with terminal_exec; the user can also type in the Terminal window.",
            )
            .tool_arc(bridged(Arc::new(TerminalExec)));
        builder.routes(route_specs());
    }

    fn route_handler(&self, tag: &str) -> Option<RouteHandler> {
        self.routes_for(tag)
    }
}

