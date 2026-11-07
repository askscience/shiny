//! Word plugin tools: create/write/edit/read/list/delete documents, set how
//! they are composed (`doc_design`) and export them (`doc_export`).
//!
//! Documents live in the plugin's `documents` table as real .odt bytes
//! (WordCraft codecs, see `engine.rs`). The agent writes and reads Markdown.

use async_trait::async_trait;
use serde_json::{json, Value};
use sqlx::SqlitePool;

use shiny_plugin_sdk::errors::AppError;
use crate::engine::{self, Design, Format};
use shiny_plugin_sdk::outcome::ActionOutcome;
use shiny_plugin_sdk::services::PluginCtx;
use shiny_plugin_sdk::tools::{ParamHelpers, Tool, ToolRequest};

/// The agent's Markdown (headings, lists, tables, emphasis, links) as the
/// editor's HTML fragment, through WordCraft's Markdown reader.
pub fn content_to_html(content: &str) -> String {
    engine::markdown_to_html(content)
}

/// Byte range of the first case-insensitive occurrence of `needle` in `hay`.
/// Byte-safe (uses `char_indices`), unlike the old `to_lowercase().find()`
/// approach which could misalign offsets when case-folding changes byte length.
fn find_ci(hay: &str, needle: &str) -> Option<std::ops::Range<usize>> {
    let needle_chars: Vec<char> = needle.chars().collect();
    if needle_chars.is_empty() {
        return None;
    }
    let hay_chars: Vec<(usize, char)> = hay.char_indices().collect();
    if hay_chars.len() < needle_chars.len() {
        return None;
    }
    'outer: for i in 0..=(hay_chars.len() - needle_chars.len()) {
        for (j, n) in needle_chars.iter().enumerate() {
            if !hay_chars[i + j].1.eq_ignore_ascii_case(n) {
                continue 'outer;
            }
        }
        let start = hay_chars[i].0;
        let last = &hay_chars[i + needle_chars.len() - 1];
        let end = last.0 + last.1.len_utf8();
        return Some(start..end);
    }
    None
}

