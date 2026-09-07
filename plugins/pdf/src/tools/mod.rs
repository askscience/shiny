//! PDF plugin tools: create/read/list/rotate/reorder/merge/delete PDFs.
//!
//! PDFs live in the plugin-owned `pdf_documents` table as real .pdf bytes
//! (parsed and edited with pdf_oxide). All work goes through the SDK's
//! synchronous `ctx.db()` accessor — no async worker threads.

use async_trait::async_trait;
use serde_json::{json, Value};

use shiny_plugin_sdk::db::Value as DbValue;
use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::outcome::ActionOutcome;
use shiny_plugin_sdk::services::PluginCtx;
use shiny_plugin_sdk::tools::{ParamHelpers, Tool, ToolRequest};

use crate::ops;

fn as_text(v: &DbValue) -> String {
    match v {
        DbValue::Text(s) => s.clone(),
        DbValue::Int(n) => n.to_string(),
        _ => String::new(),
    }
}

fn as_int(v: &DbValue) -> i64 {
    match v {
        DbValue::Int(n) => *n,
        _ => 0,
    }
}

fn as_blob(v: &DbValue) -> Vec<u8> {
    match v {
        DbValue::Blob(b) => b.clone(),
        _ => Vec::new(),
    }
}

fn last_pdf_id(ctx: &PluginCtx, user_id: &str) -> Result<Option<String>, AppError> {
    let rows = ctx.db().query(
        "SELECT id FROM pdf_documents WHERE user_id = ?1 ORDER BY updated_at DESC LIMIT 1",
        &[DbValue::text(user_id)],
    )?;
    Ok(rows.first().map(|r| as_text(&r[0])))
}

/// Resolve an optional `pdf_id` to a concrete one (most recent fallback).
fn resolve_pdf_id(ctx: &PluginCtx, user_id: &str, given: Option<String>) -> Result<String, AppError> {
    match given {
        Some(id) if !id.trim().is_empty() => Ok(id),
        _ => last_pdf_id(ctx, user_id)?
            .ok_or_else(|| AppError::BadRequest("No PDF yet — import or create one first".into())),
    }
}

/// Load a PDF's (title, bytes) for a user.
fn load_pdf(ctx: &PluginCtx, user_id: &str, id: &str) -> Result<(String, Vec<u8>), AppError> {
    let rows = ctx.db().query(
        "SELECT title, bytes FROM pdf_documents WHERE id = ?1 AND user_id = ?2",
        &[DbValue::text(id), DbValue::text(user_id)],
    )?;
    let row = rows.first().ok_or_else(|| AppError::NotFound("PDF not found".into()))?;
    Ok((as_text(&row[0]), as_blob(&row[1])))
}

/// Replace a document's bytes and refresh its cached page count.
fn commit(ctx: &PluginCtx, user_id: &str, id: &str, bytes: Vec<u8>) -> Result<usize, AppError> {
    let count = ops::page_count(&bytes)?;
    let changed = ctx.db().execute(
        "UPDATE pdf_documents SET bytes = ?1, page_count = ?2, updated_at = datetime('now') \
         WHERE id = ?3 AND user_id = ?4",
        &[DbValue::blob(bytes), DbValue::Int(count as i64), DbValue::text(id), DbValue::text(user_id)],
    )?;
    if changed == 0 {
        return Err(AppError::NotFound("PDF not found".into()));
    }
    Ok(count)
}

fn doc_summary_json(row: &[DbValue]) -> Value {
    json!({ "pdf_id": as_text(&row[0]), "title": as_text(&row[1]), "page_count": as_int(&row[2]), "updated_at": as_text(&row[3]) })
}

/* ── pdf_create ─────────────────────────────────────────────── */

pub struct PdfCreate;

