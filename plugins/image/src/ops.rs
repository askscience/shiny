//! Image operations — a shared engine that turns a JSON list of operations
//! into `photon-rs` calls. Used by both the agent tool (`image_edit`) and the
//! REST route (`POST /api/images/:id/apply`), so they always agree.

use photon_rs::transform::SamplingFilter;
use photon_rs::PhotonImage;
use serde_json::Value;

use shiny_plugin_sdk::errors::AppError;

/// Hard ceiling for any layer/document dimension the server will allocate.
/// Matches the document resize cap; keeps an RGBA buffer bounded at
/// 8192² × 4 = 256 MiB and stops absurd `width`/`height` values from reaching
/// `vec![0; …]`, where a failed allocation aborts the whole host process.
pub const MAX_LAYER_DIM: u32 = 8192;

/// Validate a layer/document size and return its RGBA byte length. All math is
/// checked, so no `u32` pair can overflow `usize` before the allocation.
pub fn checked_layer_len(w: u32, h: u32) -> Result<usize, AppError> {
    if w == 0 || h == 0 {
        return Err(AppError::BadRequest(
            "layer width and height must be positive".into(),
        ));
    }
    if w > MAX_LAYER_DIM || h > MAX_LAYER_DIM {
        return Err(AppError::BadRequest(format!(
            "layer dimensions may not exceed {MAX_LAYER_DIM}×{MAX_LAYER_DIM}"
        )));
    }
    (w as usize)
        .checked_mul(h as usize)
        .and_then(|px| px.checked_mul(4))
        .ok_or_else(|| AppError::BadRequest("layer dimensions overflow".into()))
}

/// photon-rs convolutions sample a 3×3 neighbourhood; on 1–2 px images they
/// index past the raw buffer and panic (`conv.rs`). Skip them instead.
pub fn convolution_safe(img: &PhotonImage) -> bool {
    img.get_width() >= 3 && img.get_height() >= 3
}

/// Read only the declared dimensions from the header, before the decoder
/// allocates a full pixel buffer. A 32 MiB PNG can still declare a
/// 100000×100000 canvas; decoding first would abort on allocation failure.
fn declared_dimensions(bytes: &[u8]) -> Result<(u32, u32), AppError> {
    use std::io::Cursor;
    let reader = image::io::Reader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| AppError::BadRequest(format!("couldn't read that image ({e})")))?;
    reader
        .into_dimensions()
        .map_err(|e| AppError::BadRequest(format!("couldn't read that image ({e})")))
}

/// Decode image bytes (PNG/JPEG/GIF/WebP/BMP/…) into a `PhotonImage`.
pub fn decode(bytes: &[u8]) -> Result<PhotonImage, AppError> {
    let (w, h) = declared_dimensions(bytes)?;
    if w == 0 || h == 0 {
        return Err(AppError::BadRequest("empty image".into()));
    }
    if w > MAX_LAYER_DIM || h > MAX_LAYER_DIM {
        return Err(AppError::BadRequest(format!(
            "image is too large (max {MAX_LAYER_DIM} px per side)"
        )));
    }
    photon_rs::native::open_image_from_bytes(bytes).map_err(|e| {
        AppError::BadRequest(format!(
            "couldn't read that image ({e}) — PNG, JPEG, GIF, WebP and BMP are supported"
        ))
    })
}

/// Resize so the longest side is at most `max_dim` (only ever shrinks).
pub fn fit(img: &mut PhotonImage, max_dim: u32) {
    let (w, h) = (img.get_width(), img.get_height());
    let longest = w.max(h);
    if longest > max_dim {
        let scale = max_dim as f64 / longest as f64;
        let nw = ((w as f64) * scale).round().max(1.0) as u32;
        let nh = ((h as f64) * scale).round().max(1.0) as u32;
        *img = photon_rs::transform::resize(img, nw, nh, SamplingFilter::Lanczos3);
    }
}

fn f64_param(op: &Value, key: &str, default: f64) -> f64 {
    op.get(key).and_then(|v| v.as_f64()).unwrap_or(default)
}

fn i64_param(op: &Value, key: &str, default: i64) -> i64 {
    op.get(key).and_then(|v| v.as_i64()).unwrap_or(default)
}

fn is_reset(op: &Value) -> bool {
    op.get("op")
        .and_then(|v| v.as_str())
        .map(|s| s.eq_ignore_ascii_case("reset"))
        .unwrap_or(false)
}

