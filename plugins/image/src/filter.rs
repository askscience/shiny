//! Convolution and render filters — blur, sharpen and vignette.
//!
//! Each filter mutates a `PhotonImage` in place and returns `Ok(false)` for
//! names it does not own, so the dispatcher can try the other engines.

use photon_rs::PhotonImage;
use serde_json::Value;

use crate::pixels::{from_raw, lerp, raw, to_u8};

fn f64_param(op: &Value, key: &str, default: f64) -> f64 {
    op.get(key).and_then(|v| v.as_f64()).unwrap_or(default)
}

fn int_param(op: &Value, key: &str, default: i64) -> i64 {
    op.get(key).and_then(|v| v.as_i64()).unwrap_or(default)
}

fn gaussian_blur(img: &mut PhotonImage, radius: u32) {
    if crate::ops::convolution_safe(img) {
        photon_rs::conv::gaussian_blur(img, radius as i32);
    }
}

fn unsharp_mask(img: &mut PhotonImage, radius: u32, amount: f64, threshold: f64) {
    let w = img.get_width();
    let h = img.get_height();
    if !crate::ops::convolution_safe(img) {
        return;
    }
    let mut blurred = img.clone();
    photon_rs::conv::gaussian_blur(&mut blurred, radius.max(1) as i32);
    let base = raw(img);
    let blur = raw(&blurred);
    let mut out = base.clone();
    let mut i = 0;
    while i + 3 < out.len() {
        for c in 0..3 {
            let diff = base[i + c] as f64 - blur[i + c] as f64;
            if diff.abs() >= threshold.max(0.0) {
                out[i + c] = to_u8(base[i + c] as f64 + diff * amount);
            }
        }
        i += 4;
    }
    *img = from_raw(out, w, h);
}

fn vignette(img: &mut PhotonImage, amount: f64, size: f64, roundness: f64) {
    let w = img.get_width();
    let h = img.get_height();
    let (cx, cy) = (w as f64 / 2.0, h as f64 / 2.0);
    let strength = amount / 100.0;
    let src = raw(img);
    let mut out = src.clone();
    for y in 0..h {
        for x in 0..w {
            let dx = (x as f64 - cx) / cx.max(1.0);
            let dy = (y as f64 - cy) / cy.max(1.0);
            let d = (dx * dx + dy * dy).sqrt();
            let t = ((d - size / 100.0) / (1.0 - size / 100.0).max(1e-3)).clamp(0.0, 1.0);
            let f = 1.0 - strength * t.powf(lerp(4.0, 1.0, roundness / 100.0));
            let i = ((y as usize) * (w as usize) + x as usize) * 4;
            for c in 0..3 {
                out[i + c] = to_u8(src[i + c] as f64 * f.clamp(0.0, 1.0));
            }
        }
    }
    *img = from_raw(out, w, h);
}

/// Dispatch one filter. Returns `Ok(true)` when handled.
pub fn apply(img: &mut PhotonImage, name: &str, op: &Value) -> Result<bool, String> {
    let radius = |op: &Value, d: u32| -> u32 {
        int_param(op, "radius", d as i64).clamp(0, 100) as u32
    };
    match name {
        "blur" | "gaussian_blur" | "gaussian" => gaussian_blur(img, radius(op, 2).max(1)),
        "sharpen" | "unsharp_mask" => unsharp_mask(
            img,
            radius(op, 1).max(1),
            f64_param(op, "amount", 1.0),
            f64_param(op, "threshold", 0.0),
        ),
        "vignette" => vignette(
            img,
            f64_param(op, "amount", 50.0),
            f64_param(op, "size", 30.0),
            f64_param(op, "roundness", 0.0),
        ),
        _ => return Ok(false),
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn img(px: Vec<u8>, w: u32, h: u32) -> PhotonImage {
        PhotonImage::new(px, w, h)
    }

    #[test]
    fn unknown_filter_is_not_handled() {
        let mut i = img(vec![0, 0, 0, 255], 1, 1);
        assert_eq!(apply(&mut i, "definitely_not_a_filter", &json!({})).unwrap(), false);
    }

    #[test]
    fn vignette_darkens_the_corner_but_not_the_centre() {
        let mut i = img(vec![255u8; 4 * 25], 5, 5);
        apply(&mut i, "vignette", &json!({"amount": 50})).unwrap();
        let p = i.get_raw_pixels();
        let centre = (2 * 5 + 2) * 4;
        let corner = 0;
        assert_eq!(p[centre], 255);
        assert!(p[corner] < 150, "corner should darken, got {}", p[corner]);
    }

    #[test]
    fn blur_on_a_uniform_patch_is_identity() {
        let px = vec![50u8; 4 * 16];
        let mut i = img(px.clone(), 4, 4);
        apply(&mut i, "blur", &json!({"radius": 2})).unwrap();
        assert_eq!(i.get_raw_pixels(), px);
    }
}
