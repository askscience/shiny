//! Impress plugin tools: create/write/edit/read/list/delete presentations.
//!
//! Decks live in the core-owned `presentations` table as a JSON array of the
//! SDK `Slide` model. The plugin writes through its own SQLite pool
//! (`ctx.pool()`), exactly like the word/calc plugins write the shared
//! `documents`/`spreadsheets` tables. Real `.odp` bytes are produced by the
//! core `/api/presentations/:id/export` route via the SDK `odp` codec.

use async_trait::async_trait;
use serde_json::{json, Value};
use sqlx::SqlitePool;

use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::odp::{normalize_layout, normalize_theme, Slide};
use shiny_plugin_sdk::outcome::ActionOutcome;
use shiny_plugin_sdk::services::PluginCtx;
use shiny_plugin_sdk::tools::{ParamHelpers, Tool, ToolRequest};

const MAX_SLIDES: usize = 200;
const MAX_TEXT_LEN: usize = 10_000;

async fn last_deck_id(pool: &SqlitePool, user_id: &str) -> Result<Option<String>, AppError> {
    Ok(sqlx::query_scalar(
        "SELECT id FROM presentations WHERE user_id = ?1 ORDER BY updated_at DESC LIMIT 1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?)
}

/* ── slide coercion (robust against whatever JSON shape the LLM emits) ── */

fn str_field(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Coerce a JSON value to a list of strings. Accepts a plain string, an array
/// of strings (or scalars), or a single number/bool.
fn coerce_string_list(v: &Value) -> Vec<String> {
    match v {
        Value::Array(arr) => arr.iter().map(str_field).collect(),
        Value::Null => Vec::new(),
        other => {
            let s = str_field(other);
            if s.trim().is_empty() {
                Vec::new()
            } else {
                vec![s]
            }
        }
    }
}

/// Coerce the `columns` value: an array of arrays of strings (each column is a
/// list of bullets). A flat array of strings is treated as a single column.
fn coerce_columns(v: &Value) -> Vec<Vec<String>> {
    match v {
        Value::Array(arr) => {
            // If the first element is itself an array, treat each as a column;
            // otherwise treat the whole thing as one column.
            let nested = arr.iter().any(|x| x.is_array());
            if nested {
                arr.iter().map(coerce_string_list).filter(|c| !c.is_empty()).collect()
            } else {
                let col = coerce_string_list(v);
                if col.is_empty() { Vec::new() } else { vec![col] }
            }
        }
        Value::Null => Vec::new(),
        other => {
            let col = coerce_string_list(other);
            if col.is_empty() { Vec::new() } else { vec![col] }
        }
    }
}

fn coerce_slide(v: &Value) -> Slide {
    let obj = v.as_object();
    let get = |key: &str| obj.and_then(|o| o.get(key)).cloned().unwrap_or(Value::Null);

    let layout = normalize_layout(
        get("layout").as_str().unwrap_or("content"),
    );
    Slide {
        layout,
        title: str_field(&get("title")),
        subtitle: str_field(&get("subtitle")),
        bullets: coerce_string_list(&get("bullets")),
        columns: coerce_columns(&get("columns")),
        body: str_field(&get("body")),
        attribution: str_field(&get("attribution")),
        notes: str_field(&get("notes")),
    }
}

/// Parse the `slides` parameter. Accepts an array of slide objects, or a
/// single slide object (treated as a one-slide deck).
fn coerce_slides(value: Option<&Value>) -> Result<Vec<Slide>, AppError> {
    let Some(v) = value else {
        return Ok(Vec::new());
    };
    let slides: Vec<Slide> = match v {
        Value::Array(arr) => arr.iter().map(coerce_slide).collect(),
        Value::Null => Vec::new(),
        _ => vec![coerce_slide(v)],
    };

    if slides.len() > MAX_SLIDES {
        return Err(AppError::BadRequest(format!(
            "Too many slides (max {MAX_SLIDES})"
        )));
    }
    for s in &slides {
        for t in [&s.title, &s.subtitle, &s.body, &s.attribution, &s.notes] {
            if t.chars().count() > MAX_TEXT_LEN {
                return Err(AppError::BadRequest(format!(
                    "Slide text is too long (max {MAX_TEXT_LEN} chars per field)"
                )));
            }
        }
        if s.bullets.iter().chain(s.columns.iter().flatten())
            .any(|b| b.chars().count() > MAX_TEXT_LEN)
        {
            return Err(AppError::BadRequest(format!(
                "A bullet is too long (max {MAX_TEXT_LEN} chars)"
            )));
        }
    }
    Ok(slides)
}

/* ── slide_create ───────────────────────────────────────────── */

pub struct SlideCreate;

#[async_trait]
impl Tool for SlideCreate {
    fn name(&self) -> &str { "slide_create" }
    fn aliases(&self) -> &[&str] { &["create_presentation", "new_presentation", "new_deck"] }
    fn step_label(&self) -> &str { "Building slides…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `slide_create` — Create a new presentation. params: `{ title?: string, theme?: string, slides?: [slide, …] }` — `slides` optionally seeds the deck so one call creates AND fills it; returns the new `deck_id`. Themes: aurora|slate|ocean|mono|ember. Slide layouts: title|section|content|two-column|quote|blank.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("Untitled");
        let n = data.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
        if n > 0 {
            format!("Created presentation \"{title}\" with {n} slides")
        } else {
            format!("Created presentation \"{title}\"")
        }
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let title = req
            .params
            .param_str("title")
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| "Untitled".into());
        let theme = normalize_theme(&req.params.param_str("theme").unwrap_or_else(|| "aurora".into()));
        let slides = coerce_slides(req.params.get("slides"))?;
        let count = slides.len();
        let slides_json = serde_json::to_string(&slides)?;
        let id = uuid::Uuid::new_v4().to_string();

        sqlx::query(
            "INSERT INTO presentations (id, user_id, title, slides, theme, aspect, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, '16x9', datetime('now'), datetime('now'))",
        )
        .bind(&id)
        .bind(req.traveler_id)
        .bind(&title)
        .bind(&slides_json)
        .bind(&theme)
        .execute(ctx.pool().await)
        .await?;

        Ok(ActionOutcome::ok(
            "slide_create",
            json!({ "deck_id": id, "title": title, "theme": theme, "count": count }),
        ))
    }
}

/* ── slide_write ────────────────────────────────────────────── */

pub struct SlideWrite;

#[async_trait]
impl Tool for SlideWrite {
    fn name(&self) -> &str { "slide_write" }
    fn aliases(&self) -> &[&str] { &["write_presentation", "set_slides", "rewrite_deck"] }
    fn step_label(&self) -> &str { "Writing slides…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `slide_write` — Replace the ENTIRE slide list of a presentation (and optionally its title/theme). params: `{ deck_id?: string, title?: string, theme?: string, slides: [slide, …] }` — without `deck_id` targets the most recently used presentation. Only use for full rewrites; always pass the complete slide list.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("presentation");
        let n = data.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
        format!("Wrote {n} slides to \"{title}\"")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let slides = coerce_slides(req.params.get("slides"))?;
        let count = slides.len();
        let deck_id = match req.params.param_str("deck_id") {
            Some(id) if !id.trim().is_empty() => id,
            _ => last_deck_id(ctx.pool().await, req.traveler_id)
                .await?
                .ok_or_else(|| {
                    AppError::BadRequest("No presentation yet — call slide_create first".into())
                })?,
        };

        let current: (String, String) =
            sqlx::query_as("SELECT title, theme FROM presentations WHERE id = ?1 AND user_id = ?2")
                .bind(&deck_id)
                .bind(req.traveler_id)
                .fetch_optional(ctx.pool().await)
                .await?
                .ok_or_else(|| AppError::NotFound("Presentation not found".into()))?;

        let new_title = match req.params.param_str("title") {
            Some(t) if !t.trim().is_empty() => t.trim().chars().take(120).collect::<String>(),
            _ => current.0,
        };
        let new_theme = match req.params.param_str("theme") {
            Some(t) if !t.trim().is_empty() => normalize_theme(&t),
            _ => current.1,
        };
        let slides_json = serde_json::to_string(&slides)?;

        let result = sqlx::query(
            "UPDATE presentations SET title = ?1, theme = ?2, slides = ?3, updated_at = datetime('now') \
             WHERE id = ?4 AND user_id = ?5",
        )
        .bind(&new_title)
        .bind(&new_theme)
        .bind(&slides_json)
        .bind(&deck_id)
        .bind(req.traveler_id)
        .execute(ctx.pool().await)
        .await?;
        if result.rows_affected() == 0 {
            return Err(AppError::NotFound("Presentation not found".into()));
        }

        Ok(ActionOutcome::ok(
            "slide_write",
            json!({ "deck_id": deck_id, "title": new_title, "count": count }),
        ))
    }
}

/* ── slide_edit ─────────────────────────────────────────────── */

pub struct SlideEdit;

#[async_trait]
impl Tool for SlideEdit {
    fn name(&self) -> &str { "slide_edit" }
    fn aliases(&self) -> &[&str] {
        &["edit_presentation", "update_slide", "add_slide", "change_slide", "modify_slide"]
    }
    fn step_label(&self) -> &str { "Editing slides…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `slide_edit` — Change ONE slide in a presentation. params: `{ deck_id?: string, slide: slide, index?: number }` — replaces the slide at `index` (0-based); if `index` is omitted or out of range, the slide is appended at the end. Use for \"change slide 2\", \"add a closing slide\", etc. For a full rewrite use `slide_write`.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("presentation");
        format!("Edited \"{title}\"")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let slide_value = req
            .params
            .get("slide")
            .cloned()
            .ok_or_else(|| AppError::BadRequest("slide required — the slide object".into()))?;
        let new_slide = coerce_slide(&slide_value);

        let deck_id = match req.params.param_str("deck_id") {
            Some(id) if !id.trim().is_empty() => id,
            _ => last_deck_id(ctx.pool().await, req.traveler_id)
                .await?
                .ok_or_else(|| {
                    AppError::BadRequest("No presentation yet — call slide_create first".into())
                })?,
        };

        let row = sqlx::query_as::<_, (String, String)>(
            "SELECT title, slides FROM presentations WHERE id = ?1 AND user_id = ?2",
        )
        .bind(&deck_id)
        .bind(req.traveler_id)
        .fetch_optional(ctx.pool().await)
        .await?
        .ok_or_else(|| AppError::NotFound("Presentation not found".into()))?;
        let (title, slides_json) = row;

        let mut slides: Vec<Slide> = if slides_json.trim().is_empty() {
            Vec::new()
        } else {
            serde_json::from_str(&slides_json).unwrap_or_default()
        };

        let index = req.params.param_u32("index").map(|i| i as usize);
        match index {
            Some(i) if i < slides.len() => slides[i] = new_slide,
            _ => slides.push(new_slide),
        }
        if slides.len() > MAX_SLIDES {
            return Err(AppError::BadRequest(format!(
                "Too many slides (max {MAX_SLIDES})"
            )));
        }

        let slides_json = serde_json::to_string(&slides)?;
        sqlx::query(
            "UPDATE presentations SET slides = ?1, updated_at = datetime('now') \
             WHERE id = ?2 AND user_id = ?3",
        )
        .bind(&slides_json)
        .bind(&deck_id)
        .bind(req.traveler_id)
        .execute(ctx.pool().await)
        .await?;

        Ok(ActionOutcome::ok(
            "slide_edit",
            json!({ "deck_id": deck_id, "title": title, "count": slides.len() }),
        ))
    }
}

/* ── slide_read ─────────────────────────────────────────────── */

pub struct SlideRead;

#[async_trait]
impl Tool for SlideRead {
    fn name(&self) -> &str { "slide_read" }
    fn aliases(&self) -> &[&str] { &["read_presentation", "open_presentation", "get_presentation"] }
    fn step_label(&self) -> &str { "Reading slides…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `slide_read` — Read a presentation's full slide model. params: `{ deck_id?: string }` — returns `deck_id`, `title`, `theme`, and `slides` (the exact array shape to write back). Without `deck_id` reads the most recently used presentation.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("presentation");
        let n = data.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
        format!("Read \"{title}\" ({n} slides)")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let deck_id = match req.params.param_str("deck_id") {
            Some(id) if !id.trim().is_empty() => id,
            _ => last_deck_id(ctx.pool().await, req.traveler_id)
                .await?
                .ok_or_else(|| {
                    AppError::BadRequest("No presentation yet — call slide_create first".into())
                })?,
        };

        let row = sqlx::query_as::<_, (String, String, String)>(
            "SELECT title, slides, theme FROM presentations WHERE id = ?1 AND user_id = ?2",
        )
        .bind(&deck_id)
        .bind(req.traveler_id)
        .fetch_optional(ctx.pool().await)
        .await?
        .ok_or_else(|| AppError::NotFound("Presentation not found".into()))?;
        let (title, slides_json, theme) = row;

        let slides: Vec<Slide> = if slides_json.trim().is_empty() {
            Vec::new()
        } else {
            serde_json::from_str(&slides_json).unwrap_or_default()
        };

        Ok(ActionOutcome::ok(
            "slide_read",
            json!({
                "deck_id": deck_id,
                "title": title,
                "theme": theme,
                "slides": slides,
                "count": slides.len(),
                "hint": "To change one slide use slide_edit {slide,index}; to add a slide use slide_edit {slide} (no index appends); use slide_write only for a complete rewrite.",
            }),
        ))
    }
}

/* ── slide_list ─────────────────────────────────────────────── */

pub struct SlideList;

#[async_trait]
impl Tool for SlideList {
    fn name(&self) -> &str { "slide_list" }
    fn aliases(&self) -> &[&str] { &["list_presentations", "presentations", "list_decks"] }
    fn step_label(&self) -> &str { "Listing presentations…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `slide_list` — List the user's presentations. params: `{}` — returns `deck_id`, `title`, `slide_count`, `updated_at` per presentation.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let n = data.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
        format!("Found {n} presentations")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let rows = sqlx::query_as::<_, (String, String, String, String)>(
            "SELECT id, title, slides, updated_at FROM presentations \
             WHERE user_id = ?1 ORDER BY updated_at DESC LIMIT 100",
        )
        .bind(req.traveler_id)
        .fetch_all(ctx.pool().await)
        .await?;

        let decks: Vec<Value> = rows
            .iter()
            .map(|(id, title, slides_json, updated_at)| {
                let count: i64 = if slides_json.trim().is_empty() {
                    0
                } else {
                    serde_json::from_str::<Vec<Value>>(slides_json)
                        .map(|s| s.len() as i64)
                        .unwrap_or(0)
                };
                json!({ "deck_id": id, "title": title, "slide_count": count, "updated_at": updated_at })
            })
            .collect();

        Ok(ActionOutcome::ok(
            "slide_list",
            json!({ "presentations": decks, "count": decks.len() }),
        ))
    }
}

/* ── slide_delete ───────────────────────────────────────────── */

pub struct SlideDelete;

#[async_trait]
impl Tool for SlideDelete {
    fn name(&self) -> &str { "slide_delete" }
    fn aliases(&self) -> &[&str] { &["delete_presentation", "remove_presentation", "delete_deck"] }
    fn step_label(&self) -> &str { "Deleting presentation…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `slide_delete` — Permanently delete a presentation (needs `{\"confirm\":true}`). params: `{ deck_id: string, confirm: true }`")
    }
    fn humanize(&self, _r: &str, _d: &Value) -> String {
        "Presentation deleted".into()
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let deck_id = req.params.require_str("deck_id")?;

        // Deleting a presentation is permanent and irreversible — mirror the
        // calc_delete guard so accidental "delete the content" requests fail.
        if !req.params.param_bool("confirm").unwrap_or(false) {
            return Ok(ActionOutcome::error(
                "slide_delete",
                "refusing: deleting a presentation is permanent and cannot be undone. Only call \
                 slide_delete with {\"confirm\":true} when the user explicitly asks to delete the \
                 whole presentation/deck.",
            ));
        }

        let result = sqlx::query("DELETE FROM presentations WHERE id = ?1 AND user_id = ?2")
            .bind(&deck_id)
            .bind(req.traveler_id)
            .execute(ctx.pool().await)
            .await?;
        if result.rows_affected() == 0 {
            return Ok(ActionOutcome::error("slide_delete", "Presentation not found"));
        }
        Ok(ActionOutcome::ok("slide_delete", json!({ "deck_id": deck_id })))
    }
}
