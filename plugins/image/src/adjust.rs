//! Tone and colour adjustments — hue/saturation, exposure, vibrance and the
//! one-click auto contrast / auto tone.
//!
//! Every function mutates a `PhotonImage` in place on straight RGBA channels and
//! returns `Ok(false)` when the operation name is not one of ours, so the
//! dispatcher can fall through to the filter engine.

use photon_rs::PhotonImage;
use serde_json::Value;

use crate::pixels::{clamp, hsl_to_rgb, map_pixels, rgb_to_hsl, to_u8};

fn f64_param(op: &Value, key: &str, default: f64) -> f64 {
    op.get(key).and_then(|v| v.as_f64()).unwrap_or(default)
}

fn bool_param(op: &Value, key: &str, default: bool) -> bool {
    op.get(key).and_then(|v| v.as_bool()).unwrap_or(default)
}

/* ── Hue / Saturation / Lightness ───────────────────────────────── */

fn apply_hue_saturation(img: &mut PhotonImage, op: &Value) {
    let hue = f64_param(op, "hue", 0.0);
    let sat = f64_param(op, "saturation", 0.0).clamp(-100.0, 100.0);
    let light = f64_param(op, "lightness", 0.0).clamp(-100.0, 100.0);
    let colorize = bool_param(op, "colorize", false);
    if hue == 0.0 && sat == 0.0 && light == 0.0 && !colorize {
        return;
    }
    map_pixels(img, |r, g, b, a| {
        let (h, mut s, mut l) = rgb_to_hsl(r as f64, g as f64, b as f64);
        let h = if colorize { hue } else { h + hue };
        if colorize {
            s = clamp(sat / 100.0, 0.0, 1.0);
            l = clamp(light / 100.0, 0.0, 1.0);
        } else {
            let sf = if sat >= 0.0 { 1.0 + sat / 50.0 } else { 1.0 + sat / 100.0 };
            s = clamp(s * sf, 0.0, 1.0);
            if light >= 0.0 {
                l = l + (1.0 - l) * (light / 100.0);
            } else {
                l *= 1.0 + light / 100.0;
            }
        }
        let (nr, ng, nb) = hsl_to_rgb(h, s, l);
        (to_u8(nr), to_u8(ng), to_u8(nb), a)
    });
}

/* ── Auto contrast / auto tone ──────────────────────────────────── */

fn histogram(px: &[u8]) -> [[u32; 256]; 3] {
    let mut hist = [[0u32; 256]; 3];
    let mut i = 0;
    while i + 3 < px.len() {
        hist[0][px[i] as usize] += 1;
        hist[1][px[i + 1] as usize] += 1;
        hist[2][px[i + 2] as usize] += 1;
        i += 4;
    }
    hist
}

/// Find the low/high percentile bounds for one channel histogram.
fn percentile_bounds(hist: &[u32; 256], clip: f64) -> (u8, u8) {
    let total: u32 = hist.iter().sum();
    if total == 0 {
        return (0, 255);
    }
    let limit = (total as f64 * clip / 100.0).round() as u32;
    let mut acc = 0u32;
    let mut lo = 0usize;
    for (i, &c) in hist.iter().enumerate() {
        acc += c;
        if acc > limit {
            lo = i;
            break;
        }
    }
    acc = 0;
    let mut hi = 255usize;
    for (i, &c) in hist.iter().enumerate().rev() {
        acc += c;
        if acc > limit {
            hi = i;
            break;
        }
    }
    (lo as u8, hi.max(lo + 1) as u8)
}

