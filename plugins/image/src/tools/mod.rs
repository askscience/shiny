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

use crate::ops;

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
        "operations required — pass an array like [{\"op\":\"grayscale\"},{\"op\":\"brightness\",\"amount\":20}]".into(),
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
    fn aliases(&self) -> &[&str] { &["edit_image", "apply_effect", "apply_filter", "transform_image"] }
    fn step_label(&self) -> &str { "Editing image…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `image_edit` — Apply one or more operations to an image. params: `{ image_id?: string, operations: [ {op, ...}, ... ] }` — operations apply in order and save. Without `image_id` edits the most recently used image. See the skills doc for the full operation list.")
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

        let row = sqlx::query_as::<_, (String, String, Vec<u8>, Vec<u8>, i64, i64, i64, i64, String)>(
            "SELECT title, id, bytes, original, width, height, orig_width, orig_height, format \
             FROM images WHERE id = ?1 AND user_id = ?2",
        )
        .bind(&image_id)
        .bind(req.traveler_id)
        .fetch_optional(ctx.pool().await)
        .await?
        .ok_or_else(|| AppError::NotFound("Image not found".into()))?;
        let (title, _id, bytes, original, width, height, ow, oh, format) = row;

        let raw = ops::to_raw(&bytes, &format)?;
        let original_raw = ops::to_raw(&original, &format)?;
        let w = width.max(0) as u32;
        let h = height.max(0) as u32;
        let (orig_w, orig_h) = if format == "rgba" {
            (ow.max(0) as u32, oh.max(0) as u32)
        } else {
            (w, h)
        };

        let (new_raw, nw, nh) = ops::apply_raw(&raw, w, h, &original_raw, orig_w, orig_h, &operations)?;

        sqlx::query(
            "UPDATE images SET bytes = ?1, width = ?2, height = ?3, format = 'rgba', \
             orig_width = ?4, orig_height = ?5, updated_at = datetime('now') \
             WHERE id = ?6 AND user_id = ?7",
        )
        .bind(&new_raw)
        .bind(nw as i64)
        .bind(nh as i64)
        .bind(orig_w as i64)
        .bind(orig_h as i64)
        .bind(&image_id)
        .bind(req.traveler_id)
        .execute(ctx.pool().await)
        .await?;

        // The window holds an in-memory session; drop it so its next render
        // reloads these freshly-written pixels.
        crate::session::clear(&image_id);

        Ok(ActionOutcome::ok(
            "image_edit",
            json!({
                "image_id": image_id,
                "title": title,
                "width": nw,
                "height": nh,
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
        crate::session::clear(&image_id);
        Ok(ActionOutcome::ok("image_delete", json!({ "image_id": image_id, "title": title.unwrap_or_default() })))
    }
}
