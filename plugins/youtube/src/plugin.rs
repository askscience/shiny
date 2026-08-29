use std::sync::Arc;
use async_trait::async_trait;
use shiny_plugin_sdk::{
    manifest::Manifest,
    plugin::{Plugin, PLUGIN_ENTRY_SYMBOL},
    services::PluginCtx,
    tools::RegistryBuilder,
};

pub struct YoutubePlugin;

/// Persona fragment the agent system prompt sees when this plugin is active.
pub const PERSONA: &str = "a video assistant; find and play YouTube videos";

#[async_trait]
impl Plugin for YoutubePlugin {
    fn manifest(&self) -> &Manifest {
        static M: std::sync::OnceLock<Manifest> = std::sync::OnceLock::new();
        M.get_or_init(|| Manifest {
            name: "youtube".into(),
            version: semver::Version::new(0, 1, 0),
            api_level: 1,
            entry_symbol: PLUGIN_ENTRY_SYMBOL.into(),
            target_triple: None,
            description: Some(
                "Watch YouTube videos in the YouTube window — search and play videos with the AI".into(),
            ),
            author: Some("shiny".into()),
            summary: Some("YouTube: AI search and playback in the YouTube window".into()),
            migrations_dir: "migrations".into(),
            skills_dir: "skills".into(),
            web_dir: "web".into(),
            signature: None,
        })
    }

    fn register(&self, _ctx: Arc<PluginCtx>, builder: &mut RegistryBuilder<'_>) {
        builder
            .persona(PERSONA)
            .skills(include_str!("../skills/youtube.md"))
            .context_line("YouTube: enabled — the YouTube window plays videos the AI searches for.");
        for tool in [
            Arc::new(crate::tools::YoutubeSearch) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::YoutubePlay) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
        ] {
            builder.tool_arc(shiny_plugin_sdk::tools::bridged(tool));
        }
    }
}

/// The C entry symbol the loader transmutes and calls.
#[no_mangle]
pub extern "C" fn shiny_plugin_entry() -> *mut dyn Plugin {
    Box::into_raw(Box::new(YoutubePlugin))
}
