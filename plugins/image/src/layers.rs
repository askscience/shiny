//! Layer persistence and document compositing.
//!
//! `images` is a document; `image_layers` is its bottom-to-top stack. This
//! module loads the stack, builds the compositor tree, and owns the layer
//! mutations (create, update, delete, reorder, duplicate, merge, flatten).
//! Every mutation that changes pixels calls [`refresh_composite`] so the
//! `images` row keeps a usable flattened cache.

use shiny_plugin_sdk::db::{Db, Value};
use shiny_plugin_sdk::errors::AppError;

use crate::blend::BlendMode;
use crate::composite::{self, Group, Node, Raster};

/// One row of `image_layers`.
#[derive(Debug, Clone)]
pub struct LayerRow {
    pub id: String,
    pub image_id: String,
    pub name: String,
    pub position: i64,
    pub visible: bool,
    pub opacity: f32,
    pub blend: BlendMode,
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub bytes: Vec<u8>,
    pub original: Vec<u8>,
    pub mask: Option<Vec<u8>>,
    pub mask_w: u32,
    pub mask_h: u32,
    pub mask_enabled: bool,
    pub is_group: bool,
    pub group_id: Option<String>,
}

impl LayerRow {
    fn to_raster(&self) -> Raster<'_> {
        Raster {
            pixels: &self.bytes,
            w: self.w,
            h: self.h,
            x: self.x,
            y: self.y,
            opacity: self.opacity,
            blend: self.blend,
            visible: self.visible,
            mask: self.mask.as_deref(),
            mask_w: self.mask_w,
            mask_h: self.mask_h,
            mask_enabled: self.mask_enabled,
        }
    }
}

/// Document canvas + flattened cache fields.
pub struct DocMeta {
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub bytes: Vec<u8>,
    pub original: Vec<u8>,
    pub orig_w: u32,
    pub orig_h: u32,
    pub format: String,
}

fn as_text(v: &Value) -> String {
    match v {
        Value::Text(s) => s.clone(),
        Value::Int(n) => n.to_string(),
        _ => String::new(),
    }
}

fn as_int(v: &Value) -> i64 {
    match v {
        Value::Int(n) => *n,
        _ => 0,
    }
}

fn as_f64(v: &Value) -> f64 {
    match v {
        Value::Int(n) => *n as f64,
        Value::Text(s) => s.parse().unwrap_or(0.0),
        _ => 0.0,
    }
}

fn as_blob(v: &Value) -> Vec<u8> {
    match v {
        Value::Blob(b) => b.clone(),
        _ => Vec::new(),
    }
}

fn opt_blob(v: &Value) -> Option<Vec<u8>> {
    match v {
        Value::Blob(b) if !b.is_empty() => Some(b.clone()),
        _ => None,
    }
}

fn as_bool(v: &Value) -> bool {
    as_int(v) != 0
}

