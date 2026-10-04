//! Image operations — a shared engine that turns a JSON list of operations
//! into `photon-rs` calls. Used by both the agent tool (`image_edit`) and the
//! REST route (`POST /api/images/:id/apply`), so they always agree.

use photon_rs::transform::SamplingFilter;
use photon_rs::PhotonImage;
use serde_json::Value;

use shiny_plugin_sdk::errors::AppError;

/// Preset filter names accepted by `photon_rs::filters::filter`.
const FILTERS: &[&str] = &[
    "oceanic", "islands", "marine", "seagreen", "flagblue", "diamante", "liquid",
    "radio", "twenties", "rosetint", "mauve", "bluechrome", "vintage", "perfume",
    "serenity", "golden", "pastel_pink", "cali", "dramatic", "firenze", "obsidian", "lofi",
];

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

/// Apply one operation in-place. Selection masking happens in `apply_raw`.
fn apply_one(img: &mut PhotonImage, op: &Value, idx: usize) -> Result<(), AppError> {
    let name = op.get("op").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
    let err = |m: String| AppError::BadRequest(format!("operation {idx} (\"{name}\"): {m}"));

    // Adjustment → filter → paint engines first; each returns Ok(false) when it
    // does not own the name so the next engine can try.
    if crate::adjust::apply(img, &name, op).map_err(|m| err(m))? {
        return Ok(());
    }
    if crate::filter::apply(img, &name, op).map_err(|m| err(m))? {
        return Ok(());
    }
    if crate::paint::apply(img, &name, op).map_err(|m| err(m))? {
        return Ok(());
    }

    match name.as_str() {
        "grayscale" | "greyscale" => photon_rs::monochrome::grayscale(img),
        "sepia" => photon_rs::monochrome::sepia(img),
        "invert" => photon_rs::channels::invert(img),
        "solarize" => photon_rs::effects::solarize(img),
        "noise" => photon_rs::noise::add_noise_rand(img),
        "brightness" => {
            let a = i64_param(op, "amount", 0).clamp(-255, 255) as i16;
            photon_rs::effects::adjust_brightness(img, a);
        }
        "contrast" => {
            let a = f64_param(op, "amount", 0.0).clamp(-255.0, 255.0) as f32;
            photon_rs::effects::adjust_contrast(img, a);
        }
        "blur" => {
            let r = i64_param(op, "radius", 2).clamp(1, 50) as i32;
            if convolution_safe(img) {
                photon_rs::conv::gaussian_blur(img, r);
            }
        }
        "sharpen" => {
            if convolution_safe(img) {
                photon_rs::conv::sharpen(img);
            }
        }
        "edge" | "edge_detection" => {
            if convolution_safe(img) {
                photon_rs::conv::edge_detection(img);
            }
        }
        "emboss" => {
            if convolution_safe(img) {
                photon_rs::conv::emboss(img);
            }
        }
        "sobel" => {
            if convolution_safe(img) {
                photon_rs::conv::sobel_global(img);
            }
        }
        "laplace" => {
            if convolution_safe(img) {
                photon_rs::conv::laplace(img);
            }
        }
        "threshold" => {
            let t = i64_param(op, "amount", 128).clamp(0, 255) as u32;
            photon_rs::monochrome::threshold(img, t);
        }
        "tint" => {
            let r = i64_param(op, "r", 0).clamp(0, 255) as u32;
            let g = i64_param(op, "g", 0).clamp(0, 255) as u32;
            let b = i64_param(op, "b", 0).clamp(0, 255) as u32;
            photon_rs::effects::tint(img, r, g, b);
        }
        "rotate" => {
            // rem_euclid keeps any angle (negative, >360°, huge) in a sane
            // f32 range so precision doesn't fall apart.
            let angle = f64_param(op, "angle", 0.0).rem_euclid(360.0) as f32;
            *img = photon_rs::transform::rotate(img, angle);
        }
        "resize" | "image_size" => {
            let w = i64_param(op, "width", 0);
            let h = i64_param(op, "height", 0);
            if w <= 0 || h <= 0 {
                return Err(err("resize needs positive width and height".into()));
            }
            // Cap at a sane maximum — a giant `width` used to overflow into
            // absurd allocation sizes.
            const MAX_SIDE: i64 = 8192;
            let w = w.min(MAX_SIDE) as u32;
            let h = h.min(MAX_SIDE) as u32;
            *img = photon_rs::transform::resize(img, w, h, SamplingFilter::Lanczos3);
        }
        "crop" => {
            let x = i64_param(op, "x", 0);
            let y = i64_param(op, "y", 0);
            let w = i64_param(op, "width", 0);
            let h = i64_param(op, "height", 0);
            if w <= 0 || h <= 0 {
                return Err(err("crop needs positive width and height".into()));
            }
            // u64 math: `x + w` in u32 could overflow and wrap past the
            // bounds check below.
            if x < 0 || y < 0
                || x as u64 + w as u64 > img.get_width() as u64
                || y as u64 + h as u64 > img.get_height() as u64
            {
                return Err(err("crop rectangle is outside the image bounds".into()));
            }
            *img = photon_rs::transform::crop(
                img,
                x as u32,
                y as u32,
                (x + w) as u32,
                (y + h) as u32,
            );
        }
        "flip_h" | "fliph" => photon_rs::transform::fliph(img),
        "flip_v" | "flipv" => photon_rs::transform::flipv(img),
        "filter" => {
            let n = op.get("name").and_then(|v| v.as_str()).unwrap_or("lofi").to_lowercase();
            if !FILTERS.contains(&n.as_str()) {
                return Err(err(format!(
                    "unknown filter \"{n}\" — choose one of: {}",
                    FILTERS.join(", ")
                )));
            }
            photon_rs::filters::filter(img, &n);
        }
        "curves" => {
            let points = parse_curve_points(op, idx)?;
            let lut = build_curve_lut(&points);
            apply_lut(img, &lut);
        }
        "reset" => { /* handled by apply_raw */ }
        other => return Err(err(format!("unknown operation \"{other}\""))),
    }
    Ok(())
}

