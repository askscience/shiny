//! Image plugin tools: list/get/edit/delete images.
//!
//! Images live in the plugin-owned `images` table; the tools write through the
//! plugin's own SQLite pool (`ctx.pool()`), and apply edits via `crate::ops`.

use async_trait::async_trait;
use serde_json::{json, Value};
use sqlx::SqlitePool;

use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::outcome::ActionOutcome;
use shiny_plugin_sdk::services::PluginCtx;
use shiny_plugin_sdk::tools::{ParamHelpers, Tool, ToolRequest};

use crate::doc;

type MetaRow = (String, String, i64, i64, String); // id, title, width, height, updated_at

fn meta_json(r: &MetaRow) -> Value {
    json!({
        "image_id": r.0,
        "title": r.1,
        "width": r.2,
        "height": r.3,
        "updated_at": r.4,
    })
}

/// Resolve an `image_id` param to a real id (accepts UUID or exact title).
async fn resolve_image_id(
    pool: &SqlitePool,
    user_id: &str,
    id_or_title: Option<String>,
) -> Result<Option<String>, AppError> {
    let Some(v) = id_or_title.filter(|s| !s.trim().is_empty()) else {
        return Ok(None);
    };
    let v = v.trim();

    let by_id: Option<String> = sqlx::query_scalar(
        "SELECT id FROM images WHERE id = ?1 AND user_id = ?2",
    )
    .bind(v)
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    if by_id.is_some() {
        return Ok(by_id);
    }

    let by_title: Option<String> = sqlx::query_scalar(
        "SELECT id FROM images WHERE lower(title) = lower(?1) AND user_id = ?2 \
         ORDER BY updated_at DESC LIMIT 1",
    )
    .bind(v)
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    Ok(by_title)
}

/// Normalize the operations the LLM emits into an ordered list.
fn operations_param(req: &ToolRequest<'_>) -> Result<Vec<Value>, AppError> {
    if let Some(arr) = req.params.get("operations").and_then(|v| v.as_array()) {
        return Ok(arr.clone());
    }
    if let Some(obj) = req.params.get("operation") {
        return Ok(vec![obj.clone()]);
    }
    Err(AppError::BadRequest(
        "operations required — pass an array like [{\"op\":\"brightness\",\"amount\":20}]".into(),
    ))
}

/* ── image_list ─────────────────────────────────────────────── */

pub struct ImageList;

#[async_trait]
impl Tool for ImageList {
    fn name(&self) -> &str { "image_list" }
    fn aliases(&self) -> &[&str] { &["list_images", "images"] }
    fn step_label(&self) -> &str { "Listing images…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `image_list` — List the user's images. params: `{}` — returns `images` (each with `image_id`, `title`, `width`, `height`, `updated_at`) and `count`.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let n = data.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
        format!("Found {n} images")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let rows = sqlx::query_as::<_, MetaRow>(
            "SELECT id, title, width, height, updated_at FROM images \
             WHERE user_id = ?1 ORDER BY updated_at DESC LIMIT 100",
        )
        .bind(req.traveler_id)
        .fetch_all(ctx.pool().await)
        .await?;

        let images: Vec<Value> = rows.iter().map(meta_json).collect();
        Ok(ActionOutcome::ok("image_list", json!({ "images": images, "count": images.len() })))
    }
}

/* ── image_get ──────────────────────────────────────────────── */

pub struct ImageGet;

#[async_trait]
impl Tool for ImageGet {
    fn name(&self) -> &str { "image_get" }
    fn aliases(&self) -> &[&str] { &["get_image", "image_info"] }
    fn step_label(&self) -> &str { "Reading image…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `image_get` — Read one image's metadata. params: `{ image_id?: string }` — returns `image_id`, `title`, `width`, `height`, `updated_at`. Without `image_id` reads the most recently used image.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("Image");
        format!("Read \"{title}\"")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let image_id = resolve_image_id(ctx.pool().await, req.traveler_id, req.params.param_str("image_id"))
            .await?
            .ok_or_else(|| AppError::NotFound("Image not found".into()))?;

        let row = sqlx::query_as::<_, MetaRow>(
            "SELECT id, title, width, height, updated_at FROM images WHERE id = ?1 AND user_id = ?2",
        )
        .bind(&image_id)
        .bind(req.traveler_id)
        .fetch_optional(ctx.pool().await)
        .await?
        .ok_or_else(|| AppError::NotFound("Image not found".into()))?;

        Ok(ActionOutcome::ok("image_get", meta_json(&row)))
    }
}

