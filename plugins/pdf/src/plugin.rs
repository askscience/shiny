use std::sync::Arc;
use async_trait::async_trait;
use shiny_plugin_sdk::{
    manifest::Manifest,
    plugin::Plugin,
    routes::{RouteHandler, RouteSpec},
    services::PluginCtx,
    tools::{RegistryBuilder, Tool},
};

shiny_plugin_sdk::ctx_plugin!(PdfPlugin);

/// Persona fragment the agent system prompt sees when this plugin is active.
pub const PERSONA: &str = "a PDF editor; view, read and edit the user's PDF documents";

fn route_specs() -> Vec<RouteSpec> {
    vec![
        RouteSpec::get("/api/pdfs", "pdf_list"),
        RouteSpec::post("/api/pdfs", "pdf_create"),
        RouteSpec::post("/api/pdfs/import", "pdf_import"),
        RouteSpec::get("/api/pdfs/:id", "pdf_get"),
        RouteSpec::put("/api/pdfs/:id", "pdf_rename"),
        RouteSpec::delete("/api/pdfs/:id", "pdf_delete"),
        RouteSpec::get("/api/pdfs/:id/export", "pdf_export"),
        RouteSpec::get("/api/pdfs/:id/file", "pdf_file"),
        RouteSpec::get("/api/pdfs/:id/pages/:page", "pdf_render"),
        RouteSpec::get("/api/pdfs/:id/text/:page", "pdf_text"),
        RouteSpec::get("/api/pdfs/:id/runs/:page", "pdf_text_runs"),
        RouteSpec::post("/api/pdfs/:id/edit-text", "pdf_edit_text"),
        RouteSpec::post("/api/pdfs/:id/rotate", "pdf_rotate"),
        RouteSpec::post("/api/pdfs/:id/reorder", "pdf_reorder"),
        RouteSpec::post("/api/pdfs/:id/delete-pages", "pdf_delete_pages"),
        RouteSpec::post("/api/pdfs/:id/merge", "pdf_merge"),
        RouteSpec::post("/api/pdfs/:id/replace-text", "pdf_replace_text"),
        RouteSpec::post("/api/pdfs/:id/annotate", "pdf_annotate"),
        RouteSpec::post("/api/pdfs/:id/watermark", "pdf_watermark"),
    ]
}

#[async_trait]
impl Plugin for PdfPlugin {
    fn manifest(&self) -> &Manifest {
        static M: std::sync::OnceLock<Manifest> = std::sync::OnceLock::new();
        M.get_or_init(|| {
            Manifest::new("pdf")
                .description("PDF viewer & editor — render pages, extract text, edit text, add highlights/notes/links/watermarks, rotate/delete/reorder/merge pages, create PDFs")
                .summary("PDF: view pages, extract text, edit & annotate (highlight, note, link, watermark), rotate/reorder/merge, create PDFs")
        })
    }

    fn register(&self, ctx: Arc<PluginCtx>, builder: &mut RegistryBuilder<'_>) {
        self.install_ctx(ctx);
        builder
            .persona(PERSONA)
            .skills(include_str!("../skills/pdf.md"))
            .context_line("PDF: enabled — the PDF window renders and edits .pdf files stored server-side.");
        builder.routes(route_specs());
        builder.tools([
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
            Arc::new(crate::tools::PdfDelete) as Arc<dyn Tool>,]);
    }

    fn route_handler(&self, tag: &str) -> Option<RouteHandler> {
        self.routes_for(tag)
    }
}

