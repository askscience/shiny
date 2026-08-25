use std::sync::Arc;
use async_trait::async_trait;
use shiny_plugin_sdk::{
    manifest::Manifest,
    plugin::{Plugin, PLUGIN_ENTRY_SYMBOL},
    services::PluginCtx,
    tools::RegistryBuilder,
};

pub struct WordPlugin;

/// Persona fragment the agent system prompt sees when this plugin is active.
pub const PERSONA: &str =
    "a writer's assistant; draft, edit and manage the user's documents";

#[async_trait]
impl Plugin for WordPlugin {
    fn manifest(&self) -> &Manifest {
        static M: std::sync::OnceLock<Manifest> = std::sync::OnceLock::new();
        M.get_or_init(|| Manifest {
            name: "word".into(),
            version: semver::Version::new(0, 1, 0),
            api_level: 1,
            entry_symbol: PLUGIN_ENTRY_SYMBOL.into(),
            target_triple: None,
            description: Some(
                "Simple word processor — documents stored as open .odt files".into(),
            ),
            author: Some("shiny".into()),
            summary: Some("Word processor: create, edit and read .odt documents".into()),
            migrations_dir: "migrations".into(),
            skills_dir: "skills".into(),
            web_dir: "web".into(),
            signature: None,
        })
    }

    fn register(&self, _ctx: Arc<PluginCtx>, builder: &mut RegistryBuilder<'_>) {
        builder
            .persona(PERSONA)
            .skills(include_str!("../skills/word.md"))
            .context_line(
                "Word: enabled — the Word window edits documents stored as open .odt files.",
            );
        for tool in [
            Arc::new(crate::tools::DocCreate) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::DocWrite) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::DocEdit) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::DocAppend) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::DocRead) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::DocList) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::DocDelete) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
        ] {
            builder.tool_arc(shiny_plugin_sdk::tools::bridged(tool));
        }
    }
}

/// The C entry symbol the loader transmutes and calls.
#[no_mangle]
pub extern "C" fn shiny_plugin_entry() -> *mut dyn Plugin {
    Box::into_raw(Box::new(WordPlugin))
}