#[async_trait]
impl Tool for PdfCreate {
    fn name(&self) -> &str { "pdf_create" }
    fn aliases(&self) -> &[&str] { &["create_pdf", "new_pdf"] }
    fn step_label(&self) -> &str { "Creating PDF…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `pdf_create` — Create a professionally-styled PDF. params: `{ title?: string, content: string, format?: \"html\"|\"markdown\"|\"plain\" }` — default `format:\"html\"`: `content` is semantic HTML (`h1`-`h4`, `p`, `ul`/`ol`/`li`, `table`, `strong`/`b`, `em`/`i`, `blockquote`, `pre`/`code`, `hr`, `br`) or Markdown (both accepted). The renderer styles it automatically (bold sized headings, title rule, table header rule, lists, quotes) — CSS/colors are not applied, so express importance with structure and emphasis. Use `format:\"plain\"` only when the user explicitly asks for plain/raw text. Returns the new `pdf_id`.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("Untitled");
        format!("Created PDF \"{title}\"")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let title = req.params.param_str("title")
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| "Untitled".into());
        let content = req.params.param_str("content").unwrap_or_default();
        let css = req.params.param_str("css").unwrap_or_default();
        let format = req.params.param_str("format").unwrap_or_else(|| {
            if req.params.param_bool("plain").unwrap_or(false) {
                "plain".to_string()
            } else {
                "html".to_string()
            }
        });
        let bytes = ops::create(&content, &css, &format)?;
        let count = ops::page_count(&bytes)?;
        let id = uuid::Uuid::new_v4().to_string();

        ctx.db().execute(
            "INSERT INTO pdf_documents (id, user_id, title, bytes, page_count, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'), datetime('now'))",
            &[DbValue::text(&id), DbValue::text(req.traveler_id), DbValue::text(&title), DbValue::blob(bytes), DbValue::Int(count as i64)],
        )?;

        Ok(ActionOutcome::ok("pdf_create", json!({ "pdf_id": id, "title": title, "page_count": count })))
    }
}

/* ── pdf_list ───────────────────────────────────────────────── */

pub struct PdfList;

#[async_trait]
impl Tool for PdfList {
    fn name(&self) -> &str { "pdf_list" }
    fn aliases(&self) -> &[&str] { &["list_pdfs", "pdfs"] }
    fn step_label(&self) -> &str { "Listing PDFs…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `pdf_list` — List the user's PDFs. params: `{}` — returns `pdf_id`, `title`, `page_count`, `updated_at` per document.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let n = data.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
        format!("Found {n} PDFs")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let rows = ctx.db().query(
            "SELECT id, title, page_count, updated_at FROM pdf_documents \
             WHERE user_id = ?1 ORDER BY updated_at DESC LIMIT 100",
            &[DbValue::text(req.traveler_id)],
        )?;
        let docs: Vec<Value> = rows.iter().map(|r| doc_summary_json(r)).collect();
        Ok(ActionOutcome::ok("pdf_list", json!({ "documents": docs, "count": docs.len() })))
    }
}

/* ── pdf_read ───────────────────────────────────────────────── */

pub struct PdfRead;

#[async_trait]
impl Tool for PdfRead {
    fn name(&self) -> &str { "pdf_read" }
    fn aliases(&self) -> &[&str] { &["read_pdf", "open_pdf"] }
    fn step_label(&self) -> &str { "Reading PDF…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `pdf_read` — Read a PDF's text. params: `{ pdf_id?: string, page?: number }` — omit `page` to read every page (joined). Returns `content` and `page_count`.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("PDF");
        format!("Read \"{title}\"")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let id = resolve_pdf_id(ctx, req.traveler_id, req.params.param_str("pdf_id"))?;
        let (title, bytes) = load_pdf(ctx, req.traveler_id, &id)?;
        let total = ops::page_count(&bytes)?;

        let content = match req.params.param_u32("page") {
            Some(p) => ops::extract_text(&bytes, p as usize)?,
            None => {
                let mut joined = String::new();
                for p in 0..total {
                    let t = ops::extract_text(&bytes, p)?;
                    if !t.trim().is_empty() {
                        if !joined.is_empty() { joined.push_str("\n\n"); }
                        joined.push_str(&format!("--- Page {} ---\n", p + 1));
                        joined.push_str(&t);
                    }
                }
                joined
            }
        };

        Ok(ActionOutcome::ok("pdf_read", json!({
            "pdf_id": id,
            "title": title,
            "content": content,
            "page_count": total,
        })))
    }
}

/* ── pdf_rotate ─────────────────────────────────────────────── */

pub struct PdfRotate;