async fn last_doc_id(pool: &SqlitePool, user_id: &str) -> Result<Option<String>, AppError> {
    Ok(sqlx::query_scalar(
        "SELECT id FROM documents WHERE user_id = ?1 \
         ORDER BY max(updated_at, COALESCE(opened_at, '')) DESC LIMIT 1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?)
}

fn doc_summary_json(row: &(String, String, String)) -> Value {
    json!({ "doc_id": row.0, "title": row.1, "updated_at": row.2 })
}

/// The first Markdown heading of `content`, else its first non-empty line,
/// cleaned of Markdown marks and kept to 80 characters.
fn title_from_content(content: &str) -> Option<String> {
    let lines: Vec<&str> = content.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let pick = lines
        .iter()
        .find(|l| l.starts_with('#'))
        .or_else(|| lines.first())?;
    let clean: String = pick
        .trim_start_matches('#')
        .trim()
        .replace("**", "")
        .replace('*', "")
        .chars()
        .take(80)
        .collect();
    let clean = clean.trim().to_string();
    (!clean.is_empty()).then_some(clean)
}

/* ── doc_create ─────────────────────────────────────────────── */

pub struct DocCreate;

#[async_trait]
impl Tool for DocCreate {
    fn name(&self) -> &str { "doc_create" }
    fn aliases(&self) -> &[&str] { &["create_document", "new_document"] }
    fn step_label(&self) -> &str { "Creating document…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `doc_create` — Create a new document. params: `{ title?: string, content?: string }` — content is Markdown: `#`/`##`/`###` headings, paragraphs, `- ` and `1. ` lists, `| a | b |` tables, `**bold**`, `*italic*`, `> quotes`, links. Returns the new `doc_id`.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("Untitled");
        format!("Created document \"{title}\"")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let content = req.params.param_str("content").unwrap_or_default();
        // A document written without a title is named after its first
        // heading (or first line), not left as "Untitled".
        let title = req
            .params
            .param_str("title")
            .or_else(|| req.params.param_str("name"))
            .filter(|t| !t.trim().is_empty())
            .or_else(|| title_from_content(&content))
            .unwrap_or_else(|| "Untitled".into());
        let html = content_to_html(&content);
        let odt_bytes = engine::html_to_odt(&html)?;
        let id = uuid::Uuid::new_v4().to_string();

        sqlx::query(
            "INSERT INTO documents (id, user_id, title, odt, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, datetime('now'), datetime('now'))",
        )
        .bind(&id)
        .bind(req.traveler_id)
        .bind(&title)
        .bind(&odt_bytes)
        .execute(ctx.pool().await)
        .await?;

        Ok(ActionOutcome::ok(
            "doc_create",
            json!({ "doc_id": id, "title": title }),
        ))
    }
}

/* ── doc_write ──────────────────────────────────────────────── */

pub struct DocWrite;

#[async_trait]
impl Tool for DocWrite {
    fn name(&self) -> &str { "doc_write" }
    fn aliases(&self) -> &[&str] { &["write_document", "update_document"] }
    fn step_label(&self) -> &str { "Writing document…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `doc_write` — Write or replace a document's content. params: `{ doc_id?: string, content: string }` — without `doc_id` targets the most recently used document.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("document");
        format!("Wrote to \"{title}\"")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let content = req.params.param_str("content").unwrap_or_default();
        if content.trim().is_empty() {
            return Err(AppError::BadRequest("content required (non-empty)".into()));
        }
        let doc_id = match req.params.param_str("doc_id") {
            Some(id) if !id.trim().is_empty() => id,
            _ => last_doc_id(ctx.pool().await, req.traveler_id)
                .await?
                .ok_or_else(|| {
                    AppError::BadRequest(
                        "No document yet — call doc_create first".into(),
                    )
                })?,
        };

        let title: Option<String> =
            sqlx::query_scalar("SELECT title FROM documents WHERE id = ?1 AND user_id = ?2")
                .bind(&doc_id)
                .bind(req.traveler_id)
                .fetch_optional(ctx.pool().await)
                .await?;
        let title = title.ok_or_else(|| AppError::NotFound("Document not found".into()))?;

        let html = content_to_html(&content);
        let odt_bytes = engine::html_to_odt(&html)?;
        let result = sqlx::query(
            "UPDATE documents SET odt = ?1, updated_at = datetime('now') \
             WHERE id = ?2 AND user_id = ?3",
        )
        .bind(&odt_bytes)
        .bind(&doc_id)
        .bind(req.traveler_id)
        .execute(ctx.pool().await)
        .await?;
        if result.rows_affected() == 0 {
            return Err(AppError::NotFound("Document not found".into()));
        }

        Ok(ActionOutcome::ok(
            "doc_write",
            json!({ "doc_id": doc_id, "title": title }),
        ))
    }
}

/* ── doc_edit ───────────────────────────────────────────────── */

pub struct DocEdit;

#[async_trait]
impl Tool for DocEdit {
    fn name(&self) -> &str { "doc_edit" }
    fn aliases(&self) -> &[&str] {
        &["edit_document", "modify_document", "change_document", "replace_text"]
    }
    fn step_label(&self) -> &str { "Editing document…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `doc_edit` — Make a targeted change inside a document. params: `{ doc_id?: string, old: string, new: string }` — replaces the first occurrence of `old` with `new`, keeping everything else intact. Use for \"change X to Y\", \"fix the name\", etc. When the text to change isn't in the document, call `doc_write` with the COMPLETE desired text instead.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("document");
        format!("Edited \"{title}\"")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let old = req
            .params
            .param_str("old")
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| AppError::BadRequest("old required — the text to change".into()))?;
        let new = req.params.param_str("new").unwrap_or_default();
        let doc_id = match req.params.param_str("doc_id") {
            Some(id) if !id.trim().is_empty() => id,
            _ => last_doc_id(ctx.pool().await, req.traveler_id)
                .await?
                .ok_or_else(|| {
                    AppError::BadRequest("No document yet — call doc_create first".into())
                })?,
        };

        let row = sqlx::query_as::<_, (String, Vec<u8>)>(
            "SELECT title, odt FROM documents WHERE id = ?1 AND user_id = ?2",
        )
        .bind(&doc_id)
        .bind(req.traveler_id)
        .fetch_optional(ctx.pool().await)
        .await?
        .ok_or_else(|| AppError::NotFound("Document not found".into()))?;
        let (title, odt_bytes) = row;

        let html = engine::odt_to_html(&odt_bytes)?;
        // The HTML carries the text escaped: match and insert it that way.
        let old_html = wordcraft_formats::html::esc(&old);
        let new_html = wordcraft_formats::html::esc(&new);
        let edited = if html.contains(&old_html) {
            html.replacen(&old_html, &new_html, 1)
        } else if let Some(range) = find_ci(&html, &old_html) {
            let mut s = html;
            s.replace_range(range, &new_html);
            s
        } else {
            // The model must not guess: tell it to rewrite in full. Include
            // the complete current content so it can produce a correct rewrite.
            let preview: String = engine::odt_to_markdown(&odt_bytes)?;
            return Ok(ActionOutcome::error(
                "doc_edit",
                format!(
                    "Text \"{old}\" not found in the document. Current content: {preview}"
                ),
            ));
        };

        let new_odt = engine::html_to_odt(&edited)?;
        sqlx::query(
            "UPDATE documents SET odt = ?1, updated_at = datetime('now') \
             WHERE id = ?2 AND user_id = ?3",
        )
        .bind(&new_odt)
        .bind(&doc_id)
        .bind(req.traveler_id)
        .execute(ctx.pool().await)
        .await?;

        Ok(ActionOutcome::ok(
            "doc_edit",
            json!({ "doc_id": doc_id, "title": title }),
        ))
    }
}

/* ── doc_append ─────────────────────────────────────────────── */

pub struct DocAppend;

#[async_trait]
impl Tool for DocAppend {
    fn name(&self) -> &str { "doc_append" }
    fn aliases(&self) -> &[&str] { &["append_text", "add_to_document"] }
    fn step_label(&self) -> &str { "Appending to document…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `doc_append` — Append content to a document, keeping what's already in it. params: `{ doc_id?: string, content: string }` — without `doc_id` targets the most recently used document.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("document");
        format!("Appended to \"{title}\"")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let content = req.params.param_str("content").unwrap_or_default();
        if content.trim().is_empty() {
            return Err(AppError::BadRequest("content required (non-empty)".into()));
        }
        let doc_id = match req.params.param_str("doc_id") {
            Some(id) if !id.trim().is_empty() => id,
            _ => last_doc_id(ctx.pool().await, req.traveler_id)
                .await?
                .ok_or_else(|| {
                    AppError::BadRequest("No document yet — call doc_create first".into())
                })?,
        };

        let row = sqlx::query_as::<_, (String, Vec<u8>)>(
            "SELECT title, odt FROM documents WHERE id = ?1 AND user_id = ?2",
        )
        .bind(&doc_id)
        .bind(req.traveler_id)
        .fetch_optional(ctx.pool().await)
        .await?
        .ok_or_else(|| AppError::NotFound("Document not found".into()))?;
        let (title, odt_bytes) = row;

        // Append at the HTML level so existing formatting survives.
        let existing = engine::odt_to_html(&odt_bytes)?;
        let html = format!("{}\n{}", existing, content_to_html(&content));
        let new_odt = engine::html_to_odt(&html)?;
        sqlx::query(
            "UPDATE documents SET odt = ?1, updated_at = datetime('now') \
             WHERE id = ?2 AND user_id = ?3",
        )
        .bind(&new_odt)
        .bind(&doc_id)
        .bind(req.traveler_id)
        .execute(ctx.pool().await)
        .await?;

        Ok(ActionOutcome::ok(
            "doc_append",
            json!({ "doc_id": doc_id, "title": title }),
        ))
    }
}

/* ── doc_read ───────────────────────────────────────────────── */

pub struct DocRead;

#[async_trait]
impl Tool for DocRead {
    fn name(&self) -> &str { "doc_read" }
    fn aliases(&self) -> &[&str] { &["read_document", "open_document", "doc_open"] }
    fn step_label(&self) -> &str { "Reading document…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `doc_read` — Read a document's content. params: `{ doc_id?: string }` — without `doc_id` reads the most recently used document.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("document");
        format!("Read \"{title}\"")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let doc_id = match req.params.param_str("doc_id") {
            Some(id) if !id.trim().is_empty() => id,
            _ => last_doc_id(ctx.pool().await, req.traveler_id)
                .await?
                .ok_or_else(|| {
                    AppError::BadRequest(
                        "No document yet — call doc_create first".into(),
                    )
                })?,
        };

        let row = sqlx::query_as::<_, (String, Vec<u8>)>(
            "SELECT title, odt FROM documents WHERE id = ?1 AND user_id = ?2",
        )
        .bind(&doc_id)
        .bind(req.traveler_id)
        .fetch_optional(ctx.pool().await)
        .await?
        .ok_or_else(|| AppError::NotFound("Document not found".into()))?;
        let (title, odt_bytes) = row;
        let text = engine::odt_to_markdown(&odt_bytes)?;

        Ok(ActionOutcome::ok(
            "doc_read",
            json!({
                "doc_id": doc_id,
                "title": title,
                "content": text,
                "format": "markdown",
                "hint": "To change a word or sentence use doc_edit {old,new}; to add content use doc_append; use doc_write only for a complete rewrite.",
            }),
        ))
    }
}

/* ── doc_list ───────────────────────────────────────────────── */

pub struct DocList;

#[async_trait]
impl Tool for DocList {
    fn name(&self) -> &str { "doc_list" }
    fn aliases(&self) -> &[&str] { &["list_documents", "documents"] }
    fn step_label(&self) -> &str { "Listing documents…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `doc_list` — List the user's documents. params: `{}` — returns `doc_id`, `title`, `updated_at` per document.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let n = data.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
        format!("Found {n} documents")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let rows = sqlx::query_as::<_, (String, String, String)>(
            "SELECT id, title, updated_at FROM documents \
             WHERE user_id = ?1 ORDER BY updated_at DESC LIMIT 100",
        )
        .bind(req.traveler_id)
        .fetch_all(ctx.pool().await)
        .await?;

        let docs: Vec<Value> = rows.iter().map(doc_summary_json).collect();
        Ok(ActionOutcome::ok(
            "doc_list",
            json!({ "documents": docs, "count": docs.len() }),
        ))
    }
}

/* ── doc_delete ─────────────────────────────────────────────── */

pub struct DocDelete;

#[async_trait]
impl Tool for DocDelete {
    fn name(&self) -> &str { "doc_delete" }
    fn aliases(&self) -> &[&str] { &["delete_document", "remove_document"] }
    fn step_label(&self) -> &str { "Deleting document…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `doc_delete` — Delete a document. params: `{ doc_id: string }`")
    }
    fn humanize(&self, _r: &str, _d: &Value) -> String {
        "Document deleted".into()
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let doc_id = req.params.require_str("doc_id")?;
        let result = sqlx::query("DELETE FROM documents WHERE id = ?1 AND user_id = ?2")
            .bind(&doc_id)
            .bind(req.traveler_id)
            .execute(ctx.pool().await)
            .await?;
        if result.rows_affected() == 0 {
            return Ok(ActionOutcome::error("doc_delete", "Document not found"));
        }
        Ok(ActionOutcome::ok("doc_delete", json!({ "doc_id": doc_id })))
    }
}
/* ── shared ─────────────────────────────────────────────────── */