/// Operations that change the layer's rectangle (and therefore invalidate an
/// active selection for anything that follows in the same batch).
fn is_geometry(name: &str) -> bool {
    matches!(
        name,
        "resize" | "image_size" | "crop" | "crop_to_selection" | "rotate" | "flip_h"
            | "fliph" | "flip_v" | "flipv" | "transform" | "free_transform"
    )
}

/// Decode stored bytes to raw RGBA. Raw (`rgba`) rows are returned verbatim;
/// legacy encoded rows are decoded.
pub fn to_raw(bytes: &[u8], format: &str) -> Result<Vec<u8>, AppError> {
    if format == "rgba" {
        Ok(bytes.to_vec())
    } else {
        Ok(decode(bytes)?.get_raw_pixels())
    }
}

/// Encode raw RGBA pixels to a PNG byte vector (used only when serving or
/// downloading — never on the edit hot path).
pub fn encode_png(raw: &[u8], w: u32, h: u32) -> Vec<u8> {
    PhotonImage::new(raw.to_vec(), w, h).get_bytes()
}

/// Encode a small PNG preview of a layer (longest side `max_dim`), preserving
/// transparency. Used by the Layers panel thumbnails.
pub fn thumbnail_png(raw: &[u8], w: u32, h: u32, max_dim: u32) -> Vec<u8> {
    if w == 0 || h == 0 || raw.len() < (w as usize) * (h as usize) * 4 {
        return PhotonImage::new(vec![0, 0, 0, 0], 1, 1).get_bytes();
    }
    let mut img = PhotonImage::new(raw.to_vec(), w, h);
    let longest = w.max(h);
    if longest > max_dim {
        let scale = max_dim as f64 / longest as f64;
        let nw = ((w as f64) * scale).round().max(1.0) as u32;
        let nh = ((h as f64) * scale).round().max(1.0) as u32;
        img = photon_rs::transform::resize(&img, nw, nh, SamplingFilter::Lanczos3);
    }
    img.get_bytes()
}

/// Bounding box of the non-zero area of a coverage mask, if any.
fn mask_bbox(mask: &[u8], w: u32, h: u32) -> Option<(u32, u32, u32, u32)> {
    let (mut minx, mut miny, mut maxx, mut maxy) = (u32::MAX, u32::MAX, 0u32, 0u32);
    for y in 0..h {
        for x in 0..w {
            if mask[(y as usize) * (w as usize) + x as usize] > 0 {
                minx = minx.min(x);
                miny = miny.min(y);
                maxx = maxx.max(x);
                maxy = maxy.max(y);
            }
        }
    }
    if minx == u32::MAX {
        None
    } else {
        Some((minx, miny, maxx + 1, maxy + 1))
    }
}

/// Blend `over` into `base` using a per-pixel coverage mask (0..255), used to
/// confine any operation to the active selection.
fn blend_masked(base: &PhotonImage, over: &PhotonImage, mask: &[u8], w: u32, h: u32) -> PhotonImage {
    let b = base.get_raw_pixels();
    let o = over.get_raw_pixels();
    let mut out = b.clone();
    for i in 0..(w as usize) * (h as usize) {
        let a = mask.get(i).copied().unwrap_or(0) as f64 / 255.0;
        if a <= 0.0 {
            continue;
        }
        let p = i * 4;
        for c in 0..4 {
            out[p + c] = ((b[p + c] as f64) * (1.0 - a) + (o[p + c] as f64) * a).round() as u8;
        }
    }
    PhotonImage::new(out, w, h)
}

