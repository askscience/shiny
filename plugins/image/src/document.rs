//! Document-level geometry: crop, resize, rotate and flip the whole canvas,
//! reflowing every layer (and group) so the stack stays aligned.

use photon_rs::PhotonImage;
use photon_rs::transform::SamplingFilter;

use shiny_plugin_sdk::db::{Db, Value};
use shiny_plugin_sdk::errors::AppError;

use crate::layers::{self, LayerRow};

fn set_doc_size(
    db: &Db,
    uid: &str,
    image_id: &str,
    w: u32,
    h: u32,
) -> Result<(), AppError> {
    db.execute(
        "UPDATE images SET width = ?1, height = ?2, selection = NULL, \
         selection_width = 0, selection_height = 0, updated_at = datetime('now') \
         WHERE id = ?3 AND user_id = ?4",
        &[
            Value::Int(w.max(1) as i64),
            Value::Int(h.max(1) as i64),
            Value::text(image_id),
            Value::text(uid),
        ],
    )?;
    Ok(())
}

fn write_geom(
    db: &Db,
    uid: &str,
    image_id: &str,
    layer: &LayerRow,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    bytes: Option<Vec<u8>>,
) -> Result<(), AppError> {
    match bytes {
        Some(b) => db.execute(
            "UPDATE image_layers SET x = ?1, y = ?2, width = ?3, height = ?4, \
             bytes = ?5, original = ?5, updated_at = datetime('now') \
             WHERE id = ?6 AND image_id = ?7 AND user_id = ?8",
            &[
                Value::Int(x as i64),
                Value::Int(y as i64),
                Value::Int(w as i64),
                Value::Int(h as i64),
                Value::blob(b),
                Value::text(&layer.id),
                Value::text(image_id),
                Value::text(uid),
            ],
        )?,
        None => db.execute(
            "UPDATE image_layers SET x = ?1, y = ?2, updated_at = datetime('now') \
             WHERE id = ?3 AND image_id = ?4 AND user_id = ?5",
            &[
                Value::Int(x as i64),
                Value::Int(y as i64),
                Value::text(&layer.id),
                Value::text(image_id),
                Value::text(uid),
            ],
        )?,
    };
    Ok(())
}

/// Crop the document to `[x, y, x+w, y+h]`; layers keep their pixels and are
/// simply offset so the canvas clips them.
pub fn crop(db: &Db, uid: &str, image_id: &str, x: i32, y: i32, w: u32, h: u32) -> Result<(), AppError> {
    if w == 0 || h == 0 {
        return Err(AppError::BadRequest("crop needs positive width and height".into()));
    }
    let rows = layers::ensure_base_layer(db, uid, image_id)?;
    for layer in &rows {
        let nx = layer.x - x;
        let ny = layer.y - y;
        write_geom(db, uid, image_id, layer, nx, ny, layer.w, layer.h, None)?;
    }
    set_doc_size(db, uid, image_id, w, h)?;
    layers::refresh_composite(db, uid, image_id)?;
    Ok(())
}

/// Resize the document, scaling every pixel layer proportionally.
pub fn resize(db: &Db, uid: &str, image_id: &str, w: u32, h: u32) -> Result<(), AppError> {
    if w == 0 || h == 0 {
        return Err(AppError::BadRequest("resize needs positive width and height".into()));
    }
    const MAX_SIDE: u32 = 8192;
    let w = w.min(MAX_SIDE);
    let h = h.min(MAX_SIDE);
    let doc = layers::load_doc(db, uid, image_id)?;
    let dw = doc.width.max(1) as f64;
    let dh = doc.height.max(1) as f64;
    let sx = w as f64 / dw;
    let sy = h as f64 / dh;
    let rows = layers::ensure_base_layer(db, uid, image_id)?;
    for layer in &rows {
        if layer.is_group {
            continue;
        }
        let nw = ((layer.w as f64 * sx).round().max(1.0)) as u32;
        let nh = ((layer.h as f64 * sy).round().max(1.0)) as u32;
        let nx = (layer.x as f64 * sx).round() as i32;
        let ny = (layer.y as f64 * sy).round() as i32;
        let img = PhotonImage::new(layer.bytes.clone(), layer.w.max(1), layer.h.max(1));
        let resized = photon_rs::transform::resize(&img, nw, nh, SamplingFilter::Lanczos3);
        write_geom(db, uid, image_id, layer, nx, ny, nw, nh, Some(resized.get_raw_pixels()))?;
    }
    set_doc_size(db, uid, image_id, w, h)?;
    layers::refresh_composite(db, uid, image_id)?;
    Ok(())
}