/// `doc_id` from the params, else the most recently used document.
async fn target_doc(ctx: &PluginCtx, req: &ToolRequest<'_>) -> Result<String, AppError> {
    match req.params.param_str("doc_id") {
        Some(id) if !id.trim().is_empty() => Ok(id),
        _ => last_doc_id(ctx.pool().await, req.traveler_id)
            .await?
            .ok_or_else(|| AppError::BadRequest("No document yet — call doc_create first".into())),
    }
}

/* ── doc_design ─────────────────────────────────────────────── */

/// Merge `doc_design` params into a stored design: only the keys passed
/// change. Margins are millimetres (0 = the 25 mm default) and go through the
/// same `Design::validate` as the window's design route.
fn apply_design_params(mut design: Design, p: &Value) -> Result<Design, AppError> {
    if let Some(t) = p.param_str("template") {
        let t = t.trim().to_ascii_lowercase();
        design = design.with_template_defaults(if t == "plain" { "" } else { &t });
    }
    let flag = |k: &str| p.get(k).and_then(|v| v.as_bool().or_else(|| v.as_str().map(|s| s == "true")));
    if let Some(v) = flag("cover") { design.cover = v; }
    if let Some(v) = flag("toc") { design.toc = v; }
    if let Some(v) = flag("header") { design.header = v; }
    if let Some(v) = flag("page_numbers") { design.page_numbers = v; }
    for (key, slot) in [
        ("paper", &mut design.paper),
        ("subtitle", &mut design.subtitle),
        ("author", &mut design.author),
        ("date", &mut design.date),
        ("lang", &mut design.lang),
        ("accent", &mut design.accent),
    ] {
        if let Some(v) = p.param_str(key) {
            *slot = v.trim().to_string();
        }
    }
    for (key, slot) in [
        ("margin_top", &mut design.margin_top),
        ("margin_right", &mut design.margin_right),
        ("margin_bottom", &mut design.margin_bottom),
        ("margin_left", &mut design.margin_left),
    ] {
        match p.get(key) {
            None | Some(Value::Null) => {}
            Some(v) => {
                let mm = v
                    .as_f64()
                    .or_else(|| v.as_str().and_then(|s| s.trim().trim_end_matches("mm").trim().parse().ok()))
                    .ok_or_else(|| AppError::BadRequest(format!("{key} must be a number of millimetres")))?;
                *slot = mm as f32;
            }
        }
    }
    design.validate()?;
    Ok(design)
}