fn apply_auto_contrast(img: &mut PhotonImage, op: &Value) {
    let clip = f64_param(op, "clip", 0.5);
    let px = img.get_raw_pixels();
    let hist = histogram(&px);
    let mut bounds = [(0u8, 255u8); 3];
    for c in 0..3 {
        bounds[c] = percentile_bounds(&hist[c], clip);
    }
    map_pixels(img, |r, g, b, a| {
        let m = |v: u8, c: usize| -> u8 {
            let (lo, hi) = bounds[c];
            let span = (hi as f64 - lo as f64).max(1.0);
            to_u8(clamp((v as f64 - lo as f64) / span, 0.0, 1.0) * 255.0)
        };
        (m(r, 0), m(g, 1), m(b, 2), a)
    });
}

/// Auto tone = per-channel auto levels with a gentle gamma.
fn apply_auto_tone(img: &mut PhotonImage, op: &Value) {
    let px = img.get_raw_pixels();
    let hist = histogram(&px);
    let gamma = f64_param(op, "gamma", 1.0).clamp(0.1, 4.0);
    let mut bounds = [(0u8, 255u8); 3];
    for c in 0..3 {
        bounds[c] = percentile_bounds(&hist[c], 0.1);
    }
    map_pixels(img, |r, g, b, a| {
        let m = |v: u8, c: usize| -> u8 {
            let (lo, hi) = bounds[c];
            let span = (hi as f64 - lo as f64).max(1.0);
            let t = clamp((v as f64 - lo as f64) / span, 0.0, 1.0).powf(1.0 / gamma);
            to_u8(t * 255.0)
        };
        (m(r, 0), m(g, 1), m(b, 2), a)
    });
}

/* ── Exposure / vibrance ────────────────────────────────────────── */

fn apply_exposure(img: &mut PhotonImage, op: &Value) {
    let exposure = f64_param(op, "exposure", 0.0);
    let offset = f64_param(op, "offset", 0.0);
    let gamma = f64_param(op, "gamma", 1.0).clamp(0.01, 9.99);
    let factor = 2f64.powf(exposure);
    map_pixels(img, |r, g, b, a| {
        let m = |v: u8| -> u8 {
            let t = clamp((v as f64 / 255.0) * factor + offset / 255.0, 0.0, 1.0);
            to_u8(t.powf(1.0 / gamma) * 255.0)
        };
        (m(r), m(g), m(b), a)
    });
}

fn apply_vibrance(img: &mut PhotonImage, op: &Value) {
    let amount = f64_param(op, "amount", 0.0).clamp(-100.0, 100.0) / 100.0;
    if amount == 0.0 {
        return;
    }
    map_pixels(img, |r, g, b, a| {
        let (h, s, l) = rgb_to_hsl(r as f64, g as f64, b as f64);
        // Vibrance protects already-saturated pixels.
        let boost = amount * (1.0 - s);
        let s2 = clamp(s * (1.0 + boost), 0.0, 1.0);
        let (nr, ng, nb) = hsl_to_rgb(h, s2, l);
        (to_u8(nr), to_u8(ng), to_u8(nb), a)
    });
}

/// Dispatch one adjustment. Returns `Ok(true)` when handled.
pub fn apply(img: &mut PhotonImage, name: &str, op: &Value) -> Result<bool, String> {
    match name {
        "hue_saturation" | "hue" => apply_hue_saturation(img, op),
        "auto_contrast" => apply_auto_contrast(img, op),
        "auto_tone" => apply_auto_tone(img, op),
        "exposure" => apply_exposure(img, op),
        "vibrance" => apply_vibrance(img, op),
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
    fn hue_saturation_zero_is_a_noop() {
        let px = vec![120, 60, 200, 255];
        let mut i = img(px.clone(), 1, 1);
        apply(&mut i, "hue_saturation", &json!({"hue": 0, "saturation": 0, "lightness": 0})).unwrap();
        assert_eq!(i.get_raw_pixels(), px);
    }

    #[test]
    fn vibrance_zero_is_a_noop() {
        let px = vec![120, 60, 200, 255];
        let mut i = img(px.clone(), 1, 1);
        apply(&mut i, "vibrance", &json!({"amount": 0})).unwrap();
        assert_eq!(i.get_raw_pixels(), px);
    }
}
