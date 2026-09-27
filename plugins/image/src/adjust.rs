//! Tone and colour adjustments — levels, curves companions, hue/saturation,
//! colour balance, exposure, vibrance, posterize, gradient map, channel mixer,
//! black & white and photo filter.
//!
//! Every function mutates a `PhotonImage` in place on straight RGBA channels and
//! returns `Ok(false)` when the operation name is not one of ours, so the
//! dispatcher can fall through to the convolution/filter engine.

use photon_rs::PhotonImage;
use serde_json::Value;

use crate::pixels::{clamp, hsl_to_rgb, lerp, luminance, map_pixels, rgb_to_hsl, to_u8};

fn f64_param(op: &Value, key: &str, default: f64) -> f64 {
    op.get(key).and_then(|v| v.as_f64()).unwrap_or(default)
}

fn bool_param(op: &Value, key: &str, default: bool) -> bool {
    op.get(key).and_then(|v| v.as_bool()).unwrap_or(default)
}

fn int_param(op: &Value, key: &str, default: i64) -> i64 {
    op.get(key).and_then(|v| v.as_i64()).unwrap_or(default)
}

/* ── Levels ─────────────────────────────────────────────────────── */

fn apply_levels(img: &mut PhotonImage, op: &Value) {
    let channel = op.get("channel").and_then(|v| v.as_str()).unwrap_or("rgb").to_ascii_lowercase();
    let in_black = f64_param(op, "in_black", 0.0).clamp(0.0, 255.0);
    let in_white = f64_param(op, "in_white", 255.0).clamp(0.0, 255.0);
    let gamma = f64_param(op, "gamma", 1.0).clamp(0.01, 9.99);
    let out_black = f64_param(op, "out_black", 0.0).clamp(0.0, 255.0);
    let out_white = f64_param(op, "out_white", 255.0).clamp(0.0, 255.0);
    let span = (in_white - in_black).max(1e-6);
    let inv_gamma = 1.0 / gamma;

    let map = |v: f64| -> f64 {
        let t = clamp((v - in_black) / span, 0.0, 1.0);
        let t = t.powf(inv_gamma);
        out_black + t * (out_white - out_black)
    };
    let which: [bool; 3] = [
        channel == "rgb" || channel == "red",
        channel == "rgb" || channel == "green",
        channel == "rgb" || channel == "blue",
    ];
    map_pixels(img, |r, g, b, a| {
        let mut out = [r as f64, g as f64, b as f64];
        for c in 0..3 {
            if which[c] {
                out[c] = map(out[c]);
            }
        }
        (to_u8(out[0]), to_u8(out[1]), to_u8(out[2]), a)
    });
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

/* ── Colour balance ─────────────────────────────────────────────── */

/// Per-pixel weight of the shadows / midtones / highlights bands.
fn tone_weights(l: f64) -> (f64, f64, f64) {
    let shadows = (1.0 - l).powi(3);
    let highlights = l.powi(3);
    let midtones = (1.0 - shadows - highlights).max(0.0);
    (shadows, midtones, highlights)
}

fn apply_color_balance(img: &mut PhotonImage, op: &Value) {
    let read = |key: &str| -> [f64; 3] {
        op.get(key)
            .and_then(|v| v.as_array())
            .map(|a| {
                let g = |i: usize| a.get(i).and_then(|v| v.as_f64()).unwrap_or(0.0);
                [g(0), g(1), g(2)]
            })
            .unwrap_or([0.0, 0.0, 0.0])
    };
    let sh = read("shadows");
    let mid = read("midtones");
    let hi = read("highlights");
    let preserve = bool_param(op, "preserve_luminosity", true);
    if sh == [0.0; 3] && mid == [0.0; 3] && hi == [0.0; 3] {
        return;
    }
    map_pixels(img, |r, g, b, a| {
        let lum = luminance(r as f64, g as f64, b as f64) / 255.0;
        let blend_sh = luminance(r as f64, g as f64, b as f64) / 255.0;
        let (ws, wm, wh) = tone_weights(blend_sh);
        let mut out = [r as f64, g as f64, b as f64];
        for c in 0..3 {
            let delta = sh[c] * ws + mid[c] * wm + hi[c] * wh;
            // +100 shifts a channel fully to its opposite: scale by 1.275 so a
            // 100 setting is a visible but non-clipping push.
            out[c] += delta * 1.275;
        }
        let _ = lum;
        if preserve {
            let old = luminance(r as f64, g as f64, b as f64);
            let new = luminance(out[0], out[1], out[2]);
            if new.abs() > 1e-6 {
                let scale = old / new;
                for c in 0..3 {
                    out[c] *= scale;
                }
            }
        }
        (to_u8(out[0]), to_u8(out[1]), to_u8(out[2]), a)
    });
}

/* ── Desaturate / black & white ─────────────────────────────────── */

fn apply_desaturate(img: &mut PhotonImage, op: &Value) {
    let amount = f64_param(op, "amount", 100.0).clamp(0.0, 100.0) / 100.0;
    let wr = f64_param(op, "red", 0.3);
    let wg = f64_param(op, "green", 0.59);
    let wb = f64_param(op, "blue", 0.11);
    let sum = (wr + wg + wb).max(1e-6);
    map_pixels(img, |r, g, b, a| {
        let gray = (r as f64 * wr + g as f64 * wg + b as f64 * wb) / sum;
        (
            to_u8(lerp(r as f64, gray, amount)),
            to_u8(lerp(g as f64, gray, amount)),
            to_u8(lerp(b as f64, gray, amount)),
            a,
        )
    });
}

fn apply_black_white(img: &mut PhotonImage, op: &Value) {
    let reds = f64_param(op, "reds", 40.0);
    let yellows = f64_param(op, "yellows", 60.0);
    let greens = f64_param(op, "greens", 40.0);
    let cyans = f64_param(op, "cyans", 60.0);
    let blues = f64_param(op, "blues", 20.0);
    let magentas = f64_param(op, "magentas", 80.0);
    map_pixels(img, |r, g, b, a| {
        let (h, s, l) = rgb_to_hsl(r as f64, g as f64, b as f64);
        let hue = h / 60.0; // 0=R 1=Y 2=G 3=C 4=B 5=M
        let weights = [reds, yellows, greens, cyans, blues, magentas];
        let i = hue.floor().rem_euclid(6.0) as usize;
        let t = hue - hue.floor();
        let w = lerp(weights[i], weights[(i + 1) % 6], t);
        let gray = (l * 100.0) * (1.0 + w / 100.0);
        let gray = clamp(gray, 0.0, 255.0) * (1.0 - s * 0.0);
        let v = to_u8(gray);
        let _ = (s, l);
        (v, v, v, a)
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

/* ── Exposure / vibrance / posterize ────────────────────────────── */

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

fn apply_posterize(img: &mut PhotonImage, op: &Value) {
    let levels = int_param(op, "levels", 4).clamp(2, 255) as f64;
    let step = 255.0 / (levels - 1.0);
    map_pixels(img, |r, g, b, a| {
        let q = |v: u8| to_u8(((v as f64 / step).round()) * step);
        (q(r), q(g), q(b), a)
    });
}

/* ── Gradient map / channel mixer / photo filter ────────────────── */

fn parse_stops(op: &Value) -> Vec<(f64, [f64; 3])> {
    let mut stops: Vec<(f64, [f64; 3])> = Vec::new();
    if let Some(arr) = op.get("colors").and_then(|v| v.as_array()) {
        let n = arr.len();
        for (i, item) in arr.iter().enumerate() {
            let color = if let Some(c) = item.as_array() {
                let g = |k: usize| c.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0);
                [g(0), g(1), g(2)]
            } else if let Some(o) = item.as_object() {
                let g = |k: &str| o.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0);
                [g("r"), g("g"), g("b")]
            } else {
                continue;
            };
            let pos = item
                .get("pos")
                .and_then(|v| v.as_f64())
                .unwrap_or(if n > 1 { i as f64 / (n - 1) as f64 } else { 0.0 });
            stops.push((clamp(pos, 0.0, 1.0), color));
        }
    }
    if stops.is_empty() {
        stops = vec![(0.0, [0.0, 0.0, 0.0]), (1.0, [255.0, 255.0, 255.0])];
    }
    if bool_param(op, "reverse", false) {
        stops.reverse();
        for s in stops.iter_mut() {
            s.0 = 1.0 - s.0;
        }
        stops.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    }
    stops
}

fn sample_stops(stops: &[(f64, [f64; 3])], t: f64) -> [f64; 3] {
    if stops.is_empty() {
        return [0.0, 0.0, 0.0];
    }
    if t <= stops[0].0 {
        return stops[0].1;
    }
    let last = stops[stops.len() - 1];
    if t >= last.0 {
        return last.1;
    }
    for i in 0..stops.len() - 1 {
        let (p0, c0) = stops[i];
        let (p1, c1) = stops[i + 1];
        if t >= p0 && t <= p1 {
            let f = if (p1 - p0).abs() < 1e-9 { 0.0 } else { (t - p0) / (p1 - p0) };
            return [lerp(c0[0], c1[0], f), lerp(c0[1], c1[1], f), lerp(c0[2], c1[2], f)];
        }
    }
    last.1
}

fn apply_gradient_map(img: &mut PhotonImage, op: &Value) {
    let stops = parse_stops(op);
    let use_luma = bool_param(op, "use_luminance", true);
    map_pixels(img, |r, g, b, a| {
        let t = if use_luma {
            luminance(r as f64, g as f64, b as f64) / 255.0
        } else {
            (r as f64 + g as f64 + b as f64) / 765.0
        };
        let c = sample_stops(&stops, t);
        (to_u8(c[0]), to_u8(c[1]), to_u8(c[2]), a)
    });
}

fn apply_channel_mixer(img: &mut PhotonImage, op: &Value) {
    let read = |key: &str| -> [f64; 3] {
        op.get(key)
            .and_then(|v| v.as_array())
            .map(|a| {
                let g = |i: usize| a.get(i).and_then(|v| v.as_f64()).unwrap_or(if i == 0 { 100.0 } else { 0.0 });
                [g(0) / 100.0, g(1) / 100.0, g(2) / 100.0]
            })
            .unwrap_or([0.0, 0.0, 0.0])
    };
    let monochrome = bool_param(op, "monochrome", false);
    let gray = read("gray");
    let (rr, gr, br) = if monochrome { (gray, gray, gray) } else { (read("red"), read("green"), read("blue")) };
    if !monochrome && rr == [0.0, 0.0, 0.0] && gr == [0.0, 0.0, 0.0] && br == [0.0, 0.0, 0.0] {
        return;
    }
    map_pixels(img, |r, g, b, a| {
        let (rf, gf, bf) = (r as f64, g as f64, b as f64);
        let nr = rf * rr[0] + gf * rr[1] + bf * rr[2];
        let ng = rf * gr[0] + gf * gr[1] + bf * gr[2];
        let nb = rf * br[0] + gf * br[1] + bf * br[2];
        if monochrome {
            let v = to_u8(nr);
            (v, v, v, a)
        } else {
            (to_u8(nr), to_u8(ng), to_u8(nb), a)
        }
    });
}

fn apply_photo_filter(img: &mut PhotonImage, op: &Value) {
    let filter = [
        f64_param(op, "r", 236.0),
        f64_param(op, "g", 138.0),
        f64_param(op, "b", 0.0),
    ];
    let density = clamp(f64_param(op, "density", 25.0), 0.0, 100.0) / 100.0;
    let preserve = bool_param(op, "preserve_luminosity", true);
    map_pixels(img, |r, g, b, a| {
        let src = [r as f64, g as f64, b as f64];
        let mut out = [0.0f64; 3];
        for c in 0..3 {
            out[c] = lerp(src[c], filter[c], density);
        }
        if preserve {
            let old = luminance(src[0], src[1], src[2]);
            let new = luminance(out[0], out[1], out[2]);
            if new.abs() > 1e-6 {
                let scale = old / new;
                for c in 0..3 {
                    out[c] *= scale;
                }
            }
        }
        (to_u8(out[0]), to_u8(out[1]), to_u8(out[2]), a)
    });
}

/// Dispatch one adjustment. Returns `Ok(true)` when handled.
pub fn apply(img: &mut PhotonImage, name: &str, op: &Value) -> Result<bool, String> {
    match name {
        "levels" => apply_levels(img, op),
        "hue_saturation" | "hue" => apply_hue_saturation(img, op),
        "color_balance" => apply_color_balance(img, op),
        "desaturate" => apply_desaturate(img, op),
        "black_and_white" | "black_white" => apply_black_white(img, op),
        "auto_contrast" => apply_auto_contrast(img, op),
        "auto_tone" => apply_auto_tone(img, op),
        "exposure" => apply_exposure(img, op),
        "vibrance" => apply_vibrance(img, op),
        "posterize" => apply_posterize(img, op),
        "gradient_map" => apply_gradient_map(img, op),
        "channel_mixer" => apply_channel_mixer(img, op),
        "photo_filter" => apply_photo_filter(img, op),
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
    fn levels_identity_is_a_noop() {
        let px = vec![10, 20, 30, 255, 200, 210, 220, 128];
        let mut i = img(px.clone(), 2, 1);
        apply(&mut i, "levels", &json!({"in_black": 0, "in_white": 255, "gamma": 1.0})).unwrap();
        assert_eq!(i.get_raw_pixels(), px);
    }

    #[test]
    fn posterize_two_levels_quantises() {
        let mut i = img(vec![10, 200, 128, 255], 1, 1);
        apply(&mut i, "posterize", &json!({"levels": 2})).unwrap();
        let p = i.get_raw_pixels();
        assert!(p[0] == 0 || p[0] == 255);
        assert_eq!(p[1], 255);
    }

    #[test]
    fn desaturate_full_removes_colour() {
        let mut i = img(vec![200, 40, 20, 255], 1, 1);
        apply(&mut i, "desaturate", &json!({"amount": 100})).unwrap();
        let p = i.get_raw_pixels();
        assert_eq!(p[0], p[1]);
        assert_eq!(p[1], p[2]);
    }

    #[test]
    fn hue_saturation_zero_is_a_noop() {
        let px = vec![120, 60, 200, 255];
        let mut i = img(px.clone(), 1, 1);
        apply(&mut i, "hue_saturation", &json!({"hue": 0, "saturation": 0, "lightness": 0})).unwrap();
        assert_eq!(i.get_raw_pixels(), px);
    }
}