/// Apply one operation in-place.
fn apply_one(img: &mut PhotonImage, op: &Value, idx: usize) -> Result<(), AppError> {
    let name = op.get("op").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
    let err = |m: String| AppError::BadRequest(format!("operation {idx} (\"{name}\"): {m}"));

    // Adjustment and filter engines first; each returns Ok(false) when it does
    // not own the name so the next engine can try.
    if crate::adjust::apply(img, &name, op).map_err(&err)? {
        return Ok(());
    }
    if crate::filter::apply(img, &name, op).map_err(&err)? {
        return Ok(());
    }

    match name.as_str() {
        "grayscale" | "greyscale" => photon_rs::monochrome::grayscale(img),
        "sepia" => photon_rs::monochrome::sepia(img),
        "invert" => photon_rs::channels::invert(img),
        "brightness" => {
            let a = i64_param(op, "amount", 0).clamp(-255, 255) as i16;
            photon_rs::effects::adjust_brightness(img, a);
        }
        "contrast" => {
            let a = f64_param(op, "amount", 0.0).clamp(-255.0, 255.0) as f32;
            photon_rs::effects::adjust_contrast(img, a);
        }
        "rotate" => {
            // rem_euclid keeps any angle (negative, >360°, huge) in a sane
            // f32 range so precision doesn't fall apart.
            let angle = f64_param(op, "angle", 0.0).rem_euclid(360.0) as f32;
            *img = photon_rs::transform::rotate(img, angle);
        }
        "flip_h" | "fliph" => photon_rs::transform::fliph(img),
        "flip_v" | "flipv" => photon_rs::transform::flipv(img),
        "look" => {
            let n = op.get("name").and_then(|v| v.as_str()).unwrap_or("");
            if n.trim().is_empty() {
                return Err(err("name required — pass {\"op\":\"look\",\"name\":\"portra\"}".into()));
            }
            crate::looks::apply(img, n).map_err(&err)?;
        }
        "reset" => { /* handled by apply_raw */ }
        other => return Err(err(format!("unknown operation \"{other}\""))),
    }
    Ok(())
}

/// Encode raw RGBA pixels to a PNG byte vector (used only when serving or
/// downloading — never on the edit hot path).
pub fn encode_png(raw: &[u8], w: u32, h: u32) -> Vec<u8> {
    PhotonImage::new(raw.to_vec(), w, h).get_bytes()
}

/// Apply operations to raw RGBA pixels in memory (no codec round-trip).
/// `reset` swaps in the original pixels and original dimensions; every other
/// operation mutates the image in order. Returns the new pixels, width and
/// height.
pub fn apply_raw(
    current_raw: &[u8],
    current_w: u32,
    current_h: u32,
    original_raw: &[u8],
    original_w: u32,
    original_h: u32,
    ops: &[Value],
) -> Result<(Vec<u8>, u32, u32), AppError> {
    let wants_reset = ops.iter().any(is_reset);
    let (base, bw, bh) = if wants_reset {
        (original_raw, original_w, original_h)
    } else {
        (current_raw, current_w, current_h)
    };

    let mut img = PhotonImage::new(base.to_vec(), bw, bh);
    for (i, op) in ops.iter().enumerate() {
        if is_reset(op) {
            continue;
        }
        apply_one(&mut img, op, i + 1)?;
    }
    Ok((img.get_raw_pixels(), img.get_width(), img.get_height()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_layer_len_bounds_dimensions() {
        // zero sides rejected
        assert!(checked_layer_len(0, 10).is_err());
        assert!(checked_layer_len(10, 0).is_err());
        // absurd sides rejected before any allocation could be attempted
        assert!(checked_layer_len(1_000_000_000, 1_000_000_000).is_err());
        assert!(checked_layer_len(MAX_LAYER_DIM + 1, 1).is_err());
        // the ceiling itself is allowed and the byte count is exact
        assert_eq!(
            checked_layer_len(MAX_LAYER_DIM, 1).unwrap(),
            MAX_LAYER_DIM as usize * 4
        );
        assert_eq!(checked_layer_len(4, 3).unwrap(), 48);
    }

    #[test]
    fn convolution_skips_tiny_images() {
        for (w, h, safe) in [(1u32, 1u32, false), (2, 2, false), (2, 8, false), (3, 3, true), (8, 1, false)] {
            let img = PhotonImage::new(vec![0u8; (w * h * 4) as usize], w, h);
            assert_eq!(convolution_safe(&img), safe, "{w}×{h}");
        }
    }

    #[test]
    fn blur_on_one_pixel_image_is_a_noop_not_a_panic() {
        let mut img = PhotonImage::new(vec![10, 20, 30, 255], 1, 1);
        apply_one(&mut img, &serde_json::json!({"op": "blur", "radius": 2}), 0)
            .expect("blur on a 1×1 image must not error or panic");
        assert_eq!(img.get_raw_pixels(), vec![10, 20, 30, 255]);
    }

    #[test]
    fn reset_restores_the_original_pixels() {
        let current = vec![9, 9, 9, 255];
        let original = vec![1, 2, 3, 255];
        let ops = vec![serde_json::json!({"op": "reset"})];
        let (out, w, h) = apply_raw(&current, 1, 1, &original, 1, 1, &ops).unwrap();
        assert_eq!((out, w, h), (original, 1, 1));
    }
}
