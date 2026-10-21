use std::sync::Arc;
use async_trait::async_trait;
use shiny_plugin_sdk::{
    manifest::Manifest,
    plugin::Plugin,
    services::PluginCtx,
    tools::RegistryBuilder,
};

pub struct KeyboardPlugin;

#[async_trait]
impl Plugin for KeyboardPlugin {
    fn manifest(&self) -> &Manifest {
        static M: std::sync::OnceLock<Manifest> = std::sync::OnceLock::new();
        M.get_or_init(|| {
            Manifest::new("keyboard")
                .description("Virtual multi-language keyboard at the bottom of the screen — types into any focused input")
                .summary("Virtual keyboard: types into any text input (8 language layouts)")
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