#[async_trait]
impl Tool for PdfRotate {
    fn name(&self) -> &str { "pdf_rotate" }
    fn aliases(&self) -> &[&str] { &["rotate_pdf", "rotate_pages"] }
    fn step_label(&self) -> &str { "Rotating pages…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `pdf_rotate` — Rotate page(s). params: `{ pdf_id?: string, pages?: number[], degrees: number, all?: boolean }` — degrees must be a multiple of 90 (clockwise). Omit `pages` (or set `all`) to rotate every page.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("PDF");
        format!("Rotated \"{title}\"")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let id = resolve_pdf_id(ctx, req.traveler_id, req.params.param_str("pdf_id"))?;
        let degrees = req.params.param_u32("degrees").unwrap_or(90) as i32;
        if degrees % 90 != 0 {
            return Err(AppError::BadRequest("degrees must be a multiple of 90".into()));
        }
        let (title, bytes) = load_pdf(ctx, req.traveler_id, &id)?;
        let pages: Option<Vec<usize>> = req.params.get("pages").and_then(|v| v.as_array()).map(|a| {
            a.iter().filter_map(|v| v.as_u64()).map(|n| n as usize).collect()
        });
        let all = req.params.param_bool("all").unwrap_or(false);
        let new_bytes = if all || pages.as_ref().map_or(true, |p| p.is_empty()) {
            ops::rotate(&bytes, None, degrees)?
        } else {
            ops::rotate(&bytes, Some(pages.as_ref().unwrap()), degrees)?
        };
        let count = commit(ctx, req.traveler_id, &id, new_bytes)?;
        Ok(ActionOutcome::ok("pdf_rotate", json!({ "pdf_id": id, "title": title, "page_count": count })))
    }
}

/* ── pdf_delete_pages ───────────────────────────────────────── */

pub struct PdfDeletePages;

#[async_trait]
impl Tool for PdfDeletePages {
    fn name(&self) -> &str { "pdf_delete_pages" }
    fn aliases(&self) -> &[&str] { &["delete_pages", "remove_pages"] }
    fn step_label(&self) -> &str { "Deleting pages…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `pdf_delete_pages` — Remove pages (0-based indices). params: `{ pdf_id?: string, pages: number[] }`")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("PDF");
        format!("Deleted pages from \"{title}\"")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let id = resolve_pdf_id(ctx, req.traveler_id, req.params.param_str("pdf_id"))?;
        let pages: Vec<usize> = req.params.get("pages").and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_u64()).map(|n| n as usize).collect())
            .ok_or_else(|| AppError::BadRequest("pages required".into()))?;
        if pages.is_empty() {
            return Err(AppError::BadRequest("pages must not be empty".into()));
        }
        let (title, bytes) = load_pdf(ctx, req.traveler_id, &id)?;
        let new_bytes = ops::delete_pages(&bytes, &pages)?;
        let count = commit(ctx, req.traveler_id, &id, new_bytes)?;
        Ok(ActionOutcome::ok("pdf_delete_pages", json!({ "pdf_id": id, "title": title, "page_count": count })))
    }
}

/* ── pdf_reorder ────────────────────────────────────────────── */

pub struct PdfReorder;

#[async_trait]
impl Tool for PdfReorder {
    fn name(&self) -> &str { "pdf_reorder" }
    fn aliases(&self) -> &[&str] { &["reorder_pages", "move_pages"] }
    fn step_label(&self) -> &str { "Reordering pages…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `pdf_reorder` — Reorder/keep pages (0-based). params: `{ pdf_id?: string, order: number[] }` — the document becomes exactly these pages, in this order.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("PDF");
        format!("Reordered \"{title}\"")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let id = resolve_pdf_id(ctx, req.traveler_id, req.params.param_str("pdf_id"))?;
        let order: Vec<usize> = req.params.get("order").and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_u64()).map(|n| n as usize).collect())
            .ok_or_else(|| AppError::BadRequest("order required".into()))?;
        if order.is_empty() {
            return Err(AppError::BadRequest("order must not be empty".into()));
        }
        let (title, bytes) = load_pdf(ctx, req.traveler_id, &id)?;
        let new_bytes = ops::select_pages(&bytes, &order)?;
        let count = commit(ctx, req.traveler_id, &id, new_bytes)?;
        Ok(ActionOutcome::ok("pdf_reorder", json!({ "pdf_id": id, "title": title, "page_count": count })))
    }
}

/* ── pdf_merge ──────────────────────────────────────────────── */

pub struct PdfMerge;