/// A nullable text column: `NULL`/empty become `None`.
fn as_opt_text(v: &Value) -> Option<String> {
    if matches!(v, Value::Null) {
        return None;
    }
    let s = as_text(v);
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/* ── Reads ──────────────────────────────────────────────────────── */

/// Load the document metadata (with ownership check).
pub fn load_doc(db: &Db, uid: &str, image_id: &str) -> Result<DocMeta, AppError> {
    let rows = db.query(
        "SELECT title, width, height, bytes, original, orig_width, orig_height, format \
         FROM images WHERE id = ?1 AND user_id = ?2",
        &[Value::text(image_id), Value::text(uid)],
    )?;
    let r = rows
        .first()
        .ok_or_else(|| AppError::NotFound("Image not found".into()))?;
    Ok(DocMeta {
        title: as_text(&r[0]),
        width: as_int(&r[1]).max(0) as u32,
        height: as_int(&r[2]).max(0) as u32,
        bytes: as_blob(&r[3]),
        original: as_blob(&r[4]),
        orig_w: as_int(&r[5]).max(0) as u32,
        orig_h: as_int(&r[6]).max(0) as u32,
        format: as_text(&r[7]),
    })
}

/// Load a document's layers, bottom-to-top within each parent.
pub fn load_layers(db: &Db, uid: &str, image_id: &str) -> Result<Vec<LayerRow>, AppError> {
    let rows = db.query(
        "SELECT id, image_id, name, position, visible, opacity, blend_mode, x, y, \
                width, height, bytes, original, mask, mask_width, mask_height, \
                mask_enabled, is_group, group_id \
         FROM image_layers WHERE image_id = ?1 AND user_id = ?2 \
         ORDER BY position ASC, rowid ASC",
        &[Value::text(image_id), Value::text(uid)],
    )?;
    Ok(rows.iter().map(|r| row_from_values(r)).collect())
}

fn row_from_values(r: &[Value]) -> LayerRow {
    LayerRow {
        id: as_text(&r[0]),
        image_id: as_text(&r[1]),
        name: as_text(&r[2]),
        position: as_int(&r[3]),
        visible: as_bool(&r[4]),
        opacity: as_f64(&r[5]).clamp(0.0, 1.0) as f32,
        blend: BlendMode::parse(&as_text(&r[6])),
        x: as_int(&r[7]) as i32,
        y: as_int(&r[8]) as i32,
        w: as_int(&r[9]).max(0) as u32,
        h: as_int(&r[10]).max(0) as u32,
        bytes: as_blob(&r[11]),
        original: as_blob(&r[12]),
        mask: opt_blob(&r[13]),
        mask_w: as_int(&r[14]).max(0) as u32,
        mask_h: as_int(&r[15]).max(0) as u32,
        mask_enabled: as_bool(&r[16]),
        is_group: as_bool(&r[17]),
        group_id: as_opt_text(&r[18]),
    }
}

/// Build the compositor node tree. `rows` must outlive the returned nodes.
fn build_nodes<'a>(rows: &'a [LayerRow]) -> Vec<Node<'a>> {
    build_children(rows, None, 0)
}

fn build_children<'a>(rows: &'a [LayerRow], parent: Option<&str>, depth: u32) -> Vec<Node<'a>> {
    if depth > 32 {
        return Vec::new();
    }
    let mut nodes = Vec::new();
    for row in rows {
        let row_parent = row.group_id.as_deref().filter(|s| !s.is_empty());
        if row_parent != parent {
            continue;
        }
        if row.is_group {
            nodes.push(Node::Group(Group {
                opacity: row.opacity,
                blend: row.blend,
                visible: row.visible,
                children: build_children(rows, Some(row.id.as_str()), depth + 1),
            }));
        } else {
            nodes.push(Node::Layer(row.to_raster()));
        }
    }
    nodes
}

/// Composite a document's stack to `w` × `h` straight RGBA.
pub fn compose(rows: &[LayerRow], w: u32, h: u32) -> Vec<u8> {
    // Fast path: a lone, full-canvas, normal pixel layer is its own composite.
    if let [row] = rows {
        if !row.is_group
            && row.visible
            && row.opacity >= 1.0
            && row.blend == BlendMode::Normal
            && row.x == 0
            && row.y == 0
            && row.w == w
            && row.h == h
            && row.mask.is_none()
        {
            return row.bytes.clone();
        }
    }
    let nodes = build_nodes(rows);
    composite::composite(w, h, &nodes)
}

/// Recompose the document and write the flattened cache back to `images`.
/// Returns the composed RGBA buffer and its dimensions.
pub fn refresh_composite(
    db: &Db,
    uid: &str,
    image_id: &str,
) -> Result<(Vec<u8>, u32, u32), AppError> {
    let doc = load_doc(db, uid, image_id)?;
    let rows = load_layers(db, uid, image_id)?;
    if rows.is_empty() {
        return Ok((doc.bytes, doc.width, doc.height));
    }
    let (w, h) = (doc.width.max(1), doc.height.max(1));
    let raw = compose(&rows, w, h);
    db.execute(
        "UPDATE images SET bytes = ?1, width = ?2, height = ?3, format = 'rgba', \
         updated_at = datetime('now') WHERE id = ?4 AND user_id = ?5",
        &[
            Value::blob(raw.clone()),
            Value::Int(w as i64),
            Value::Int(h as i64),
            Value::text(image_id),
            Value::text(uid),
        ],
    )?;
    Ok((raw, w, h))
}