/// Apply operations to raw RGBA pixels in memory (no codec round-trip).
/// `reset` swaps in the original pixels and original dimensions; every other
/// operation mutates the image in order. `mask` is an optional per-pixel
/// coverage buffer the size of the *current* layer that confines every
/// non-geometric operation to the active selection. Returns the new pixels,
/// width and height.
#[allow(clippy::too_many_arguments)]
pub fn apply_raw(
    current_raw: &[u8],
    current_w: u32,
    current_h: u32,
    original_raw: &[u8],
    original_w: u32,
    original_h: u32,
    ops: &[Value],
    mask: Option<&[u8]>,
) -> Result<(Vec<u8>, u32, u32), AppError> {
    let wants_reset = ops.iter().any(is_reset);
    let (base, bw, bh) = if wants_reset {
        (original_raw, original_w, original_h)
    } else {
        (current_raw, current_w, current_h)
    };

    let mut img = PhotonImage::new(base.to_vec(), bw, bh);
    let mut mask = mask;
    for (i, op) in ops.iter().enumerate() {
        if is_reset(op) {
            continue;
        }
        let name = op.get("op").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();

        // Crop to the selection's bounding box.
        if name == "crop_to_selection" {
            let m = mask.ok_or_else(|| {
                AppError::BadRequest(format!(
                    "operation {} (\"crop_to_selection\"): an active selection is required",
                    i + 1
                ))
            })?;
            let (w, h) = (img.get_width(), img.get_height());
            let (x0, y0, x1, y1) = mask_bbox(m, w, h).ok_or_else(|| {
                AppError::BadRequest(format!(
                    "operation {} (\"crop_to_selection\"): the selection is empty",
                    i + 1
                ))
            })?;
            img = photon_rs::transform::crop(&img, x0, y0, x1, y1);
            mask = None;
            continue;
        }

        if is_geometry(&name) {
            apply_one(&mut img, op, i + 1)?;
            // Geometry changes the rectangle; a stale selection no longer maps.
            mask = None;
            continue;
        }

        match mask {
            Some(m) => {
                let (w, h) = (img.get_width(), img.get_height());
                let mut tmp = PhotonImage::new(img.get_raw_pixels(), w, h);
                apply_one(&mut tmp, op, i + 1)?;
                img = blend_masked(&img, &tmp, m, w, h);
            }
            None => apply_one(&mut img, op, i + 1)?,
        }
    }
    Ok((img.get_raw_pixels(), img.get_width(), img.get_height()))
}

// ---------- Curves (tone curve color correction) ----------------------------

fn clamp255(v: f64) -> f64 {
    v.max(0.0).min(255.0)
}

/// Clamp + round a value into the u8 range (rounding avoids the downward
/// bias a plain `as u8` truncation would introduce).
fn to_u8(v: f64) -> u8 {
    (v.max(0.0).min(255.0).round() as i64).clamp(0, 255) as u8
}

/// Parse a `curves` operation's control points from either `[[x,y], …]` or
/// `[{"x":…,"y":…}, …]`. Returns (x, y) pairs in 0..=255 space, sorted by x.
fn parse_curve_points(op: &Value, idx: usize) -> Result<Vec<(f64, f64)>, AppError> {
    let arr = op
        .get("points")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            AppError::BadRequest(format!(
                "operation {idx} (\"curves\"): points required — pass an array like [[0,0],[128,150],[255,255]]"
            ))
        })?;

    let mut pts = Vec::with_capacity(arr.len());
    for (i, item) in arr.iter().enumerate() {
        let (x, y) = if let Some(pair) = item.as_array() {
            if pair.len() < 2 {
                return Err(AppError::BadRequest(format!(
                    "operation {idx} (\"curves\"): point {i} needs [x,y]"
                )));
            }
            (pair[0].as_f64(), pair[1].as_f64())
        } else if let (Some(x), Some(y)) = (
            item.get("x").and_then(|v| v.as_f64()),
            item.get("y").and_then(|v| v.as_f64()),
        ) {
            (Some(x), Some(y))
        } else {
            return Err(AppError::BadRequest(format!(
                "operation {idx} (\"curves\"): point {i} must be [x,y] or {{\"x\":…,\"y\":…}}"
            )));
        };
        let x = x.ok_or_else(|| AppError::BadRequest(format!("operation {idx} (\"curves\"): point {i} x missing")))?;
        let y = y.ok_or_else(|| AppError::BadRequest(format!("operation {idx} (\"curves\"): point {i} y missing")))?;
        pts.push((clamp255(x), clamp255(y)));
    }
    if pts.len() < 2 {
        return Err(AppError::BadRequest(format!(
            "operation {idx} (\"curves\"): need at least 2 points"
        )));
    }
    pts.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    Ok(pts)
}