#[async_trait]
impl Tool for PdfMerge {
    fn name(&self) -> &str { "pdf_merge" }
    fn aliases(&self) -> &[&str] { &["merge_pdfs", "combine_pdfs", "append_pdf"] }
    fn step_label(&self) -> &str { "Merging PDFs…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `pdf_merge` — Append another PDF to this one. params: `{ pdf_id?: string, other_id: string }` — `other_id`'s pages are added to the end of `pdf_id` (default: most recent PDF).")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("PDF");
        format!("Merged into \"{title}\"")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let id = resolve_pdf_id(ctx, req.traveler_id, req.params.param_str("pdf_id"))?;
        let other_id = req.params.require_str("other_id")?;
        if other_id == id {
            return Err(AppError::BadRequest("cannot merge a PDF with itself".into()));
        }
        let (title, bytes) = load_pdf(ctx, req.traveler_id, &id)?;
        let (_, other) = load_pdf(ctx, req.traveler_id, &other_id)?;
        let new_bytes = ops::merge(&bytes, &other)?;
        let count = commit(ctx, req.traveler_id, &id, new_bytes)?;
        Ok(ActionOutcome::ok("pdf_merge", json!({ "pdf_id": id, "title": title, "page_count": count })))
    }
}

/* ── pdf_delete ─────────────────────────────────────────────── */

pub struct PdfDelete;

#[async_trait]
impl Tool for PdfDelete {
    fn name(&self) -> &str { "pdf_delete" }
    fn aliases(&self) -> &[&str] { &["delete_pdf", "remove_pdf"] }
    fn step_label(&self) -> &str { "Deleting PDF…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `pdf_delete` — Delete a PDF. params: `{ pdf_id: string }`")
    }
    fn humanize(&self, _r: &str, _d: &Value) -> String {
        "PDF deleted".into()
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let id = req.params.require_str("pdf_id")?;
        let changed = ctx.db().execute(
            "DELETE FROM pdf_documents WHERE id = ?1 AND user_id = ?2",
            &[DbValue::text(&id), DbValue::text(req.traveler_id)],
        )?;
        if changed == 0 {
            return Ok(ActionOutcome::error("pdf_delete", "PDF not found"));
        }
        Ok(ActionOutcome::ok("pdf_delete", json!({ "pdf_id": id })))
    }
}

/* ── shared helpers for the editing tools ───────────────────── */

fn arr_f32(v: Option<&Value>, len: usize) -> Option<Vec<f32>> {
    let arr = v?.as_array()?;
    if arr.len() != len {
        return None;
    }
    arr.iter().map(|x| x.as_f64().map(|f| f as f32)).collect()
}

fn require_rect(params: &Value) -> Result<[f32; 4], AppError> {
    let v = arr_f32(params.get("rect"), 4)
        .ok_or_else(|| AppError::BadRequest("rect must be [x, y, width, height] in points".into()))?;
    Ok([v[0], v[1], v[2], v[3]])
}

/* ── pdf_replace_text ───────────────────────────────────────── */

pub struct PdfReplaceText;

#[async_trait]
impl Tool for PdfReplaceText {
    fn name(&self) -> &str { "pdf_replace_text" }
    fn aliases(&self) -> &[&str] { &["replace_text", "edit_pdf_text", "find_and_replace"] }
    fn step_label(&self) -> &str { "Editing PDF text…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `pdf_replace_text` — Find & replace text on a page. params: `{ pdf_id?: string, page?: number, old: string, new: string }` — replaces every occurrence of `old` with `new` on `page` (default 0).")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("PDF");
        format!("Edited text in \"{title}\"")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let id = resolve_pdf_id(ctx, req.traveler_id, req.params.param_str("pdf_id"))?;
        let page = req.params.param_u32("page").unwrap_or(0) as usize;
        let old = req.params.require_str("old")?;
        let new = req.params.param_str("new").unwrap_or_default();
        let (title, bytes) = load_pdf(ctx, req.traveler_id, &id)?;
        let (new_bytes, touched) = ops::replace_text(&bytes, page, &old, &new)?;
        let count = commit(ctx, req.traveler_id, &id, new_bytes)?;
        Ok(ActionOutcome::ok("pdf_replace_text", json!({
            "pdf_id": id, "title": title, "page": page, "replaced": touched, "page_count": count,
        })))
    }
}

/* ── pdf_annotate ───────────────────────────────────────────── */

pub struct PdfAnnotate;

