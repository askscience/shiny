use std::sync::Arc;
use async_trait::async_trait;
use shiny_plugin_sdk::{
    manifest::Manifest,
    plugin::{Plugin, PLUGIN_ENTRY_SYMBOL},
    services::PluginCtx,
    tools::RegistryBuilder,
};

pub struct TravelerPlugin;

/// Persona fragment the agent system prompt sees when this plugin is active.
/// Re-used via `read_skills_file()` so the value lives in skills/traveler-api-tools.md
/// alongside the markdown doc the LLM consumes. Kept short so the generic
/// "AI sphere" base prompt reads naturally when concatenated.
pub const PERSONA: &str =
    "a travel navigator AI; address the user by first name when it feels natural, \
     suggest places to visit, walks, and routes, and proactively offer to track \
     trips and diarize the day";

#[async_trait]
impl Plugin for TravelerPlugin {
    fn manifest(&self) -> &Manifest {
        static M: std::sync::OnceLock<Manifest> = std::sync::OnceLock::new();
        M.get_or_init(|| Manifest {
            name: "traveler".into(),
            version: semver::Version::new(0, 1, 0),
            api_level: 1,
            entry_symbol: PLUGIN_ENTRY_SYMBOL.into(),
            target_triple: None,
            description: Some("Trip tracking, GPS, diary, map and navigation tools".into()),
            author: None,
            summary: Some("Trip tracking, GPS, diary, map tools, navigation".into()),
            migrations_dir: "migrations".into(),
            skills_dir: "skills".into(),
            web_dir: "web".into(),
            signature: None,
        })
    }

    fn register(&self, _ctx: Arc<PluginCtx>, builder: &mut RegistryBuilder<'_>) {
        builder
            .persona(PERSONA)
            .skills(include_str!("../skills/traveler-api-tools.md"))
            .context_line("Map: enabled — OpenStreetMap background is active.");
        for tool in crate::tools::all_tools() {
            builder.tool_arc(shiny_plugin_sdk::tools::bridged(tool));
        }
    }
}

/// The C entry symbol the loader transmutes and calls.
#[no_mangle]
pub extern "C" fn shiny_plugin_entry() -> *mut dyn Plugin {
    Box::into_raw(Box::new(TravelerPlugin))
}