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

use crate::{layers, ops};

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
        Some("- `image_edit` — Apply one or more operations to one layer of an image. params: `{ image_id?: string, layer_id?: string, operations: [ {op, ...}, ... ] }` — operations apply in order and save. Without `layer_id` edits the topmost pixel layer; without `image_id` edits the most recently used image. See the skills doc for the full operation list.")
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

        let title = layers::load_doc(&db, req.traveler_id, &image_id)?.title;
        layers::ensure_base_layer(&db, req.traveler_id, &image_id)?;
        let target = layers::active_layer(
            &db,
            req.traveler_id,
            &image_id,
            req.params.param_str("layer_id").as_deref(),
        )?;

        let (new_raw, nw, nh) = ops::apply_raw(
            &target.bytes,
            target.w,
            target.h,
            &target.original,
            target.w,
            target.h,
            &operations,
        )?;
        layers::set_layer_pixels(
            &db,
            req.traveler_id,
            &image_id,
            &target.id,
            new_raw,
            target.original.clone(),
            nw,
            nh,
            target.x,
            target.y,
        )?;
        layers::refresh_composite(&db, req.traveler_id, &image_id)?;

        Ok(ActionOutcome::ok(
            "image_edit",
            json!({
                "image_id": image_id,
                "title": title,
                "layer_id": target.id,
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
        ctx.db().execute(
            "DELETE FROM image_layers WHERE image_id = ?1 AND user_id = ?2",
            &[shiny_plugin_sdk::db::Value::text(&image_id), shiny_plugin_sdk::db::Value::text(req.traveler_id)],
        )?;
        Ok(ActionOutcome::ok("image_delete", json!({ "image_id": image_id, "title": title.unwrap_or_default() })))
    }
}

/* ── image_layer_list ───────────────────────────────────────── */

pub struct ImageLayerList;

#[async_trait]
impl Tool for ImageLayerList {
    fn name(&self) -> &str { "image_layer_list" }
    fn aliases(&self) -> &[&str] { &["list_layers", "image_layers"] }
    fn step_label(&self) -> &str { "Listing layers…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `image_layer_list` — List an image's layer stack, bottom-to-top. params: `{ image_id?: string }` — returns `layers` (each with `layer_id`, `name`, `position`, `visible`, `opacity`, `blend_mode`, `is_group`, `group_id`) and `count`.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let n = data.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
        format!("Found {n} layers")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let image_id = resolve_image_id(ctx.pool().await, req.traveler_id, req.params.param_str("image_id"))
            .await?
            .ok_or_else(|| AppError::NotFound("Image not found".into()))?;
        let rows = layers::ensure_base_layer(ctx.db(), req.traveler_id, &image_id)?;
        let out: Vec<Value> = rows.iter().map(|l| json!({
            "layer_id": l.id,
            "name": l.name,
            "position": l.position,
            "visible": l.visible,
            "opacity": l.opacity,
            "blend_mode": l.blend.as_str(),
            "is_group": l.is_group,
            "group_id": l.group_id,
        })).collect();
        Ok(ActionOutcome::ok("image_layer_list", json!({
            "image_id": image_id,
            "layers": out,
            "count": out.len(),
        })))
    }
}

/* ── image_layer_add ────────────────────────────────────────── */

pub struct ImageLayerAdd;

#[async_trait]
impl Tool for ImageLayerAdd {
    fn name(&self) -> &str { "image_layer_add" }
    fn aliases(&self) -> &[&str] { &["add_layer", "new_layer"] }
    fn step_label(&self) -> &str { "Adding layer…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `image_layer_add` — Add a layer to an image. params: `{ image_id?: string, name?: string, group_id?: string, folder?: bool, from_image_id?: string }` — blank pixel layer by default; `folder:true` creates a folder; `from_image_id` copies another image's pixels into the new layer. Returns the new `layer_id`.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let name = data.get("name").and_then(|v| v.as_str()).unwrap_or("layer");
        format!("Added layer \"{name}\"")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let db = ctx.db();
        let image_id = resolve_image_id(ctx.pool().await, req.traveler_id, req.params.param_str("image_id"))
            .await?
            .ok_or_else(|| AppError::NotFound("Image not found".into()))?;
        let doc = layers::load_doc(&db, req.traveler_id, &image_id)?;
        let name = req.params.param_str("name").unwrap_or_else(|| "Layer".into());
        let group_id = req.params.param_str("group_id");

        if req.params.param_bool("folder").unwrap_or(false) {
            let folder_id = layers::create_group(
                &db,
                req.traveler_id,
                &image_id,
                &name,
                group_id.as_deref(),
            )?;
            return Ok(ActionOutcome::ok("image_layer_add", json!({
                "image_id": image_id, "layer_id": folder_id, "name": name, "is_group": true,
            })));
        }

        if let Some(src) = req.params.param_str("from_image_id") {
            let src_id = resolve_image_id(ctx.pool().await, req.traveler_id, Some(src))
                .await?
                .ok_or_else(|| AppError::NotFound("Source image not found".into()))?;
            let (raw, w, h) = layers::rendered(&db, req.traveler_id, &src_id)?;
            let layer_id = layers::insert_layer(
                &db, req.traveler_id, &image_id, &name, group_id.as_deref(), 0, 0, w, h, raw,
            )?;
            layers::refresh_composite(&db, req.traveler_id, &image_id)?;
            return Ok(ActionOutcome::ok("image_layer_add", json!({
                "image_id": image_id, "layer_id": layer_id, "name": name, "width": w, "height": h,
            })));
        }

        let (w, h) = (doc.width.max(1), doc.height.max(1));
        let raw = vec![0u8; (w as usize) * (h as usize) * 4];
        let layer_id = layers::insert_layer(
            &db, req.traveler_id, &image_id, &name, group_id.as_deref(), 0, 0, w, h, raw,
        )?;
        layers::refresh_composite(&db, req.traveler_id, &image_id)?;
        Ok(ActionOutcome::ok("image_layer_add", json!({
            "image_id": image_id, "layer_id": layer_id, "name": name, "width": w, "height": h,
        })))
    }
}

/* ── image_layer_update ─────────────────────────────────────── */

pub struct ImageLayerUpdate;

#[async_trait]
impl Tool for ImageLayerUpdate {
    fn name(&self) -> &str { "image_layer_update" }
    fn aliases(&self) -> &[&str] { &["update_layer", "set_layer"] }
    fn step_label(&self) -> &str { "Updating layer…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `image_layer_update` — Change a layer's name, visibility, opacity or blend mode. params: `{ image_id?: string, layer_id: string, name?: string, visible?: bool, opacity?: 0..1, blend_mode?: string, group_id?: string }`. Blend modes: normal, multiply, screen, overlay, darken, lighten, difference, color_dodge, color_burn, add, subtract.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let name = data.get("name").and_then(|v| v.as_str()).unwrap_or("layer");
        format!("Updated layer \"{name}\"")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let db = ctx.db();
        let image_id = resolve_image_id(ctx.pool().await, req.traveler_id, req.params.param_str("image_id"))
            .await?
            .ok_or_else(|| AppError::NotFound("Image not found".into()))?;
        let layer_id = req.params.param_str("layer_id")
            .ok_or_else(|| AppError::BadRequest("layer_id required".into()))?;

        let patch = layers::LayerPatch {
            name: req.params.param_str("name"),
            visible: req.params.param_bool("visible"),
            opacity: req.params.param_f64("opacity").map(|v| v.clamp(0.0, 1.0) as f32),
            blend_mode: req.params.param_str("blend_mode"),
            x: req.params.param_f64("x").map(|v| v as i32),
            y: req.params.param_f64("y").map(|v| v as i32),
            group_id: req.params.param_str("group_id").map(Some),
        };
        layers::update_layer(&db, req.traveler_id, &image_id, &layer_id, &patch)?;
        layers::refresh_composite(&db, req.traveler_id, &image_id)?;

        let target = layers::load_layers(&db, req.traveler_id, &image_id)?
            .into_iter()
            .find(|l| l.id == layer_id)
            .ok_or_else(|| AppError::NotFound("Layer not found".into()))?;
        Ok(ActionOutcome::ok("image_layer_update", json!({
            "image_id": image_id,
            "layer_id": layer_id,
            "name": target.name,
            "visible": target.visible,
            "opacity": target.opacity,
            "blend_mode": target.blend.as_str(),
        })))
    }
}

/* ── image_layer_delete ─────────────────────────────────────── */

pub struct ImageLayerDelete;

#[async_trait]
impl Tool for ImageLayerDelete {
    fn name(&self) -> &str { "image_layer_delete" }
    fn aliases(&self) -> &[&str] { &["delete_layer", "remove_layer"] }
    fn step_label(&self) -> &str { "Deleting layer…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `image_layer_delete` — Delete a layer. params: `{ image_id?: string, layer_id: string, confirm: true }` — requires `confirm:true`.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let name = data.get("name").and_then(|v| v.as_str()).unwrap_or("layer");
        format!("Deleted layer \"{name}\"")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let db = ctx.db();
        let image_id = resolve_image_id(ctx.pool().await, req.traveler_id, req.params.param_str("image_id"))
            .await?
            .ok_or_else(|| AppError::NotFound("Image not found".into()))?;
        let layer_id = req.params.param_str("layer_id")
            .ok_or_else(|| AppError::BadRequest("layer_id required".into()))?;
        if !req.params.param_bool("confirm").unwrap_or(false) {
            return Ok(ActionOutcome::error(
                "image_layer_delete",
                "refusing: deleting a layer is permanent. Only call image_layer_delete with \
                 {\"confirm\":true} when the user explicitly asks.",
            ));
        }
        let name = layers::load_layers(&db, req.traveler_id, &image_id)?
            .into_iter()
            .find(|l| l.id == layer_id)
            .map(|l| l.name)
            .unwrap_or_default();
        layers::delete_layer(&db, req.traveler_id, &image_id, &layer_id)?;
        layers::refresh_composite(&db, req.traveler_id, &image_id)?;
        Ok(ActionOutcome::ok("image_layer_delete", json!({
            "image_id": image_id, "layer_id": layer_id, "name": name,
        })))
    }
}

/* ── image_layer_merge ──────────────────────────────────────── */

pub struct ImageLayerMerge;

#[async_trait]
impl Tool for ImageLayerMerge {
    fn name(&self) -> &str { "image_layer_merge" }
    fn aliases(&self) -> &[&str] { &["merge_layer", "merge_down"] }
    fn step_label(&self) -> &str { "Merging layer…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `image_layer_merge` — Merge a layer into the sibling directly beneath it. params: `{ image_id?: string, layer_id: string }`.")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let id = data.get("merged").and_then(|v| v.as_str()).unwrap_or("layer");
        format!("Merged layer {id}")
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let image_id = resolve_image_id(ctx.pool().await, req.traveler_id, req.params.param_str("image_id"))
            .await?
            .ok_or_else(|| AppError::NotFound("Image not found".into()))?;
        let layer_id = req.params.param_str("layer_id")
            .ok_or_else(|| AppError::BadRequest("layer_id required".into()))?;
        layers::merge_down(ctx.db(), req.traveler_id, &image_id, &layer_id)?;
        layers::refresh_composite(ctx.db(), req.traveler_id, &image_id)?;
        Ok(ActionOutcome::ok("image_layer_merge", json!({
            "image_id": image_id, "merged": layer_id,
        })))
    }
}

/* ── image_flatten ──────────────────────────────────────────── */

pub struct ImageFlatten;

#[async_trait]
impl Tool for ImageFlatten {
    fn name(&self) -> &str { "image_flatten" }
    fn aliases(&self) -> &[&str] { &["flatten_image", "flatten_layers"] }
    fn step_label(&self) -> &str { "Flattening image…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `image_flatten` — Flatten every layer and folder into one layer. params: `{ image_id?: string }`.")
    }
    fn humanize(&self, _r: &str, _data: &Value) -> String {
        "Flattened the image".into()
    }

    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let image_id = resolve_image_id(ctx.pool().await, req.traveler_id, req.params.param_str("image_id"))
            .await?
            .ok_or_else(|| AppError::NotFound("Image not found".into()))?;
        layers::ensure_base_layer(ctx.db(), req.traveler_id, &image_id)?;
        layers::flatten(ctx.db(), req.traveler_id, &image_id)?;
        let (_, w, h) = layers::refresh_composite(ctx.db(), req.traveler_id, &image_id)?;
        Ok(ActionOutcome::ok("image_flatten", json!({
            "image_id": image_id, "width": w, "height": h,
        })))
    }
}
