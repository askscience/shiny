use std::sync::Arc;
use async_trait::async_trait;
use shiny_plugin_sdk::{
    manifest::Manifest,
    plugin::{Plugin, PLUGIN_ENTRY_SYMBOL},
    services::PluginCtx,
    tools::RegistryBuilder,
};

pub struct CalcPlugin;

/// Persona fragment the agent system prompt sees when this plugin is active.
pub const PERSONA: &str =
    "a spreadsheet assistant; build, edit and compute with the user's spreadsheets";

#[async_trait]
impl Plugin for CalcPlugin {
    fn manifest(&self) -> &Manifest {
        static M: std::sync::OnceLock<Manifest> = std::sync::OnceLock::new();
        M.get_or_init(|| Manifest {
            name: "calc".into(),
            version: semver::Version::new(0, 1, 0),
            api_level: 1,
            entry_symbol: PLUGIN_ENTRY_SYMBOL.into(),
            target_triple: None,
            description: Some(
                "Spreadsheet calculator — sheets stored as open cell grids, editable in the Calc window".into(),
            ),
            author: Some("shiny".into()),
            summary: Some("Calc: create, edit and compute with spreadsheets".into()),
            migrations_dir: "migrations".into(),
            skills_dir: "skills".into(),
            web_dir: "web".into(),
            signature: None,
        })
    }

    fn register(&self, _ctx: Arc<PluginCtx>, builder: &mut RegistryBuilder<'_>) {
        builder
            .persona(PERSONA)
            .skills(include_str!("../skills/calc.md"))
            .context_line(
                "Calc: enabled — the Calc window edits spreadsheets stored as open cell grids.",
            );
        for tool in [
            Arc::new(crate::tools::CalcCreate) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::CalcWrite) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::CalcClear) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::CalcRead) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::CalcList) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::CalcDelete) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
        ] {
            builder.tool_arc(shiny_plugin_sdk::tools::bridged(tool));
        }
    }
}

/// The C entry symbol the loader transmutes and calls.
#[no_mangle]
pub extern "C" fn shiny_plugin_entry() -> *mut dyn Plugin {
    Box::into_raw(Box::new(CalcPlugin))
}