#[async_trait]
impl Tool for PdfAnnotate {
    fn name(&self) -> &str { "pdf_annotate" }
    fn aliases(&self) -> &[&str] { &["add_annotation", "highlight_pdf", "add_note", "add_pdf_note", "annotate_pdf"] }
    fn step_label(&self) -> &str { "Annotating PDF…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `pdf_annotate` — Add an annotation to a page. params: `{ pdf_id?: string, page?: number, kind: string, rect: [x,y,width,height], text?: string, color?: [r,g,b] }` — `kind` is one of `highlight`, `underline`, `strikeout`, `squiggly`, `note`, `free_text`, `link`. `rect` is in PDF points (origin bottom-left; a Letter page is 612×792). For `link` the `text` is the URL; for `note`/`free_text` it is the note/box text. `color` is 0..1 RGB.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("PDF");
        format!("Annotated \"{title}\"")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let id = resolve_pdf_id(ctx, req.traveler_id, req.params.param_str("pdf_id"))?;
        let page = req.params.param_u32("page").unwrap_or(0) as usize;
        let kind = req.params.require_str("kind")?;
        let rect = require_rect(req.params)?;
        let text = req.params.param_str("text").unwrap_or_default();
        let color: Option<[f32; 3]> = arr_f32(req.params.get("color"), 3).map(|v| [v[0], v[1], v[2]]);
        let (title, bytes) = load_pdf(ctx, req.traveler_id, &id)?;
        let new_bytes = ops::annotate(&bytes, page, &kind, rect, &text, color)?;
        let count = commit(ctx, req.traveler_id, &id, new_bytes)?;
        Ok(ActionOutcome::ok("pdf_annotate", json!({
            "pdf_id": id, "title": title, "page": page, "kind": kind, "page_count": count,
        })))
    }
}

/* ── pdf_add_note ───────────────────────────────────────────── */

pub struct PdfAddNote;

#[async_trait]
impl Tool for PdfAddNote {
    fn name(&self) -> &str { "pdf_add_note" }
    fn aliases(&self) -> &[&str] { &["add_note", "note_pdf", "add_sticky_note", "comment_pdf"] }
    fn step_label(&self) -> &str { "Adding note…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `pdf_add_note` — Add a sticky note to a page. params: `{ pdf_id?: string, page?: number, text: string, x?: number, y?: number }` — `x`/`y` are PDF points from the bottom-left (default: near the top-left of the page). Easiest way to annotate without a rectangle.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("PDF");
        format!("Added a note to \"{title}\"")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let id = resolve_pdf_id(ctx, req.traveler_id, req.params.param_str("pdf_id"))?;
        let page = req.params.param_u32("page").unwrap_or(0) as usize;
        let text = req.params.require_str("text")?;
        let (title, bytes) = load_pdf(ctx, req.traveler_id, &id)?;
        let (_, h) = ops::page_size_pt(&bytes, page)?;
        let x = req.params.param_f64("x").unwrap_or(40.0) as f32;
        let y = req.params
            .param_f64("y")
            .map(|v| v as f32)
            .unwrap_or((h - 60.0).max(0.0));
        let new_bytes = ops::annotate(&bytes, page, "note", [x, y, 20.0, 20.0], &text, None)?;
        let count = commit(ctx, req.traveler_id, &id, new_bytes)?;
        Ok(ActionOutcome::ok("pdf_add_note", json!({
            "pdf_id": id, "title": title, "page": page, "page_count": count,
        })))
    }
}

/* ── pdf_watermark ──────────────────────────────────────────── */

pub struct PdfWatermark;

#[async_trait]
impl Tool for PdfWatermark {
    fn name(&self) -> &str { "pdf_watermark" }
    fn aliases(&self) -> &[&str] { &["watermark_pdf", "add_watermark"] }
    fn step_label(&self) -> &str { "Adding watermark…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `pdf_watermark` — Add a diagonal text watermark. params: `{ pdf_id?: string, text: string, pages?: number[], all?: boolean }` — omitting `pages` (or `all:true`) watermarks every page.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("PDF");
        format!("Watermarked \"{title}\"")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let id = resolve_pdf_id(ctx, req.traveler_id, req.params.param_str("pdf_id"))?;
        let text = req.params.require_str("text")?;
        let all = req.params.param_bool("all").unwrap_or(false);
        let pages: Option<Vec<usize>> = req.params.get("pages").and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_u64()).map(|n| n as usize).collect());
        let (title, bytes) = load_pdf(ctx, req.traveler_id, &id)?;
        let targets = if all { None } else { pages.as_deref() };
        let new_bytes = ops::add_watermark(&bytes, targets, &text)?;
        let count = commit(ctx, req.traveler_id, &id, new_bytes)?;
        Ok(ActionOutcome::ok("pdf_watermark", json!({
            "pdf_id": id, "title": title, "page_count": count,
        })))
    }
}
