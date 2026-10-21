use std::sync::Arc;
use async_trait::async_trait;
use shiny_plugin_sdk::{
    manifest::Manifest,
    plugin::Plugin,
    services::PluginCtx,
    tools::RegistryBuilder,
};

pub struct HelloPlugin;

#[async_trait]
impl Plugin for HelloPlugin {
    fn manifest(&self) -> &Manifest {
        static M: std::sync::OnceLock<Manifest> = std::sync::OnceLock::new();
        M.get_or_init(|| {
            Manifest::new("hello")
                .description("A demo plugin that adds a single `hello` tool.")
                .summary("Demo plugin: adds one `hello` agent tool.")
                .author("shiny demo")
        })
    }

    fn register(&self, _ctx: Arc<PluginCtx>, builder: &mut RegistryBuilder<'_>) {
        builder
            .persona("") // hello plugin adds no persona fragment.
            .skills("- `hello` — Say hello to someone. params: `{ name?: string }`")
            .tool_arc(shiny_plugin_sdk::tools::bridged(std::sync::Arc::new(crate::tool::HelloTool)));
    }
}

/// C entry symbol the loader transmutes & calls.
#[no_mangle]
pub extern "C" fn shiny_plugin_entry() -> *mut dyn Plugin {
    Box::into_raw(Box::new(HelloPlugin))
}