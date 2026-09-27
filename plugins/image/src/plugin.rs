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
    "a layered image editor AI; build compositions by adding, reordering, blending and merging layers, then apply tone and colour adjustments, filters, painted strokes, shapes and transforms to a layer";

fn route_specs() -> Vec<RouteSpec> {
    vec![
        RouteSpec { method: HttpMethod::Get, path: "/api/images".into(), auth: "auth".into(), handler_tag: "image_list".into() },
        RouteSpec { method: HttpMethod::Post, path: "/api/images".into(), auth: "auth".into(), handler_tag: "image_create".into() },
        RouteSpec { method: HttpMethod::Get, path: "/api/images/:id".into(), auth: "auth".into(), handler_tag: "image_get".into() },
        RouteSpec { method: HttpMethod::Get, path: "/api/images/:id/data".into(), auth: "auth".into(), handler_tag: "image_data".into() },
        RouteSpec { method: HttpMethod::Put, path: "/api/images/:id".into(), auth: "auth".into(), handler_tag: "image_rename".into() },
        RouteSpec { method: HttpMethod::Post, path: "/api/images/:id/apply".into(), auth: "auth".into(), handler_tag: "image_apply".into() },
        RouteSpec { method: HttpMethod::Get, path: "/api/images/:id/render".into(), auth: "auth".into(), handler_tag: "image_render".into() },
        RouteSpec { method: HttpMethod::Get, path: "/api/images/:id/selection".into(), auth: "auth".into(), handler_tag: "image_selection_get".into() },
        RouteSpec { method: HttpMethod::Put, path: "/api/images/:id/selection".into(), auth: "auth".into(), handler_tag: "image_selection_set".into() },
        RouteSpec { method: HttpMethod::Delete, path: "/api/images/:id/selection".into(), auth: "auth".into(), handler_tag: "image_selection_delete".into() },
        RouteSpec { method: HttpMethod::Post, path: "/api/images/:id/crop".into(), auth: "auth".into(), handler_tag: "image_crop".into() },
        RouteSpec { method: HttpMethod::Post, path: "/api/images/:id/resize".into(), auth: "auth".into(), handler_tag: "image_resize".into() },
        RouteSpec { method: HttpMethod::Post, path: "/api/images/:id/rotate".into(), auth: "auth".into(), handler_tag: "image_rotate".into() },
        RouteSpec { method: HttpMethod::Post, path: "/api/images/:id/flip".into(), auth: "auth".into(), handler_tag: "image_flip".into() },
        RouteSpec { method: HttpMethod::Delete, path: "/api/images/:id".into(), auth: "auth".into(), handler_tag: "image_delete".into() },
        // Layer stack
        RouteSpec { method: HttpMethod::Get, path: "/api/images/:id/layers".into(), auth: "auth".into(), handler_tag: "image_layer_list".into() },
        RouteSpec { method: HttpMethod::Post, path: "/api/images/:id/layers".into(), auth: "auth".into(), handler_tag: "image_layer_create".into() },
        RouteSpec { method: HttpMethod::Post, path: "/api/images/:id/layers/reorder".into(), auth: "auth".into(), handler_tag: "image_layer_reorder".into() },
        RouteSpec { method: HttpMethod::Put, path: "/api/images/:id/layers/:layer_id".into(), auth: "auth".into(), handler_tag: "image_layer_update".into() },
        RouteSpec { method: HttpMethod::Delete, path: "/api/images/:id/layers/:layer_id".into(), auth: "auth".into(), handler_tag: "image_layer_delete".into() },
        RouteSpec { method: HttpMethod::Get, path: "/api/images/:id/layers/:layer_id/thumb".into(), auth: "auth".into(), handler_tag: "image_layer_thumb".into() },
        RouteSpec { method: HttpMethod::Post, path: "/api/images/:id/layers/:layer_id/image".into(), auth: "auth".into(), handler_tag: "image_layer_image".into() },
        RouteSpec { method: HttpMethod::Post, path: "/api/images/:id/layers/:layer_id/duplicate".into(), auth: "auth".into(), handler_tag: "image_layer_duplicate".into() },
        RouteSpec { method: HttpMethod::Post, path: "/api/images/:id/layers/:layer_id/merge".into(), auth: "auth".into(), handler_tag: "image_layer_merge".into() },
        RouteSpec { method: HttpMethod::Post, path: "/api/images/:id/flatten".into(), auth: "auth".into(), handler_tag: "image_flatten".into() },
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
                "Layered raster editor — layers, groups, blend modes, tone and colour adjustments, filters, painting, shapes and transforms in the Image window".into(),
            ),
            author: Some("shiny".into()),
            summary: Some("Layered raster editor: adjustments, filters, painting and transforms".into()),
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
            .context_line("Image: enabled — the Image window edits layered images (layers, blend modes, adjustments, filters, painting, selection and transforms).");
        for spec in route_specs() {
            builder.route(spec);
        }
        for tool in [
            Arc::new(crate::tools::ImageList) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::ImageGet) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::ImageEdit) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::ImageDelete) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::ImageLayerList) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::ImageLayerAdd) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::ImageLayerUpdate) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::ImageLayerDelete) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::ImageLayerMerge) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
            Arc::new(crate::tools::ImageFlatten) as Arc<dyn shiny_plugin_sdk::tools::Tool>,
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