/// Rotate the document by a multiple of 90°.
pub fn rotate(db: &Db, uid: &str, image_id: &str, angle: f64) -> Result<(), AppError> {
    let a = angle.rem_euclid(360.0).round() as i64;
    let a = if a == 0 { 0 } else if a % 90 != 0 { 90 } else { a };
    if a == 0 {
        return Ok(());
    }
    let doc = layers::load_doc(db, uid, image_id)?;
    let (dw, dh) = (doc.width.max(1), doc.height.max(1));
    let (ncw, nch) = if a % 180 == 0 { (dw, dh) } else { (dh, dw) };
    let (ncx, ncy) = (ncw as f64 / 2.0, nch as f64 / 2.0);
    let (cx, cy) = (dw as f64 / 2.0, dh as f64 / 2.0);
    let rows = layers::ensure_base_layer(db, uid, image_id)?;
    for layer in &rows {
        if layer.is_group {
            continue;
        }
        let img = PhotonImage::new(layer.bytes.clone(), layer.w.max(1), layer.h.max(1));
        let rotated = photon_rs::transform::rotate(&img, a as f32);
        let (nw, nh) = (rotated.get_width(), rotated.get_height());
        // Rotate the layer's origin about the canvas centre.
        let lx = layer.x as f64 + layer.w as f64 / 2.0 - cx;
        let ly = layer.y as f64 + layer.h as f64 / 2.0 - cy;
        let theta = (a as f64).to_radians();
        let (sin, cos) = theta.sin_cos();
        let rx = lx * cos - ly * sin;
        let ry = lx * sin + ly * cos;
        let nx = (ncx + rx - nw as f64 / 2.0).round() as i32;
        let ny = (ncy + ry - nh as f64 / 2.0).round() as i32;
        write_geom(db, uid, image_id, layer, nx, ny, nw, nh, Some(rotated.get_raw_pixels()))?;
    }
    set_doc_size(db, uid, image_id, ncw, nch)?;
    layers::refresh_composite(db, uid, image_id)?;
    Ok(())
}

/// Mirror the document horizontally or vertically.
pub fn flip(db: &Db, uid: &str, image_id: &str, axis: &str) -> Result<(), AppError> {
    let doc = layers::load_doc(db, uid, image_id)?;
    let (dw, dh) = (doc.width.max(1), doc.height.max(1));
    let horizontal = axis != "vertical";
    let rows = layers::ensure_base_layer(db, uid, image_id)?;
    for layer in &rows {
        if layer.is_group {
            continue;
        }
        let mut img = PhotonImage::new(layer.bytes.clone(), layer.w.max(1), layer.h.max(1));
        if horizontal {
            photon_rs::transform::fliph(&mut img);
        } else {
            photon_rs::transform::flipv(&mut img);
        }
        let nx = if horizontal { dw as i32 - (layer.x + layer.w as i32) } else { layer.x };
        let ny = if horizontal { layer.y } else { dh as i32 - (layer.y + layer.h as i32) };
        write_geom(db, uid, image_id, layer, nx, ny, layer.w, layer.h, Some(img.get_raw_pixels()))?;
    }
    set_doc_size(db, uid, image_id, dw, dh)?;
    layers::refresh_composite(db, uid, image_id)?;
    Ok(())
}
