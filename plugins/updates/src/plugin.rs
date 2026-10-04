//! Plugin glue: manifest, contributions, entry symbol.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use shiny_plugin_sdk::{
    manifest::Manifest,
    plugin::{Plugin, PLUGIN_ENTRY_SYMBOL},
    routes::RouteHandler,
    services::PluginCtx,
    tools::{bridged, RegistryBuilder},
};

pub struct UpdatesPlugin {
    ctx: OnceLock<Arc<PluginCtx>>,
}

pub const PERSONA: &str =
    "an assistant that can check for and install Linux system and Ollama updates on the machine";

#[async_trait]
impl Plugin for UpdatesPlugin {
    fn manifest(&self) -> &Manifest {
        static M: OnceLock<Manifest> = OnceLock::new();
        M.get_or_init(|| Manifest {
            name: "updates".into(),
            version: semver::Version::new(0, 1, 0),
            api_level: 1,
            entry_symbol: PLUGIN_ENTRY_SYMBOL.into(),
            target_triple: None,
            description: Some(
                "Check and install system + Ollama updates across every major Linux distribution"
                    .into(),
            ),
            author: Some("shiny".into()),
            summary: Some("Updates: one window (and HUD chip) for system and Ollama updates".into()),
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
            .skills(include_str!("../skills/updates.md"))
            .context_line(
                "Updates: the Updates window and the top-bar update chip show pending system and \
                 Ollama updates.",
            );

        for spec in crate::routes::route_specs() {
            builder.route(spec);
        }

        builder
            .tool_arc(bridged(Arc::new(crate::tools::UpdatesStatusTool)))
            .tool_arc(bridged(Arc::new(crate::tools::UpdatesDistroTool)))
            .tool_arc(bridged(Arc::new(crate::tools::UpdatesRefreshTool)))
            .tool_arc(bridged(Arc::new(crate::tools::UpdatesApplyTool)))
            .tool_arc(bridged(Arc::new(crate::tools::OllamaUpdateTool)));
    }

    fn route_handler(&self, tag: &str) -> Option<RouteHandler> {
        let ctx = self.ctx.get()?;
        crate::routes::handle(ctx, tag)
    }
}

/// The C entry symbol the loader transmutes and calls.
#[no_mangle]
pub extern "C" fn shiny_plugin_entry() -> *mut dyn Plugin {
    Box::into_raw(Box::new(UpdatesPlugin {
        ctx: OnceLock::new(),
    }))
}