pub struct DocDesign;

#[async_trait]
impl Tool for DocDesign {
    fn name(&self) -> &str { "doc_design" }
    fn aliases(&self) -> &[&str] { &["doc_layout", "format_document", "style_document"] }
    fn step_label(&self) -> &str { "Designing document…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `doc_design` — Set how a document looks when exported (Word/PDF): params: `{ doc_id?, template?: \"plain\"|\"classic\"|\"modern\"|\"minimal\", cover?: bool, toc?: bool, header?: bool, page_numbers?: bool, paper?: \"a4\"|\"letter\", subtitle?, author?, date?, lang?, accent?: \"#RRGGBB\", margin_top?, margin_right?, margin_bottom?, margin_left? }` — margins in millimetres, 5–80 (0 = the 25 mm default); only the keys you pass change. Choosing a template turns on the header and page numbers unless you pass them.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("document");
        format!("Designed \"{title}\"")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let doc_id = target_doc(ctx, &req).await?;
        let row = sqlx::query_as::<_, (String, String)>(
            "SELECT title, design FROM documents WHERE id = ?1 AND user_id = ?2",
        )
        .bind(&doc_id)
        .bind(req.traveler_id)
        .fetch_optional(ctx.pool().await)
        .await?
        .ok_or_else(|| AppError::NotFound("Document not found".into()))?;
        let (title, stored) = row;

        let design = match apply_design_params(Design::from_json(&stored), req.params) {
            Ok(d) => d,
            Err(AppError::BadRequest(msg)) => return Ok(ActionOutcome::error("doc_design", msg)),
            Err(e) => return Err(e),
        };
        if let Err(AppError::BadRequest(msg)) = design.validate() {
            return Ok(ActionOutcome::error("doc_design", msg));
        }
        sqlx::query("UPDATE documents SET design = ?1, updated_at = datetime('now') WHERE id = ?2 AND user_id = ?3")
            .bind(design.to_json())
            .bind(&doc_id)
            .bind(req.traveler_id)
            .execute(ctx.pool().await)
            .await?;

        Ok(ActionOutcome::ok(
            "doc_design",
            json!({ "doc_id": doc_id, "title": title, "design": design }),
        ))
    }
}