/// The composited RGBA without writing the cache.
pub fn rendered(db: &Db, uid: &str, image_id: &str) -> Result<(Vec<u8>, u32, u32), AppError> {
    let doc = load_doc(db, uid, image_id)?;
    let rows = load_layers(db, uid, image_id)?;
    let (w, h) = (doc.width.max(1), doc.height.max(1));
    if rows.is_empty() {
        return Ok((doc.bytes, w, h));
    }
    Ok((compose(&rows, w, h), w, h))
}

/* ── Writes ─────────────────────────────────────────────────────── */

/// Ensure a document has at least one pixel layer (the migrated base).
/// Returns the current stack.
pub fn ensure_base_layer(db: &Db, uid: &str, image_id: &str) -> Result<Vec<LayerRow>, AppError> {
    let mut rows = load_layers(db, uid, image_id)?;
    if !rows.is_empty() {
        return Ok(rows);
    }
    let doc = load_doc(db, uid, image_id)?;
    let w = doc.width.max(1);
    let h = doc.height.max(1);
    let bytes = if doc.format == "rgba" {
        doc.bytes
    } else {
        crate::ops::decode(&doc.bytes)?.get_raw_pixels()
    };
    let original = if doc.format == "rgba" && doc.original.len() == bytes.len() {
        doc.original
    } else {
        bytes.clone()
    };
    let id = uuid::Uuid::new_v4().to_string();
    db.execute(
        "INSERT INTO image_layers (id, image_id, user_id, name, position, visible, opacity, \
         blend_mode, x, y, width, height, bytes, original, mask_enabled, is_group) \
         VALUES (?1, ?2, ?3, 'Background', 0, 1, 1.0, 'normal', 0, 0, ?4, ?5, ?6, ?7, 1, 0)",
        &[
            Value::text(&id),
            Value::text(image_id),
            Value::text(uid),
            Value::Int(w as i64),
            Value::Int(h as i64),
            Value::blob(bytes),
            Value::blob(original),
        ],
    )?;
    rows = load_layers(db, uid, image_id)?;
    Ok(rows)
}

/// Fetch one layer by id (ownership-scoped).
pub fn get_layer(
    db: &Db,
    uid: &str,
    image_id: &str,
    layer_id: &str,
) -> Result<LayerRow, AppError> {
    load_layers(db, uid, image_id)?
        .into_iter()
        .find(|l| l.id == layer_id)
        .ok_or_else(|| AppError::NotFound("Layer not found".into()))
}

/// The active pixel layer: an explicit id, else the topmost non-group layer.
pub fn active_layer(
    db: &Db,
    uid: &str,
    image_id: &str,
    wanted: Option<&str>,
) -> Result<LayerRow, AppError> {
    let rows = ensure_base_layer(db, uid, image_id)?;
    if let Some(id) = wanted.filter(|s| !s.trim().is_empty()) {
        if let Some(l) = rows.iter().find(|l| l.id == id) {
            if !l.is_group {
                return Ok(l.clone());
            }
            // Folders can't be painted on; fall through to the topmost pixel
            // layer so an edit never targets an empty folder.
        } else {
            return Err(AppError::NotFound("Layer not found".into()));
        }
    }
    rows.into_iter()
        .filter(|l| !l.is_group)
        .next_back()
        .ok_or_else(|| AppError::NotFound("Image has no pixel layer".into()))
}

/// Insert a new pixel layer into a document. `group_id` may place it in a
/// folder; positions are appended within that parent.
#[allow(clippy::too_many_arguments)]
pub fn insert_layer(
    db: &Db,
    uid: &str,
    image_id: &str,
    name: &str,
    group_id: Option<&str>,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    bytes: Vec<u8>,
) -> Result<String, AppError> {
    let position = next_position(db, uid, image_id, group_id)?;
    let id = uuid::Uuid::new_v4().to_string();
    db.execute(
        "INSERT INTO image_layers (id, image_id, user_id, name, position, visible, opacity, \
         blend_mode, x, y, width, height, bytes, original, mask_enabled, is_group, group_id) \
         VALUES (?1, ?2, ?3, ?4, ?5, 1, 1.0, 'normal', ?6, ?7, ?8, ?9, ?10, ?10, 1, 0, ?11)",
        &[
            Value::text(&id),
            Value::text(image_id),
            Value::text(uid),
            Value::text(name),
            Value::Int(position),
            Value::Int(x as i64),
            Value::Int(y as i64),
            Value::Int(w as i64),
            Value::Int(h as i64),
            Value::blob(bytes),
            match group_id {
                Some(g) => Value::text(g),
                None => Value::Null,
            },
        ],
    )?;
    Ok(id)
}

