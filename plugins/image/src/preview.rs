//! Reduced-size previews for dial drags.
//!
//! A dial step used to render the whole picture and send it back as raw RGBA
//! (up to 1600×1600, 10 MB, read, processed and transferred per step), while
//! the window usually shows it smaller than that.
//! While a dial is being dragged (`commit=false`) the window may ask for a
//! preview no larger than what it displays: the working pixels are scaled
//! down **once**, kept here, and each step renders the settings and the look
//! over that small copy. Releasing the dial commits and renders at full
//! size, exactly as before, so nothing that is stored or exported ever comes
//! from a preview.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{Map, Value as Json};

use shiny_plugin_sdk::db::{Db, Value};
use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::util::{as_int, as_opt_text, as_text};

use crate::doc::{Doc, EditResult};

/// Bounds of the longest side a window may ask for.
const MIN_SIDE: u32 = 256;
const MAX_SIDE: u32 = 2048;

/// A document's live state without its pixel buffers.
pub struct Meta {
    pub w: u32,
    pub h: u32,
    pub look: Option<String>,
    pub adjust: Map<String, Json>,
}

/// The size a preview of a `w`×`h` picture gets for a requested longest side,
/// or `None` when the picture is not worth reducing (already about that
/// small): the caller then renders at full size.
pub fn preview_size(w: u32, h: u32, max_side: u32) -> Option<(u32, u32)> {
    let max_side = max_side.clamp(MIN_SIDE, MAX_SIDE);
    let longest = w.max(h);
    // Under a quarter fewer pixels per side is not worth a second code path.
    if (longest as u64) * 4 <= (max_side as u64) * 5 {
        return None;
    }
    let scale = max_side as f64 / longest as f64;
    let pw = ((w as f64 * scale).round() as u32).max(1);
    let ph = ((h as f64 * scale).round() as u32).max(1);
    Some((pw, ph))
}

/// Read the live state of a document stored as raw RGBA. `None` for a legacy
/// `png` row (its dimensions are only known after decoding): the caller falls
/// back to the full-size path, which rewrites the row on its first edit.
pub fn meta(db: &Db, uid: &str, image_id: &str) -> Result<Option<Meta>, AppError> {
    let rows = db.query(
        "SELECT width, height, format, look, adjust FROM images WHERE id = ?1 AND user_id = ?2",
        &[Value::text(image_id), Value::text(uid)],
    )?;
    let r = rows
        .first()
        .ok_or_else(|| AppError::NotFound("Image not found".into()))?;
    if as_text(&r[2]) != "rgba" {
        return Ok(None);
    }
    Ok(Some(Meta {
        w: as_int(&r[0]).max(1) as u32,
        h: as_int(&r[1]).max(1) as u32,
        look: as_opt_text(&r[3]),
        adjust: serde_json::from_str::<Map<String, Json>>(&as_text(&r[4])).unwrap_or_default(),
    }))
}

/// The reduced working pixels of the picture being adjusted. One slot: a
/// user drags the dials of one picture at a time, and a slot holds at most
/// `MAX_SIDE`² pixels.
struct Slot {
    uid: String,
    image_id: String,
    src: (u32, u32),
    size: (u32, u32),
    raw: Arc<Vec<u8>>,
}

static SLOT: Mutex<Option<Slot>> = Mutex::new(None);
/// Bumped whenever working pixels are written, so a reduction computed from
/// pixels read before the write is not kept.
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Forget the kept reduction: the working pixels of some document changed.
/// Every write of `images.bytes` goes through `doc::save`, which calls this.
pub fn invalidate() {
    GENERATION.fetch_add(1, Ordering::SeqCst);
    if let Ok(mut slot) = SLOT.lock() {
        *slot = None;
    }
}

/// The current generation, read **before** loading pixels to reduce.
pub fn generation() -> u64 {
    GENERATION.load(Ordering::SeqCst)
}

/// The kept reduction for this document at this size, if any.
pub fn cached(uid: &str, image_id: &str, src: (u32, u32), size: (u32, u32)) -> Option<Arc<Vec<u8>>> {
    let slot = SLOT.lock().ok()?;
    let s = slot.as_ref()?;
    (s.uid == uid && s.image_id == image_id && s.src == src && s.size == size).then(|| s.raw.clone())
}

/// Scale full-size working pixels down and keep the result, unless pixels
/// were written since `generation` was read.
pub fn reduce(
    uid: &str,
    image_id: &str,
    raw: Vec<u8>,
    src: (u32, u32),
    size: (u32, u32),
    generation: u64,
) -> Result<Arc<Vec<u8>>, AppError> {
    crate::ops::checked_layer_len(src.0, src.1)?;
    if raw.len() < (src.0 as usize) * (src.1 as usize) * 4 {
        return Err(AppError::BadRequest("image data is incomplete".into()));
    }
    let reduced = Arc::new(box_downscale(&raw, src, size));
    if let Ok(mut slot) = SLOT.lock() {
        if GENERATION.load(Ordering::SeqCst) == generation {
            *slot = Some(Slot {
                uid: uid.to_string(),
                image_id: image_id.to_string(),
                src,
                size,
                raw: reduced.clone(),
            });
        }
    }
    Ok(reduced)
}

