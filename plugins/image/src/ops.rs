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

/// Decode image bytes (PNG/JPEG/WebP/…) into a `PhotonImage`.
pub fn decode(bytes: &[u8]) -> Result<PhotonImage, AppError> {
    photon_rs::native::open_image_from_bytes(bytes)
        .map_err(|e| AppError::BadRequest(format!("could not decode image: {e}")))
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
            photon_rs::conv::gaussian_blur(img, r);
        }
        "sharpen" => photon_rs::conv::sharpen(img),
        "edge" | "edge_detection" => photon_rs::conv::edge_detection(img),
        "emboss" => photon_rs::conv::emboss(img),
        "sobel" => photon_rs::conv::sobel_global(img),
        "laplace" => photon_rs::conv::laplace(img),
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
            let angle = f64_param(op, "angle", 0.0) as f32;
            *img = photon_rs::transform::rotate(img, angle);
        }
        "resize" => {
            let w = i64_param(op, "width", 0);
            let h = i64_param(op, "height", 0);
            if w <= 0 || h <= 0 {
                return Err(err("resize needs positive width and height".into()));
            }
            *img = photon_rs::transform::resize(img, w as u32, h as u32, SamplingFilter::Lanczos3);
        }
        "crop" => {
            let x = i64_param(op, "x", 0).max(0) as u32;
            let y = i64_param(op, "y", 0).max(0) as u32;
            let w = i64_param(op, "width", 0);
            let h = i64_param(op, "height", 0);
            if w <= 0 || h <= 0 {
                return Err(err("crop needs positive width and height".into()));
            }
            let (x2, y2) = (x + w as u32, y + h as u32);
            if x2 > img.get_width() || y2 > img.get_height() || x >= x2 || y >= y2 {
                return Err(err("crop rectangle is outside the image bounds".into()));
            }
            *img = photon_rs::transform::crop(img, x, y, x2, y2);
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

/// Apply operations to raw RGBA pixels in memory (no codec round-trip).
/// `reset` swaps in the original pixels and original dimensions; every other
/// operation mutates the image in order. Returns (new raw pixels, w, h).
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
    let (base, w, h) = if wants_reset {
        (original_raw, original_w, original_h)
    } else {
        (current_raw, current_w, current_h)
    };

    let mut img = PhotonImage::new(base.to_vec(), w, h);
    for (i, op) in ops.iter().enumerate() {
        if is_reset(op) {
            continue;
        }
        apply_one(&mut img, op, i + 1)?;
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
/// (Fritsch–Carlson)** interpolation — smooth like Photoshop's Curves but with
/// no overshoot, so the mapping stays within 0..=255.
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
}
