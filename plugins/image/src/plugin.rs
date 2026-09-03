use std::sync::{Arc, OnceLock};
use async_trait::async_trait;
use shiny_plugin_sdk::{
    manifest::Manifest,
    plugin::{Plugin, PLUGIN_ENTRY_SYMBOL},
    routes::{HttpMethod, RouteHandler, RouteSpec},
    services::PluginCtx,
    tools::RegistryBuilder,
};

pub struct ImagePlugin {
    ctx: OnceLock<Arc<PluginCtx>>,
}

/// Persona fragment the agent system prompt sees when this plugin is active.
pub const PERSONA: &str =
    "an image editor AI; apply photographic effects, filters and transforms to the user's images";

fn route_specs() -> Vec<RouteSpec> {
    vec![
        RouteSpec { method: HttpMethod::Get, path: "/api/images".into(), auth: "auth".into(), handler_tag: "image_list".into() },
        RouteSpec { method: HttpMethod::Post, path: "/api/images".into(), auth: "auth".into(), handler_tag: "image_create".into() },
        RouteSpec { method: HttpMethod::Get, path: "/api/images/:id".into(), auth: "auth".into(), handler_tag: "image_get".into() },
        RouteSpec { method: HttpMethod::Get, path: "/api/images/:id/data".into(), auth: "auth".into(), handler_tag: "image_data".into() },
        RouteSpec { method: HttpMethod::Put, path: "/api/images/:id".into(), auth: "auth".into(), handler_tag: "image_rename".into() },
        RouteSpec { method: HttpMethod::Post, path: "/api/images/:id/apply".into(), auth: "auth".into(), handler_tag: "image_apply".into() },
        RouteSpec { method: HttpMethod::Delete, path: "/api/images/:id".into(), auth: "auth".into(), handler_tag: "image_delete".into() },
    ]
}

#[async_trait]
impl Plugin for ImagePlugin {
    fn manifest(&self) -> &Manifest {
        static M: std::sync::OnceLock<Manifest> = std::sync::OnceLock::new();
        M.get_or_init(|| Manifest {
            name: "image".into(),
            version: semver::Version::new(0, 1, 0),
            api_level: 1,
            entry_symbol: PLUGIN_ENTRY_SYMBOL.into(),
            target_triple: None,
            description: Some(
                "Image editor — apply photographic effects and transforms (Photon) in the Image window".into(),
            ),
            author: Some("shiny".into()),
            summary: Some("Image editor: effects, filters and transforms".into()),
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
            .skills(include_str!("../skills/image.md"))
            .context_line("Image: enabled — the Image window edits the user's images with effects and transforms.");
        for spec in route_specs() {
            builder.route(spec);
        }
        for tool in [
            Arc::new(crate::tools::ImageList) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::ImageGet) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::ImageEdit) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::ImageDelete) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
        ] {
            builder.tool_arc(shiny_plugin_sdk::tools::bridged(tool));
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
    Box::into_raw(Box::new(ImagePlugin { ctx: OnceLock::new() }))
}