/// Build a 256-entry tone-curve LUT from control points using **monotone cubic
/// (Fritsch–Carlson)** interpolation — smooth like a photographic tone curve but
/// with no overshoot, so the mapping stays within 0..=255.
pub fn build_curve_lut(points: &[(f64, f64)]) -> [u8; 256] {
    let n = points.len();
    let mut lut = [0u8; 256];
    if n == 0 {
        for (i, v) in lut.iter_mut().enumerate() {
            *v = i as u8;
        }
        return lut;
    }
    if n == 1 {
        let y = to_u8(points[0].1);
        for v in lut.iter_mut() {
            *v = y;
        }
        return lut;
    }

    let xs: Vec<f64> = points.iter().map(|p| p.0).collect();
    let ys: Vec<f64> = points.iter().map(|p| p.1).collect();

    // Secant slopes.
    let mut d = vec![0.0f64; n - 1];
    for i in 0..n - 1 {
        let dx = xs[i + 1] - xs[i];
        d[i] = if dx.abs() < 1e-9 { 0.0 } else { (ys[i + 1] - ys[i]) / dx };
    }

    // Monotone tangents (Fritsch–Carlson).
    let mut m = vec![0.0f64; n];
    if n == 2 {
        m[0] = d[0];
        m[1] = d[0];
    } else {
        m[0] = d[0];
        m[n - 1] = d[n - 2];
        for i in 1..n - 1 {
            if d[i - 1] * d[i] <= 0.0 {
                m[i] = 0.0;
            } else {
                let h_prev = xs[i] - xs[i - 1];
                let h_next = xs[i + 1] - xs[i];
                let w1 = 2.0 * h_next + h_prev;
                let w2 = h_next + 2.0 * h_prev;
                m[i] = (w1 + w2) / (w1 / d[i - 1] + w2 / d[i]);
            }
        }
    }

    // Evaluate Hermite per segment.
    for seg in 0..n - 1 {
        let x0 = xs[seg];
        let x1 = xs[seg + 1];
        let y0 = ys[seg];
        let y1 = ys[seg + 1];
        let h = x1 - x0;
        if h.abs() < 1e-9 {
            continue;
        }
        let i0 = x0.round().clamp(0.0, 255.0) as usize;
        let i1 = (x1.round().clamp(0.0, 255.0) as usize).min(255);
        for x in i0..=i1 {
            let t = (x as f64 - x0) / h;
            let t2 = t * t;
            let t3 = t2 * t;
            let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
            let h10 = t3 - 2.0 * t2 + t;
            let h01 = -2.0 * t3 + 3.0 * t2;
            let h11 = t3 - t2;
            let y = h00 * y0 + h10 * h * m[seg] + h01 * y1 + h11 * h * m[seg + 1];
            lut[x] = to_u8(y);
        }
    }

    // Fill any range outside the first/last control points.
    let first = to_u8(ys[0]);
    let last = to_u8(ys[n - 1]);
    for (x, v) in lut.iter_mut().enumerate() {
        let xf = x as f64;
        if xf < xs[0] {
            *v = first;
        } else if xf > xs[n - 1] {
            *v = last;
        }
    }
    lut
}

/// Apply a tone-curve LUT to the image's RGB channels (alpha preserved).
fn apply_lut(img: &mut PhotonImage, lut: &[u8; 256]) {
    let w = img.get_width();
    let h = img.get_height();
    let mut px = img.get_raw_pixels();
    let mut i = 0;
    while i + 3 < px.len() {
        px[i] = lut[px[i] as usize];
        px[i + 1] = lut[px[i + 1] as usize];
        px[i + 2] = lut[px[i + 2] as usize];
        i += 4;
    }
    *img = PhotonImage::new(px, w, h);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curve_lut_identity() {
        let lut = build_curve_lut(&[(0.0, 0.0), (255.0, 255.0)]);
        assert_eq!(lut[0], 0);
        assert_eq!(lut[128], 128);
        assert_eq!(lut[255], 255);
    }

    #[test]
    fn curve_lut_lifts_midtones_monotonically() {
        let lut = build_curve_lut(&[(0.0, 0.0), (128.0, 160.0), (255.0, 255.0)]);
        assert_eq!(lut[0], 0);
        assert_eq!(lut[128], 160);
        assert_eq!(lut[255], 255);
        for i in 1..256 {
            assert!(lut[i] >= lut[i - 1], "curve must be monotonic at {i}");
        }
    }

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
}
