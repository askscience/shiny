use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use shiny_plugin_sdk::{
    manifest::Manifest,
    plugin::{Plugin, PLUGIN_ENTRY_SYMBOL},
    routes::{HttpMethod, RouteHandler, RouteSpec},
    services::PluginCtx,
    tools::RegistryBuilder,
};

pub struct TerminalPlugin {
    ctx: OnceLock<Arc<PluginCtx>>,
}

/// No agent tools: the terminal is a human-facing surface. The persona line
/// only tells the model the window exists so it can point the user at it.
pub const PERSONA: &str =
    "an assistant with a real Terminal window on the desktop; the user can type shell commands into it directly";

fn route_specs() -> Vec<RouteSpec> {
    vec![
        RouteSpec {
            method: HttpMethod::Post,
            path: "/api/terminal/sessions".into(),
            auth: "auth".into(),
            handler_tag: "sessions".into(),
        },
        RouteSpec {
            method: HttpMethod::Get,
            path: "/api/terminal/stream".into(),
            auth: "auth".into(),
            handler_tag: "stream".into(),
        },
        RouteSpec {
            method: HttpMethod::Post,
            path: "/api/terminal/input".into(),
            auth: "auth".into(),
            handler_tag: "input".into(),
        },
        RouteSpec {
            method: HttpMethod::Post,
            path: "/api/terminal/resize".into(),
            auth: "auth".into(),
            handler_tag: "resize".into(),
        },
        RouteSpec {
            method: HttpMethod::Post,
            path: "/api/terminal/close".into(),
            auth: "auth".into(),
            handler_tag: "close".into(),
        },
    ]
}

#[async_trait]
impl Plugin for TerminalPlugin {
    fn manifest(&self) -> &Manifest {
        static M: std::sync::OnceLock<Manifest> = std::sync::OnceLock::new();
        M.get_or_init(|| Manifest {
            name: "terminal".into(),
            version: semver::Version::new(0, 1, 0),
            api_level: 1,
            entry_symbol: PLUGIN_ENTRY_SYMBOL.into(),
            target_triple: None,
            description: Some(
                "A real Linux terminal — a PTY-backed login shell in its own window".into(),
            ),
            author: Some("askscience".into()),
            summary: Some("Terminal: a real shell over a PTY, rendered with xterm.js".into()),
            migrations_dir: "migrations".into(),
            skills_dir: "skills".into(),
            web_dir: "web".into(),
            signature: None,
        })
    }

    fn register(&self, ctx: Arc<PluginCtx>, builder: &mut RegistryBuilder<'_>) {
        let _ = self.ctx.set(ctx);
        builder
            .persona(PERSONA)
            .skills(include_str!("../skills/terminal.md"))
            .context_line("Terminal: enabled — a real shell window is available on the desktop.");
        for spec in route_specs() {
            builder.route(spec);
        }
    }

    fn route_handler(&self, tag: &str) -> Option<RouteHandler> {
        let ctx = self.ctx.get()?;
        crate::routes::handle(ctx, tag)
    }
}

/// The C entry symbol the loader transmutes and calls.
#[no_mangle]
pub extern "C" fn shiny_plugin_entry() -> *mut dyn Plugin {
    Box::into_raw(Box::new(TerminalPlugin { ctx: OnceLock::new() }))
}