/// Insert an empty folder (group) into a document.
pub fn create_group(
    db: &Db,
    uid: &str,
    image_id: &str,
    name: &str,
    group_id: Option<&str>,
) -> Result<String, AppError> {
    let position = next_position(db, uid, image_id, group_id)?;
    let id = uuid::Uuid::new_v4().to_string();
    db.execute(
        "INSERT INTO image_layers (id, image_id, user_id, name, position, visible, opacity, \
         blend_mode, x, y, width, height, bytes, original, mask_enabled, is_group, group_id) \
         VALUES (?1, ?2, ?3, ?4, ?5, 1, 1.0, 'normal', 0, 0, 0, 0, x'', x'', 1, 1, ?6)",
        &[
            Value::text(&id),
            Value::text(image_id),
            Value::text(uid),
            Value::text(name),
            Value::Int(position),
            match group_id {
                Some(g) => Value::text(g),
                None => Value::Null,
            },
        ],
    )?;
    Ok(id)
}

fn next_position(
    db: &Db,
    uid: &str,
    image_id: &str,
    group_id: Option<&str>,
) -> Result<i64, AppError> {
    let rows = db.query(
        "SELECT COALESCE(MAX(position), -1) FROM image_layers \
         WHERE image_id = ?1 AND user_id = ?2 AND group_id IS ?3",
        &[
            Value::text(image_id),
            Value::text(uid),
            match group_id {
                Some(g) => Value::text(g),
                None => Value::Null,
            },
        ],
    )?;
    Ok(rows.first().map(|r| as_int(&r[0])).unwrap_or(-1) + 1)
}

/// Fields a `PUT /layers/:id` may change.
#[derive(Default)]
pub struct LayerPatch {
    pub name: Option<String>,
    pub visible: Option<bool>,
    pub opacity: Option<f32>,
    pub blend_mode: Option<String>,
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub group_id: Option<Option<String>>,
}

pub fn update_layer(
    db: &Db,
    uid: &str,
    image_id: &str,
    layer_id: &str,
    patch: &LayerPatch,
) -> Result<(), AppError> {
    let mut sets: Vec<String> = Vec::new();
    let mut params: Vec<Value> = Vec::new();
    if let Some(n) = &patch.name {
        params.push(Value::text(n.chars().take(120).collect::<String>()));
        sets.push(format!("name = ?{}", params.len()));
    }
    if let Some(v) = patch.visible {
        params.push(Value::Int(v as i64));
        sets.push(format!("visible = ?{}", params.len()));
    }
    if let Some(o) = patch.opacity {
        // `Value` has no Real variant, so bind the decimal as text and let
        // SQLite cast it — avoids storing opacity as a truncated integer.
        params.push(Value::text(format!("{}", o.clamp(0.0, 1.0))));
        sets.push(format!("opacity = CAST(?{} AS REAL)", params.len()));
    }
    if let Some(b) = &patch.blend_mode {
        params.push(Value::text(BlendMode::parse(b).as_str()));
        sets.push(format!("blend_mode = ?{}", params.len()));
    }
    if let Some(x) = patch.x {
        params.push(Value::Int(x as i64));
        sets.push(format!("x = ?{}", params.len()));
    }
    if let Some(y) = patch.y {
        params.push(Value::Int(y as i64));
        sets.push(format!("y = ?{}", params.len()));
    }
    if let Some(g) = &patch.group_id {
        params.push(match g {
            Some(id) => Value::text(id),
            None => Value::Null,
        });
        sets.push(format!("group_id = ?{}", params.len()));
    }
    if sets.is_empty() {
        return Ok(());
    }
    let sql = format!(
        "UPDATE image_layers SET {} , updated_at = datetime('now') \
         WHERE id = ?{} AND image_id = ?{} AND user_id = ?{}",
        sets.join(", "),
        params.len() + 1,
        params.len() + 2,
        params.len() + 3,
    );
    params.push(Value::text(layer_id));
    params.push(Value::text(image_id));
    params.push(Value::text(uid));
    let changed = db.execute(&sql, &params)?;
    if changed == 0 {
        return Err(AppError::NotFound("Layer not found".into()));
    }
    Ok(())
}

