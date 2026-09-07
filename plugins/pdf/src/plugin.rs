use std::sync::{Arc, OnceLock};
use async_trait::async_trait;
use shiny_plugin_sdk::{
    manifest::Manifest,
    plugin::{Plugin, PLUGIN_ENTRY_SYMBOL},
    routes::{HttpMethod, RouteHandler, RouteSpec},
    services::PluginCtx,
    tools::{bridged, RegistryBuilder, Tool},
};

pub struct PdfPlugin {
    ctx: OnceLock<Arc<PluginCtx>>,
}

/// Persona fragment the agent system prompt sees when this plugin is active.
pub const PERSONA: &str = "a PDF editor; view, read and edit the user's PDF documents";

fn route_specs() -> Vec<RouteSpec> {
    vec![
        RouteSpec { method: HttpMethod::Get, path: "/api/pdfs".into(), auth: "auth".into(), handler_tag: "pdf_list".into() },
        RouteSpec { method: HttpMethod::Post, path: "/api/pdfs".into(), auth: "auth".into(), handler_tag: "pdf_create".into() },
        RouteSpec { method: HttpMethod::Post, path: "/api/pdfs/import".into(), auth: "auth".into(), handler_tag: "pdf_import".into() },
        RouteSpec { method: HttpMethod::Get, path: "/api/pdfs/:id".into(), auth: "auth".into(), handler_tag: "pdf_get".into() },
        RouteSpec { method: HttpMethod::Put, path: "/api/pdfs/:id".into(), auth: "auth".into(), handler_tag: "pdf_rename".into() },
        RouteSpec { method: HttpMethod::Delete, path: "/api/pdfs/:id".into(), auth: "auth".into(), handler_tag: "pdf_delete".into() },
        RouteSpec { method: HttpMethod::Get, path: "/api/pdfs/:id/export".into(), auth: "auth".into(), handler_tag: "pdf_export".into() },
        RouteSpec { method: HttpMethod::Get, path: "/api/pdfs/:id/pages/:page".into(), auth: "auth".into(), handler_tag: "pdf_render".into() },
        RouteSpec { method: HttpMethod::Get, path: "/api/pdfs/:id/text/:page".into(), auth: "auth".into(), handler_tag: "pdf_text".into() },
        RouteSpec { method: HttpMethod::Post, path: "/api/pdfs/:id/rotate".into(), auth: "auth".into(), handler_tag: "pdf_rotate".into() },
        RouteSpec { method: HttpMethod::Post, path: "/api/pdfs/:id/reorder".into(), auth: "auth".into(), handler_tag: "pdf_reorder".into() },
        RouteSpec { method: HttpMethod::Post, path: "/api/pdfs/:id/delete-pages".into(), auth: "auth".into(), handler_tag: "pdf_delete_pages".into() },
        RouteSpec { method: HttpMethod::Post, path: "/api/pdfs/:id/merge".into(), auth: "auth".into(), handler_tag: "pdf_merge".into() },
        RouteSpec { method: HttpMethod::Post, path: "/api/pdfs/:id/replace-text".into(), auth: "auth".into(), handler_tag: "pdf_replace_text".into() },
        RouteSpec { method: HttpMethod::Post, path: "/api/pdfs/:id/annotate".into(), auth: "auth".into(), handler_tag: "pdf_annotate".into() },
        RouteSpec { method: HttpMethod::Post, path: "/api/pdfs/:id/watermark".into(), auth: "auth".into(), handler_tag: "pdf_watermark".into() },
    ]
}

#[async_trait]
impl Plugin for PdfPlugin {
    fn manifest(&self) -> &Manifest {
        static M: std::sync::OnceLock<Manifest> = std::sync::OnceLock::new();
        M.get_or_init(|| Manifest {
            name: "pdf".into(),
            version: semver::Version::new(0, 1, 0),
            api_level: 1,
            entry_symbol: PLUGIN_ENTRY_SYMBOL.into(),
            target_triple: None,
            description: Some("PDF viewer & editor — render pages, extract text, edit text, add highlights/notes/links/watermarks, rotate/delete/reorder/merge pages, create PDFs".into()),
            author: Some("shiny".into()),
            summary: Some("PDF: view pages, extract text, edit & annotate (highlight, note, link, watermark), rotate/reorder/merge, create PDFs".into()),
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
            .skills(include_str!("../skills/pdf.md"))
            .context_line("PDF: enabled — the PDF window renders and edits .pdf files stored server-side.");
        for spec in route_specs() {
            builder.route(spec);
        }
        for tool in [
            Arc::new(crate::tools::PdfCreate) as Arc<dyn Tool>,
            Arc::new(crate::tools::PdfList) as Arc<dyn Tool>,
            Arc::new(crate::tools::PdfRead) as Arc<dyn Tool>,
            Arc::new(crate::tools::PdfRotate) as Arc<dyn Tool>,
            Arc::new(crate::tools::PdfDeletePages) as Arc<dyn Tool>,
            Arc::new(crate::tools::PdfReorder) as Arc<dyn Tool>,
            Arc::new(crate::tools::PdfMerge) as Arc<dyn Tool>,
            Arc::new(crate::tools::PdfReplaceText) as Arc<dyn Tool>,
            Arc::new(crate::tools::PdfAnnotate) as Arc<dyn Tool>,
            Arc::new(crate::tools::PdfAddNote) as Arc<dyn Tool>,
            Arc::new(crate::tools::PdfWatermark) as Arc<dyn Tool>,
            Arc::new(crate::tools::PdfDelete) as Arc<dyn Tool>,
        ] {
            builder.tool_arc(bridged(tool));
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
    Box::into_raw(Box::new(PdfPlugin { ctx: OnceLock::new() }))
}
