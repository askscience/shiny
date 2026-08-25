use std::sync::Arc;
use async_trait::async_trait;
use shiny_plugin_sdk::{
    manifest::Manifest,
    plugin::{Plugin, PLUGIN_ENTRY_SYMBOL},
    services::PluginCtx,
    tools::RegistryBuilder,
};

pub struct KeyboardPlugin;

#[async_trait]
impl Plugin for KeyboardPlugin {
    fn manifest(&self) -> &Manifest {
        static M: std::sync::OnceLock<Manifest> = std::sync::OnceLock::new();
        M.get_or_init(|| Manifest {
            name: "keyboard".into(),
            version: semver::Version::new(0, 1, 0),
            api_level: 1,
            entry_symbol: PLUGIN_ENTRY_SYMBOL.into(),
            target_triple: None,
            description: Some(
                "Virtual multi-language keyboard at the bottom of the screen — types into any focused input".into(),
            ),
            author: Some("shiny".into()),
            summary: Some("Virtual keyboard: types into any text input (8 language layouts)".into()),
            migrations_dir: "migrations".into(),
            skills_dir: "skills".into(),
            web_dir: "web".into(),
            signature: None,
        })
    }

    /// The keyboard is a pure UI surface — deliberately no persona, no skills
    /// and no tools. The AI must never see it.
    fn register(&self, _ctx: Arc<PluginCtx>, _builder: &mut RegistryBuilder<'_>) {}
}

/// The C entry symbol the loader transmutes and calls.
#[no_mangle]
pub extern "C" fn shiny_plugin_entry() -> *mut dyn Plugin {
    Box::into_raw(Box::new(KeyboardPlugin))
}