pub fn delete_layer(db: &Db, uid: &str, image_id: &str, layer_id: &str) -> Result<(), AppError> {
    // Refuse to orphan a folder's children: reparent them to the folder's
    // parent instead, keeping the stack usable.
    let rows = load_layers(db, uid, image_id)?;
    let target = rows
        .iter()
        .find(|l| l.id == layer_id)
        .ok_or_else(|| AppError::NotFound("Layer not found".into()))?;
    if target.is_group {
        db.execute(
            "UPDATE image_layers SET group_id = ?1 WHERE image_id = ?2 AND user_id = ?3 AND group_id = ?4",
            &[
                match &target.group_id {
                    Some(g) => Value::text(g),
                    None => Value::Null,
                },
                Value::text(image_id),
                Value::text(uid),
                Value::text(layer_id),
            ],
        )?;
    }
    let changed = db.execute(
        "DELETE FROM image_layers WHERE id = ?1 AND image_id = ?2 AND user_id = ?3",
        &[Value::text(layer_id), Value::text(image_id), Value::text(uid)],
    )?;
    if changed == 0 {
        return Err(AppError::NotFound("Layer not found".into()));
    }
    Ok(())
}

/// Assign positions 0..n for the given ids (bottom-to-top).
pub fn set_positions(db: &Db, uid: &str, image_id: &str, ids: &[String]) -> Result<(), AppError> {
    for (i, id) in ids.iter().enumerate() {
        db.execute(
            "UPDATE image_layers SET position = ?1, updated_at = datetime('now') \
             WHERE id = ?2 AND image_id = ?3 AND user_id = ?4",
            &[
                Value::Int(i as i64),
                Value::text(id),
                Value::text(image_id),
                Value::text(uid),
            ],
        )?;
    }
    Ok(())
}

pub fn duplicate_layer(
    db: &Db,
    uid: &str,
    image_id: &str,
    layer_id: &str,
) -> Result<String, AppError> {
    let rows = load_layers(db, uid, image_id)?;
    let src = rows
        .iter()
        .find(|l| l.id == layer_id)
        .ok_or_else(|| AppError::NotFound("Layer not found".into()))?;
    let position = next_position(db, uid, image_id, src.group_id.as_deref())?;
    let id = uuid::Uuid::new_v4().to_string();
    db.execute(
        "INSERT INTO image_layers (id, image_id, user_id, name, position, visible, opacity, \
         blend_mode, x, y, width, height, bytes, original, mask, mask_width, mask_height, \
         mask_enabled, is_group, group_id) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
        &[
            Value::text(&id),
            Value::text(image_id),
            Value::text(uid),
            Value::text(format!("{} copy", src.name)),
            Value::Int(position),
            Value::Int(src.visible as i64),
            Value::text(format!("{}", src.opacity)),
            Value::text(src.blend.as_str()),
            Value::Int(src.x as i64),
            Value::Int(src.y as i64),
            Value::Int(src.w as i64),
            Value::Int(src.h as i64),
            Value::blob(src.bytes.clone()),
            Value::blob(src.original.clone()),
            match &src.mask {
                Some(m) => Value::blob(m.clone()),
                None => Value::Null,
            },
            Value::Int(src.mask_w as i64),
            Value::Int(src.mask_h as i64),
            Value::Int(src.mask_enabled as i64),
            Value::Int(src.is_group as i64),
            match &src.group_id {
                Some(g) => Value::text(g),
                None => Value::Null,
            },
        ],
    )?;
    Ok(id)
}