/* ── doc_export ─────────────────────────────────────────────── */

pub struct DocExport;

#[async_trait]
impl Tool for DocExport {
    fn name(&self) -> &str { "doc_export" }
    fn aliases(&self) -> &[&str] { &["export_document", "save_document_as", "doc_save_as"] }
    fn step_label(&self) -> &str { "Exporting document…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `doc_export` — Save a document as a file in the user's Documents folder. params: `{ doc_id?, format: \"docx\"|\"pdf\"|\"odt\"|\"md\"|\"html\" }` — uses the document's design (see `doc_design`).")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let name = data.get("filename").and_then(|v| v.as_str()).unwrap_or("document");
        format!("Exported \"{name}\"")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let doc_id = target_doc(ctx, &req).await?;
        let name = req.params.param_str("format").unwrap_or_else(|| "docx".into());
        let Some(format) = Format::parse(&name) else {
            return Ok(ActionOutcome::error("doc_export", format!("Unknown format \"{name}\" — use docx, pdf, odt, md or html")));
        };
        let title: Option<String> =
            sqlx::query_scalar("SELECT title FROM documents WHERE id = ?1 AND user_id = ?2")
                .bind(&doc_id)
                .bind(req.traveler_id)
                .fetch_optional(ctx.pool().await)
                .await?;
        let title = title.ok_or_else(|| AppError::NotFound("Document not found".into()))?;
        let filename = shiny_plugin_sdk::util::filename_for_title(&title, format.ext(), "document");
        // The window fetches the file from this route and saves it to Documents.
        Ok(ActionOutcome::ok(
            "doc_export",
            json!({
                "doc_id": doc_id,
                "title": title,
                "format": format.ext(),
                "filename": filename,
                "url": format!("/api/documents/{doc_id}/export?format={}", format.ext()),
            }),
        ))
    }
}