/// Scale RGBA pixels down by averaging the block of source pixels behind each
/// destination pixel. One pass over the source: a few milliseconds where a
/// filtered resize took a quarter of a second, which was the first step of
/// every drag.
fn box_downscale(raw: &[u8], src: (u32, u32), size: (u32, u32)) -> Vec<u8> {
    let (sw, sh) = (src.0 as usize, src.1 as usize);
    let (dw, dh) = (size.0 as usize, size.1 as usize);
    let mut out = vec![0u8; dw * dh * 4];
    // The source columns behind each destination column.
    let spans: Vec<(usize, usize)> = (0..dw)
        .map(|x| {
            let x0 = x * sw / dw;
            let x1 = ((x + 1) * sw / dw).max(x0 + 1).min(sw);
            (x0, x1)
        })
        .collect();
    for y in 0..dh {
        let y0 = y * sh / dh;
        let y1 = ((y + 1) * sh / dh).max(y0 + 1).min(sh);
        for (x, &(x0, x1)) in spans.iter().enumerate() {
            let mut sum = [0u32; 4];
            for sy in y0..y1 {
                let row = &raw[(sy * sw + x0) * 4..(sy * sw + x1) * 4];
                for px in row.chunks_exact(4) {
                    sum[0] += px[0] as u32;
                    sum[1] += px[1] as u32;
                    sum[2] += px[2] as u32;
                    sum[3] += px[3] as u32;
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as u32;
            let o = (y * dw + x) * 4;
            for c in 0..4 {
                out[o + c] = ((sum[c] + n / 2) / n) as u8;
            }
        }
    }
    out
}

/// Render a dial update over the reduced pixels. The returned settings are
/// the document's real ones; only the pixels are a preview.
pub fn render(
    reduced: &[u8],
    size: (u32, u32),
    src: (u32, u32),
    meta: Meta,
    update: &Map<String, Json>,
) -> Result<EditResult, AppError> {
    let d = Doc {
        title: String::new(),
        raw: reduced.to_vec(),
        w: size.0,
        h: size.1,
        original: Vec::new(),
        orig_w: size.0,
        orig_h: size.1,
        look: meta.look,
        adjust: meta.adjust,
    };
    let scale = size.0.max(size.1) as f64 / src.0.max(src.1).max(1) as f64;
    crate::doc::apply_settings_preview(d, update, scale)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_large_picture_is_reduced_to_the_requested_side_and_a_small_one_is_not() {
        assert_eq!(preview_size(4000, 3000, 1000), Some((1000, 750)));
        assert_eq!(preview_size(3000, 4000, 1000), Some((750, 1000)));
        // Already near the requested size: render it whole.
        assert_eq!(preview_size(1200, 900, 1000), None);
        assert_eq!(preview_size(800, 600, 1000), None);
        // The request is bounded on both ends.
        assert_eq!(preview_size(8000, 4000, 100_000), Some((2048, 1024)));
        assert_eq!(preview_size(8000, 10, 1), Some((256, 1)));
    }

    #[test]
    fn a_reduction_averages_the_pixels_behind_each_one() {
        // 4x2 → 2x1: each result pixel is the mean of a 2x2 block.
        let px = |v: u8| [v, v, v, 255];
        let raw: Vec<u8> = [px(0), px(100), px(10), px(30), px(100), px(200), px(50), px(70)].concat();
        assert_eq!(box_downscale(&raw, (4, 2), (2, 1)), [px(100), px(40)].concat());
        // Sizes that do not divide evenly still cover every destination pixel.
        let odd = box_downscale(&vec![80u8; 7 * 5 * 4], (7, 5), (3, 2));
        assert_eq!(odd, vec![80u8; 3 * 2 * 4]);
    }

    #[test]
    fn a_reduction_is_kept_until_pixels_are_written() {
        // Serialised with the other user of the one slot (the doc tests).
        let _guard = crate::doc::PREVIEW_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        invalidate();
        let src = (8, 4);
        let size = (4, 2);
        let raw = vec![200u8; 8 * 4 * 4];
        let before = generation();
        let reduced = reduce("u", "i", raw.clone(), src, size, before).unwrap();
        assert_eq!(reduced.len(), 4 * 2 * 4);
        assert!(cached("u", "i", src, size).is_some());
        assert!(cached("someone-else", "i", src, size).is_none());
        assert!(cached("u", "i", src, (2, 1)).is_none());

        invalidate();
        assert!(cached("u", "i", src, size).is_none());
        // Pixels read before a write are reduced but not kept.
        reduce("u", "i", raw, src, size, before).unwrap();
        assert!(cached("u", "i", src, size).is_none());
    }
}