/// Merge a layer into the sibling directly beneath it (same parent).
pub fn merge_down(db: &Db, uid: &str, image_id: &str, layer_id: &str) -> Result<(), AppError> {
    let doc = load_doc(db, uid, image_id)?;
    let rows = load_layers(db, uid, image_id)?;
    let upper = rows
        .iter()
        .find(|l| l.id == layer_id && !l.is_group)
        .ok_or_else(|| AppError::NotFound("Layer not found or is a folder".into()))?;
    let siblings: Vec<&LayerRow> = rows
        .iter()
        .filter(|l| l.group_id == upper.group_id && !l.is_group)
        .collect();
    let Some(pos) = siblings.iter().position(|l| l.id == layer_id) else {
        return Err(AppError::NotFound("Layer not found".into()));
    };
    if pos == 0 {
        return Err(AppError::BadRequest(
            "no layer beneath to merge into".into(),
        ));
    }
    let lower = siblings[pos - 1];

    let (w, h) = (doc.width.max(1), doc.height.max(1));
    // Bake at full opacity: the lower layer keeps its own opacity/blend as a
    // live property when the merged result is composited into the stack.
    let mut base = lower.to_raster();
    base.opacity = 1.0;
    base.blend = BlendMode::Normal;
    base.mask = None;
    let mut top = upper.to_raster();
    top.opacity = upper.opacity.clamp(0.0, 1.0);
    let merged = composite::composite(w, h, &[Node::Layer(base), Node::Layer(top)]);

    db.execute(
        "UPDATE image_layers SET bytes = ?1, original = ?1, x = 0, y = 0, width = ?2, height = ?3, \
         mask = NULL, mask_width = 0, mask_height = 0, updated_at = datetime('now') \
         WHERE id = ?4 AND image_id = ?5 AND user_id = ?6",
        &[
            Value::blob(merged),
            Value::Int(w as i64),
            Value::Int(h as i64),
            Value::text(&lower.id),
            Value::text(image_id),
            Value::text(uid),
        ],
    )?;
    db.execute(
        "DELETE FROM image_layers WHERE id = ?1 AND image_id = ?2 AND user_id = ?3",
        &[
            Value::text(layer_id),
            Value::text(image_id),
            Value::text(uid),
        ],
    )?;
    Ok(())
}

/// Flatten every layer and folder into one full-canvas pixel layer.
pub fn flatten(db: &Db, uid: &str, image_id: &str) -> Result<(), AppError> {
    let doc = load_doc(db, uid, image_id)?;
    let rows = load_layers(db, uid, image_id)?;
    if rows.is_empty() {
        return Ok(());
    }
    let (w, h) = (doc.width.max(1), doc.height.max(1));
    let raw = compose(&rows, w, h);
    db.execute(
        "DELETE FROM image_layers WHERE image_id = ?1 AND user_id = ?2",
        &[Value::text(image_id), Value::text(uid)],
    )?;
    let id = uuid::Uuid::new_v4().to_string();
    db.execute(
        "INSERT INTO image_layers (id, image_id, user_id, name, position, visible, opacity, \
         blend_mode, x, y, width, height, bytes, original, mask_enabled, is_group) \
         VALUES (?1, ?2, ?3, 'Flattened', 0, 1, 1.0, 'normal', 0, 0, ?4, ?5, ?6, ?6, 1, 0)",
        &[
            Value::text(&id),
            Value::text(image_id),
            Value::text(uid),
            Value::Int(w as i64),
            Value::Int(h as i64),
            Value::blob(raw),
        ],
    )?;
    Ok(())
}

/// Replace a pixel layer's own pixels (used by upload/replace).
pub fn set_layer_pixels(
    db: &Db,
    uid: &str,
    image_id: &str,
    layer_id: &str,
    bytes: Vec<u8>,
    original: Vec<u8>,
    w: u32,
    h: u32,
    x: i32,
    y: i32,
) -> Result<(), AppError> {
    let changed = db.execute(
        "UPDATE image_layers SET bytes = ?1, original = ?2, width = ?3, height = ?4, \
         x = ?5, y = ?6, updated_at = datetime('now') \
         WHERE id = ?7 AND image_id = ?8 AND user_id = ?9",
        &[
            Value::blob(bytes),
            Value::blob(original),
            Value::Int(w as i64),
            Value::Int(h as i64),
            Value::Int(x as i64),
            Value::Int(y as i64),
            Value::text(layer_id),
            Value::text(image_id),
            Value::text(uid),
        ],
    )?;
    if changed == 0 {
        return Err(AppError::NotFound("Layer not found".into()));
    }
    Ok(())
}