/* ── doc_insert_image ───────────────────────────────────────── */

/// Largest picture file read from the user's home.
const MAX_PICTURE: u64 = 25 * 1024 * 1024;

/// A home-relative (or absolute, inside the home) path, resolved and
/// confined to the user's files home.
fn home_file(user_id: &str, os_home: Option<&str>, raw: &str) -> Result<std::path::PathBuf, AppError> {
    let home = match os_home.map(str::trim).filter(|h| !h.is_empty()) {
        Some(h) => std::path::PathBuf::from(h),
        None => {
            let base = std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_else(|| ".".into());
            base.join(".shiny").join("home").join(shiny_plugin_sdk::util::sanitize_user(user_id))
        }
    };
    let raw = raw.trim().trim_start_matches("~/");
    let candidate = if std::path::Path::new(raw).is_absolute() { std::path::PathBuf::from(raw) } else { home.join(raw) };
    let real = candidate.canonicalize().map_err(|_| AppError::NotFound(format!("No file at {raw}")))?;
    let home_real = home.canonicalize().unwrap_or(home);
    if !real.starts_with(&home_real) || !real.is_file() {
        return Err(AppError::BadRequest("The picture must be a file in the user's home".into()));
    }
    Ok(real)
}

pub struct DocInsertImage;

#[async_trait]
impl Tool for DocInsertImage {
    fn name(&self) -> &str { "doc_insert_image" }
    fn aliases(&self) -> &[&str] { &["insert_image", "doc_add_image", "add_picture"] }
    fn step_label(&self) -> &str { "Adding picture…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `doc_insert_image` — Put a picture from the user's files into a document (e.g. one `generate_image` just made). params: `{ doc_id?, path: string, after?: string, caption?: string, width?: 10-100 }` — `path` is the file's path in the home (use the `path` `generate_image` returns); `after` is text of the heading or paragraph to place it after (default: the end); `width` is a percentage of the text width (default 100, never wider than the picture).")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("document");
        format!("Added a picture to \"{title}\"")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let raw = req
            .params
            .param_str("path")
            .or_else(|| req.params.param_str("file"))
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| AppError::BadRequest("path required — the picture's path in the user's files".into()))?;
        let doc_id = target_doc(ctx, &req).await?;
        let file = match home_file(req.user_id, req.os_home, &raw) {
            Ok(f) => f,
            Err(e) => return Ok(ActionOutcome::error("doc_insert_image", e.to_string())),
        };
        let meta = tokio::fs::metadata(&file).await.map_err(|e| AppError::Internal(format!("read error: {e}")))?;
        if meta.len() > MAX_PICTURE {
            return Ok(ActionOutcome::error("doc_insert_image", "The picture is too large (max 25 MB)"));
        }
        let bytes = tokio::fs::read(&file).await.map_err(|e| AppError::Internal(format!("read error: {e}")))?;
        let caption = req.params.param_str("caption").map(|c| c.trim().to_string()).filter(|c| !c.is_empty());
        let alt = caption.clone().unwrap_or_else(|| {
            file.file_stem().map(|s| s.to_string_lossy().replace(['-', '_'], " ")).unwrap_or_default()
        });
        let width = req.params.param_f64("width").map(|w| w as f32).unwrap_or(100.0);
        let img = match engine::picture_html(&bytes, &alt, width) {
            Ok(h) => h,
            Err(e) => return Ok(ActionOutcome::error("doc_insert_image", e.to_string())),
        };
        let mut block = format!("<p style=\"text-align:center\">{img}</p>");
        if let Some(c) = &caption {
            block.push_str(&format!("<p style=\"text-align:center\"><em>{}</em></p>", wordcraft_formats::html::esc(c)));
        }

        let row = sqlx::query_as::<_, (String, Vec<u8>)>("SELECT title, odt FROM documents WHERE id = ?1 AND user_id = ?2")
            .bind(&doc_id)
            .bind(req.traveler_id)
            .fetch_optional(ctx.pool().await)
            .await?
            .ok_or_else(|| AppError::NotFound("Document not found".into()))?;
        let (title, odt_bytes) = row;
        let html = engine::odt_to_html(&odt_bytes)?;
        let after = req.params.param_str("after");
        let (edited, placed) = engine::insert_after_block(&html, after.as_deref(), &block);
        let new_odt = engine::html_to_odt(&edited)?;
        sqlx::query("UPDATE documents SET odt = ?1, updated_at = datetime('now') WHERE id = ?2 AND user_id = ?3")
            .bind(&new_odt)
            .bind(&doc_id)
            .bind(req.traveler_id)
            .execute(ctx.pool().await)
            .await?;

        let mut data = json!({ "doc_id": doc_id, "title": title });
        if !placed {
            data["note"] = json!("The `after` text was not found; the picture was added at the end.");
        }
        Ok(ActionOutcome::ok("doc_insert_image", data))
    }
}

