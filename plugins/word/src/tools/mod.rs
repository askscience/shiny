//! Word plugin tools: create/write/read/list/delete documents.
//!
//! Documents live in the core-owned `documents` table as real .odt bytes
//! (SDK `odt` codec). The plugin writes through its own SQLite pool, exactly
//! like the traveler plugin writes the shared trips/locations tables.

use async_trait::async_trait;
use serde_json::{json, Value};
use sqlx::SqlitePool;

use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::odt;
use shiny_plugin_sdk::outcome::ActionOutcome;
use shiny_plugin_sdk::services::PluginCtx;
use shiny_plugin_sdk::tools::{ParamHelpers, Tool, ToolRequest};

/// Light markdown-ish → HTML for AI-generated content.
/// Each non-empty LINE becomes a paragraph (doc_read emits one line per
/// paragraph, so doc_read → doc_write round-trips keep their structure).
pub fn content_to_html(content: &str) -> String {
    let mut html = String::new();
    for para in content.lines().map(|l| l.trim()).filter(|l| !l.is_empty()) {
        if let Some(rest) = para.strip_prefix("### ") {
            html.push_str(&format!("<h3>{}</h3>", inline_html(rest)));
        } else if let Some(rest) = para.strip_prefix("## ") {
            html.push_str(&format!("<h2>{}</h2>", inline_html(rest)));
        } else if let Some(rest) = para.strip_prefix("# ") {
            html.push_str(&format!("<h1>{}</h1>", inline_html(rest)));
        } else {
            html.push_str(&format!("<p>{}</p>", inline_html(para)));
        }
    }
    if html.is_empty() {
        html.push_str("<p></p>");
    }
    html
}

/// Inline **bold** / *italic* / `code` conversion.
fn inline_html(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    loop {
        if let Some(pos) = rest.find("**") {
            let (pre, after) = rest.split_at(pos);
            out.push_str(pre);
            let after = &after[2..];
            if let Some(end) = after.find("**") {
                let (bold, tail) = after.split_at(end);
                out.push_str(&format!("<b>{}</b>", bold));
                rest = &tail[2..];
                continue;
            }
            out.push_str("**");
            rest = after;
            continue;
        }
        if let Some(pos) = rest.find('*') {
            let (pre, after) = rest.split_at(pos);
            out.push_str(pre);
            let after = &after[1..];
            if let Some(end) = after.find('*') {
                let (ital, tail) = after.split_at(end);
                out.push_str(&format!("<i>{}</i>", ital));
                rest = &tail[1..];
                continue;
            }
            out.push('*');
            rest = after;
            continue;
        }
        out.push_str(rest);
        break;
    }
    out
}

async fn last_doc_id(pool: &SqlitePool, user_id: &str) -> Result<Option<String>, AppError> {
    Ok(sqlx::query_scalar(
        "SELECT id FROM documents WHERE user_id = ?1 ORDER BY updated_at DESC LIMIT 1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?)
}

fn doc_summary_json(row: &(String, String, String)) -> Value {
    json!({ "doc_id": row.0, "title": row.1, "updated_at": row.2 })
}

/* ── doc_create ─────────────────────────────────────────────── */

pub struct DocCreate;

#[async_trait]
impl Tool for DocCreate {
    fn name(&self) -> &str { "doc_create" }
    fn aliases(&self) -> &[&str] { &["create_document", "new_document"] }
    fn step_label(&self) -> &str { "Creating document…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `doc_create` — Create a new document. params: `{ title?: string, content?: string }` — content is plain text with `#` headings, `**bold**`, `*italic*`, paragraphs split by blank lines. Returns the new `doc_id`.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("Untitled");
        format!("Created document \"{title}\"")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let title = req
            .params
            .param_str("title")
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| "Untitled".into());
        let content = req.params.param_str("content").unwrap_or_default();
        let html = content_to_html(&content);
        let odt_bytes = odt::html_to_odt(&title, &html)?;
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
    fn aliases(&self) -> &[&str] { &["write_document", "edit_document", "update_document"] }
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
        let odt_bytes = odt::html_to_odt(&title, &html)?;
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

        let html = odt::odt_to_html(&odt_bytes)?;
        let edited = if html.contains(&old) {
            html.replacen(&old, &new, 1)
        } else if html.to_lowercase().contains(&old.to_lowercase()) {
            let lower = html.to_lowercase();
            let pos = lower.find(&old.to_lowercase()).unwrap_or(0);
            let mut s = html;
            s.replace_range(pos..pos + old.len(), &new);
            s
        } else {
            // The model must not guess: tell it to rewrite in full.
            let preview: String = odt::odt_to_plain_text(&odt_bytes)?
                .chars()
                .take(400)
                .collect();
            return Ok(ActionOutcome::error(
                "doc_edit",
                format!(
                    "Text \"{}\" not found in the document. Current content: {}",
                    old.chars().take(80).collect::<String>(),
                    preview
                ),
            ));
        };

        let new_odt = odt::html_to_odt(&title, &edited)?;
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
        let existing = odt::odt_to_html(&odt_bytes)?;
        let html = format!("{}\n{}", existing, content_to_html(&content));
        let new_odt = odt::html_to_odt(&title, &html)?;
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
    fn aliases(&self) -> &[&str] { &["read_document", "open_document"] }
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
        let text = odt::odt_to_plain_text(&odt_bytes)?;

        Ok(ActionOutcome::ok(
            "doc_read",
            json!({
                "doc_id": doc_id,
                "title": title,
                "content": text,
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