/* ── image_edit ─────────────────────────────────────────────── */

pub struct ImageEdit;

#[async_trait]
impl Tool for ImageEdit {
    fn name(&self) -> &str { "image_edit" }
    fn aliases(&self) -> &[&str] { &["apply_effect", "apply_filter"] }
    fn step_label(&self) -> &str { "Adjusting image…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `image_edit` — Apply one or more adjustments, looks or effects to an image. A lone `look` operation (param `name`, e.g. portra) selects a look — the picture renders with it on top, picking another look replaces it, and omitting `name` clears it. Any other operation edits the visible picture, baking the selected look in first, so real edits accumulate. params: `{ image_id?: string, operations: [ {op, ...}, ... ] }` — operations apply in order and save. Without `image_id` edits the most recently used image. These are deterministic edits — for AI re-renders of a photo use the core `generate_image` tool with an `image` path. See the skills doc for the operation and look list.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("Image");
        let n = data.get("operations_applied").and_then(|v| v.as_u64()).unwrap_or(0);
        format!("Applied {n} operations to \"{title}\"")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let operations = operations_param(&req)?;
        let image_id = resolve_image_id(ctx.pool().await, req.traveler_id, req.params.param_str("image_id"))
            .await?
            .ok_or_else(|| AppError::NotFound("Image not found".into()))?;
        let db = ctx.db();

        let title: Option<String> = sqlx::query_scalar(
            "SELECT title FROM images WHERE id = ?1 AND user_id = ?2",
        )
        .bind(&image_id)
        .bind(req.traveler_id)
        .fetch_optional(ctx.pool().await)
        .await?;
        let title = title.ok_or_else(|| AppError::NotFound("Image not found".into()))?;

        let res = doc::apply_operations(&db, req.traveler_id, &image_id, &operations, true)?;

        Ok(ActionOutcome::ok(
            "image_edit",
            json!({
                "image_id": image_id,
                "title": title,
                "width": res.w,
                "height": res.h,
                "look": res.look,
                "operations_applied": operations.len(),
            }),
        ))
    }
}

/* ── image_delete ───────────────────────────────────────────── */

pub struct ImageDelete;

#[async_trait]
impl Tool for ImageDelete {
    fn name(&self) -> &str { "image_delete" }
    fn aliases(&self) -> &[&str] { &["delete_image", "remove_image"] }
    fn step_label(&self) -> &str { "Deleting image…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `image_delete` — Permanently delete an image. params: `{ image_id: string, confirm: true }` — requires `confirm:true`.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("Image");
        format!("Deleted \"{title}\"")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let image_id = resolve_image_id(ctx.pool().await, req.traveler_id, req.params.param_str("image_id"))
            .await?
            .ok_or_else(|| AppError::NotFound("Image not found".into()))?;

        if !req.params.param_bool("confirm").unwrap_or(false) {
            return Ok(ActionOutcome::error(
                "image_delete",
                "refusing: deleting an image is permanent. Only call image_delete with \
                 {\"confirm\":true} when the user explicitly asks to delete the image.",
            ));
        }

        let title: Option<String> = sqlx::query_scalar(
            "SELECT title FROM images WHERE id = ?1 AND user_id = ?2",
        )
        .bind(&image_id)
        .bind(req.traveler_id)
        .fetch_optional(ctx.pool().await)
        .await?;

        let result = sqlx::query("DELETE FROM images WHERE id = ?1 AND user_id = ?2")
            .bind(&image_id)
            .bind(req.traveler_id)
            .execute(ctx.pool().await)
            .await?;
        if result.rows_affected() == 0 {
            return Ok(ActionOutcome::error("image_delete", "Image not found"));
        }
        Ok(ActionOutcome::ok("image_delete", json!({ "image_id": image_id, "title": title.unwrap_or_default() })))
    }
}