#[cfg(test)]
mod tests {
    use super::home_file;

    #[test]
    fn pictures_must_live_in_the_home() {
        let base = std::env::temp_dir().join(format!("word-home-{}", std::process::id()));
        let home = base.join("home");
        std::fs::create_dir_all(home.join("Pictures")).unwrap();
        std::fs::write(home.join("Pictures/a.png"), b"x").unwrap();
        std::fs::write(base.join("secret.png"), b"x").unwrap();
        let h = home.to_str().unwrap();
        assert!(home_file("u", Some(h), "Pictures/a.png").is_ok());
        assert!(home_file("u", Some(h), "~/Pictures/a.png").is_ok());
        assert!(home_file("u", Some(h), "../secret.png").is_err(), "no escaping the home");
        assert!(home_file("u", Some(h), base.join("secret.png").to_str().unwrap()).is_err(), "absolute paths stay inside");
        assert!(home_file("u", Some(h), "Pictures").is_err(), "a folder is not a picture");
        assert!(home_file("u", Some(h), "missing.png").is_err());
        let _ = std::fs::remove_dir_all(&base);
    }
}

#[cfg(test)]
mod title_tests {
    use super::title_from_content;

    #[test]
    fn an_untitled_document_takes_its_first_heading() {
        assert_eq!(title_from_content("# Three-Day Lisbon Itinerary\n\n## Day 1").as_deref(), Some("Three-Day Lisbon Itinerary"));
        assert_eq!(title_from_content("Intro line\n\n## Later heading").as_deref(), Some("Later heading"));
        assert_eq!(title_from_content("**Packing list**\n- socks").as_deref(), Some("Packing list"));
        assert_eq!(title_from_content("   \n").as_deref(), None);
    }
}

#[cfg(test)]
mod design_tests {
    use super::apply_design_params;
    use crate::engine::Design;
    use serde_json::json;

    #[test]
    fn doc_design_reads_margins_in_millimetres() {
        let start = Design { margin_left: 30.0, ..Default::default() };
        let d = apply_design_params(start, &json!({ "margin_top": 20, "margin_bottom": "15", "margin_right": 0 })).unwrap();
        assert_eq!((d.margin_top, d.margin_right, d.margin_bottom, d.margin_left), (20.0, 0.0, 15.0, 30.0));
        // Same bounds as the window's design route.
        assert!(apply_design_params(Design::default(), &json!({ "margin_left": 2 })).is_err());
        assert!(apply_design_params(Design::default(), &json!({ "margin_left": 90 })).is_err());
        assert!(apply_design_params(Design::default(), &json!({ "margin_left": "wide" })).is_err());
    }
}
