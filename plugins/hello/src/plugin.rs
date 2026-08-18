use std::sync::Arc;
use async_trait::async_trait;
use shiny_plugin_sdk::{
    manifest::Manifest,
    plugin::{Plugin, PLUGIN_ENTRY_SYMBOL},
    services::PluginCtx,
    tools::RegistryBuilder,
};

pub struct HelloPlugin;

#[async_trait]
impl Plugin for HelloPlugin {
    fn manifest(&self) -> &Manifest {
        static M: std::sync::OnceLock<Manifest> = std::sync::OnceLock::new();
        M.get_or_init(|| Manifest {
            name: "hello".into(),
            version: semver::Version::new(0, 1, 0),
            api_level: 1,
            entry_symbol: PLUGIN_ENTRY_SYMBOL.into(),
            target_triple: None,
            description: Some("A demo plugin that adds a single `hello` tool.".into()),
            author: Some("shiny demo".into()),
            summary: Some("Demo plugin: adds one `hello` agent tool.".into()),
            migrations_dir: "migrations".into(),
            skills_dir: "skills".into(),
            web_dir: "web".into(),
            signature: None,
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