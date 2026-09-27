//! Painting, filling and geometry rasterisation.
//!
//! Brush/pencil/eraser/dodge/burn/blur/sharpen strokes, flood-fill bucket,
//! gradients, shape rasterisation, clipboard paste and a free-transform remap.
//! All of it is deterministic pixel work — no external service is involved.

use photon_rs::PhotonImage;
use serde_json::Value;

use crate::pixels::{clamp, from_raw, lerp, luminance, raw, sample_bilinear, to_u8};

fn f64_param(op: &Value, key: &str, default: f64) -> f64 {
    op.get(key).and_then(|v| v.as_f64()).unwrap_or(default)
}

fn int_param(op: &Value, key: &str, default: i64) -> i64 {
    op.get(key).and_then(|v| v.as_i64()).unwrap_or(default)
}

fn bool_param(op: &Value, key: &str, default: bool) -> bool {
    op.get(key).and_then(|v| v.as_bool()).unwrap_or(default)
}

fn str_param<'a>(op: &'a Value, key: &str, default: &'a str) -> &'a str {
    op.get(key).and_then(|v| v.as_str()).unwrap_or(default)
}

/// Read a colour from `key` as `[r, g, b, a]`; accepts `[r,g,b]`, `[r,g,b,a]`,
/// `{r,g,b,a}` or a `#rrggbb` string.
pub fn color_param(op: &Value, key: &str, default: [u8; 4]) -> [u8; 4] {
    let Some(v) = op.get(key) else { return default };
    if let Some(s) = v.as_str() {
        let s = s.trim_start_matches('#');
        if s.len() >= 6 {
            let p = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).unwrap_or(0);
            let a = if s.len() >= 8 {
                u8::from_str_radix(&s[6..8], 16).unwrap_or(255)
            } else {
                255
            };
            return [p(0), p(2), p(4), a];
        }
    }
    if let Some(a) = v.as_array() {
        let g = |i: usize, d: f64| a.get(i).and_then(|x| x.as_f64()).unwrap_or(d);
        return [
            g(0, 0.0).clamp(0.0, 255.0) as u8,
            g(1, 0.0).clamp(0.0, 255.0) as u8,
            g(2, 0.0).clamp(0.0, 255.0) as u8,
            g(3, 255.0).clamp(0.0, 255.0) as u8,
        ];
    }
    let g = |k: &str, d: f64| v.get(k).and_then(|x| x.as_f64()).unwrap_or(d);
    if v.get("r").is_some() {
        return [
            g("r", 0.0).clamp(0.0, 255.0) as u8,
            g("g", 0.0).clamp(0.0, 255.0) as u8,
            g("b", 0.0).clamp(0.0, 255.0) as u8,
            g("a", 255.0).clamp(0.0, 255.0) as u8,
        ];
    }
    default
}

/// Source-over one straight pixel. `mode` is a blend-mode wire name; only a
/// small set is meaningful for painting and everything else falls back to
/// `normal`.
fn blend_into(px: &mut [u8], i: usize, src: [f64; 3], sa: f64, mode: &str) {
    if sa <= 0.0 {
        return;
    }
    let da = px[i + 3] as f64 / 255.0;
    let src = [src[0], src[1], src[2]];
    let mut cb = [0.0f64; 3];
    for c in 0..3 {
        cb[c] = px[i + c] as f64;
    }
    let blended = |c: usize| -> f64 {
        let b = match mode {
            "multiply" => cb[c] * src[c],
            "screen" => 255.0 - (255.0 - cb[c]) * (255.0 - src[c]) / 255.0,
            "overlay" => {
                if cb[c] <= 127.5 {
                    2.0 * cb[c] * src[c] / 255.0
                } else {
                    255.0 - 2.0 * (255.0 - cb[c]) * (255.0 - src[c]) / 255.0
                }
            }
            "darken" => cb[c].min(src[c]),
            "lighten" => cb[c].max(src[c]),
            "difference" => (cb[c] - src[c]).abs(),
            "add" => (cb[c] + src[c]).min(255.0),
            _ => src[c],
        };
        b
    };
    let out_a = sa + da * (1.0 - sa);
    for c in 0..3 {
        let val = (blended(c) * sa + cb[c] * da * (1.0 - sa)) / out_a.max(1e-6);
        px[i + c] = to_u8(val);
    }
    px[i + 3] = to_u8(out_a * 255.0);
}

/// Normalised brush falloff at distance `d` for `radius` with `hardness` 0..1.
fn falloff(d: f64, radius: f64, hardness: f64) -> f64 {
    if d >= radius {
        return 0.0;
    }
    let inner = (hardness * radius).max(0.0);
    if d <= inner {
        1.0
    } else {
        let t = (d - inner) / (radius - inner).max(1e-6);
        1.0 - t
    }
}

/// A dense polyline of stamp positions (points with `pressure` in 0..1).
fn stroke_positions(op: &Value, spacing: f64) -> Vec<(f64, f64, f64)> {
    let pts: Vec<(f64, f64, f64)> = op
        .get("points")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|p| {
                    if let Some(a) = p.as_array() {
                        (
                            a.first().and_then(|v| v.as_f64()).unwrap_or(0.0),
                            a.get(1).and_then(|v| v.as_f64()).unwrap_or(0.0),
                            a.get(2).and_then(|v| v.as_f64()).unwrap_or(1.0),
                        )
                    } else {
                        (
                            p.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0),
                            p.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0),
                            p.get("pressure").and_then(|v| v.as_f64()).unwrap_or(1.0),
                        )
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    if pts.is_empty() {
        return Vec::new();
    }
    let spacing = spacing.max(0.5);
    let mut out = vec![pts[0]];
    for pair in pts.windows(2) {
        let (x0, y0, p0) = pair[0];
        let (x1, y1, p1) = pair[1];
        let dist = ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt();
        let n = (dist / spacing).ceil() as i64;
        for i in 1..=n.max(1) {
            let t = i as f64 / n.max(1) as f64;
            out.push((lerp(x0, x1, t), lerp(y0, y1, t), lerp(p0, p1, t)));
        }
    }
    out
}

fn push_clamped(v: &mut [f32], i: usize, x: f64) {
    if i >= v.len() {
        return;
    }
    let nv = (v[i] as f64 + x) as f32;
    v[i] = nv.min(1.0);
}

/// Brush / pencil / eraser / dodge / burn / blur / sharpen / smudge stroke.
fn paint(img: &mut PhotonImage, op: &Value) {
    let w = img.get_width();
    let h = img.get_height();
    let mode = str_param(op, "mode", "brush");
    let radius = f64_param(op, "radius", 16.0).clamp(0.5, 400.0);
    let hardness = (f64_param(op, "hardness", 100.0) / 100.0).clamp(0.0, 1.0);
    let opacity = (f64_param(op, "opacity", 100.0) / 100.0).clamp(0.0, 1.0);
    let flow = (f64_param(op, "flow", 100.0) / 100.0).clamp(0.0, 1.0);
    let strength = (f64_param(op, "strength", 30.0) / 100.0).clamp(0.0, 1.0);
    let color = color_param(op, "color", [0, 0, 0, 255]);
    let clone_offset = op
        .get("offset")
        .and_then(|v| v.as_array())
        .map(|a| {
            (
                a.first().and_then(|v| v.as_f64()).unwrap_or(0.0),
                a.get(1).and_then(|v| v.as_f64()).unwrap_or(0.0),
            )
        })
        .unwrap_or((0.0, 0.0));
    let spacing = (radius * 0.2).max(1.0);
    let stamps = stroke_positions(op, spacing);
    if stamps.is_empty() {
        return;
    }
    let mut cov = vec![0f32; (w as usize) * (h as usize)];
    for (cx, cy, pressure) in stamps {
        let r = radius * (0.25 + 0.75 * pressure.clamp(0.0, 1.0));
        let x0 = (cx - r).floor().max(0.0) as i64;
        let x1 = (cx + r).ceil().min(w as f64 - 1.0) as i64;
        let y0 = (cy - r).floor().max(0.0) as i64;
        let y1 = (cy + r).ceil().min(h as f64 - 1.0) as i64;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let d = ((x as f64 - cx).powi(2) + (y as f64 - cy).powi(2)).sqrt();
                let f = falloff(d, r, hardness);
                if f > 0.0 {
                    push_clamped(&mut cov, (y as usize) * (w as usize) + x as usize, f * flow);
                }
            }
        }
    }
    let base = raw(img);
    let mut out = base.clone();
    // A blurred copy, only for the modes that need one (1×1 images make the
    // convolution helper unhappy, and the other modes never touch it).
    let needs_blur = matches!(mode, "blur" | "sharpen" | "smudge") && w > 2 && h > 2;
    let blur = if needs_blur {
        let mut blurred = img.clone();
        photon_rs::conv::gaussian_blur(&mut blurred, (radius / 2.0).max(1.0) as i32);
        raw(&blurred)
    } else {
        Vec::new()
    };
    for i in 0..(w as usize) * (h as usize) {
        let c = cov[i] as f64 * opacity;
        if c <= 0.0 {
            continue;
        }
        let p = i * 4;
        match mode {
            "eraser" | "erase" => {
                let da = base[p + 3] as f64 / 255.0;
                out[p + 3] = to_u8((da * (1.0 - c).max(0.0)) * 255.0);
            }
            "dodge" | "burn" => {
                let sign = if mode == "burn" { -1.0 } else { 1.0 };
                for ch in 0..3 {
                    let v = base[p + ch] as f64 / 255.0;
                    let adjusted = if sign > 0.0 {
                        v + (1.0 - v) * strength * c
                    } else {
                        v - v * strength * c
                    };
                    out[p + ch] = to_u8(adjusted * 255.0);
                }
            }
            "blur" => {
                for ch in 0..3 {
                    out[p + ch] = to_u8(lerp(
                        base[p + ch] as f64,
                        blur[p + ch] as f64,
                        strength * c,
                    ));
                }
            }
            "sharpen" => {
                // Unsharp: base + (base - blur) * strength.
                for ch in 0..3 {
                    let diff = base[p + ch] as f64 - blur[p + ch] as f64;
                    out[p + ch] = to_u8(base[p + ch] as f64 + diff * strength * c * 2.0);
                }
            }
            "smudge" => {
                // Average a small neighbourhood, weighted by the brush.
                for ch in 0..3 {
                    let mut acc = 0.0;
                    let mut n = 0.0;
                    for oy in -2i64..=2 {
                        for ox in -2i64..=2 {
                            let xx = ((i % w as usize) as i64 + ox).clamp(0, w as i64 - 1) as usize;
                            let yy = ((i / w as usize) as i64 + oy).clamp(0, h as i64 - 1) as usize;
                            acc += base[(yy * w as usize + xx) * 4 + ch] as f64;
                            n += 1.0;
                        }
                    }
                    out[p + ch] = to_u8(lerp(base[p + ch] as f64, acc / n, strength * c));
                }
            }
            "clone" | "pattern" => {
                // Sample the source layer at a fixed offset and paint it down.
                let sx = (i % w as usize) as f64 + clone_offset.0;
                let sy = (i / w as usize) as f64 + clone_offset.1;
                let s = sample_bilinear(&base, w, h, sx, sy);
                blend_into(
                    &mut out,
                    p,
                    [s[0] as f64, s[1] as f64, s[2] as f64],
                    (s[3] as f64 / 255.0) * c,
                    "normal",
                );
            }
            _ => {
                blend_into(&mut out, p, [color[0] as f64, color[1] as f64, color[2] as f64],
                    (color[3] as f64 / 255.0) * c, str_param(op, "blend", "normal"));
            }
        }
    }
    *img = from_raw(out, w, h);
}

/// Flood-fill bucket. `contiguous` limits the fill to the connected region.
fn bucket(img: &mut PhotonImage, op: &Value) {
    let w = img.get_width();
    let h = img.get_height();
    let sx = int_param(op, "x", 0).clamp(0, w as i64 - 1) as usize;
    let sy = int_param(op, "y", 0).clamp(0, h as i64 - 1) as usize;
    let tolerance = f64_param(op, "tolerance", 32.0).clamp(0.0, 255.0);
    let contiguous = bool_param(op, "contiguous", true);
    let opacity = (f64_param(op, "opacity", 100.0) / 100.0).clamp(0.0, 1.0);
    let color = color_param(op, "color", [0, 0, 0, 255]);
    let mode = str_param(op, "blend", "normal");
    let px = raw(img);
    let seed = [
        px[(sy * w as usize + sx) * 4] as f64,
        px[(sy * w as usize + sx) * 4 + 1] as f64,
        px[(sy * w as usize + sx) * 4 + 2] as f64,
        px[(sy * w as usize + sx) * 4 + 3] as f64,
    ];
    let close = |i: usize| -> bool {
        let d = (0..4)
            .map(|c| (px[i + c] as f64 - seed[c]).abs())
            .fold(0.0f64, f64::max);
        d <= tolerance
    };
    let mut selected = vec![false; (w as usize) * (h as usize)];
    if contiguous {
        let mut stack = vec![(sx, sy)];
        while let Some((x, y)) = stack.pop() {
            let idx = y * w as usize + x;
            if selected[idx] || !close(idx * 4) {
                continue;
            }
            selected[idx] = true;
            if x > 0 {
                stack.push((x - 1, y));
            }
            if x + 1 < w as usize {
                stack.push((x + 1, y));
            }
            if y > 0 {
                stack.push((x, y - 1));
            }
            if y + 1 < h as usize {
                stack.push((x, y + 1));
            }
        }
    } else {
        for i in 0..(w as usize) * (h as usize) {
            if close(i * 4) {
                selected[i] = true;
            }
        }
    }
    let mut out = px.clone();
    for i in 0..(w as usize) * (h as usize) {
        if selected[i] {
            blend_into(
                &mut out,
                i * 4,
                [color[0] as f64, color[1] as f64, color[2] as f64],
                (color[3] as f64 / 255.0) * opacity,
                mode,
            );
        }
    }
    *img = from_raw(out, w, h);
}

/* ── Gradients and solid fill ───────────────────────────────────── */

fn stop_rgba(item: &Value) -> ([f64; 4], f64) {
    let pos = item.get("pos").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let color = if let Some(a) = item.as_array() {
        let g = |i: usize, d: f64| a.get(i).and_then(|v| v.as_f64()).unwrap_or(d);
        [g(0, 0.0), g(1, 0.0), g(2, 0.0), g(3, 255.0)]
    } else {
        let g = |k: &str, d: f64| item.get(k).and_then(|v| v.as_f64()).unwrap_or(d);
        [g("r", 0.0), g("g", 0.0), g("b", 0.0), g("a", 255.0)]
    };
    (color, clamp(pos, 0.0, 1.0))
}

fn gradient_stops(op: &Value) -> Vec<([f64; 4], f64)> {
    let mut stops: Vec<([f64; 4], f64)> = op
        .get("stops")
        .and_then(|v| v.as_array())
        .map(|arr| {
            let n = arr.len();
            arr.iter()
                .enumerate()
                .map(|(i, item)| {
                    let (c, mut p) = stop_rgba(item);
                    if item.get("pos").is_none() && n > 1 {
                        p = i as f64 / (n - 1) as f64;
                    }
                    (c, p)
                })
                .collect()
        })
        .unwrap_or_default();
    if stops.is_empty() {
        let fg = color_param(op, "color", [0, 0, 0, 255]);
        let bg = color_param(op, "color2", [255, 255, 255, 255]);
        stops = vec![
            ([bg[0] as f64, bg[1] as f64, bg[2] as f64, bg[3] as f64], 0.0),
            ([fg[0] as f64, fg[1] as f64, fg[2] as f64, fg[3] as f64], 1.0),
        ];
    }
    stops.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    if bool_param(op, "reverse", false) {
        stops.reverse();
        for s in stops.iter_mut() {
            s.1 = 1.0 - s.1;
        }
        stops.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    }
    stops
}

fn sample_gradient(stops: &[([f64; 4], f64)], t: f64) -> [f64; 4] {
    let t = clamp(t, 0.0, 1.0);
    if stops.is_empty() {
        return [0.0, 0.0, 0.0, 255.0];
    }
    if t <= stops[0].1 {
        return stops[0].0;
    }
    let last = stops[stops.len() - 1];
    if t >= last.1 {
        return last.0;
    }
    for i in 0..stops.len() - 1 {
        let (c0, p0) = stops[i];
        let (c1, p1) = stops[i + 1];
        if t >= p0 && t <= p1 {
            let f = if (p1 - p0).abs() < 1e-9 { 0.0 } else { (t - p0) / (p1 - p0) };
            return [
                lerp(c0[0], c1[0], f),
                lerp(c0[1], c1[1], f),
                lerp(c0[2], c1[2], f),
                lerp(c0[3], c1[3], f),
            ];
        }
    }
    last.0
}

fn gradient(img: &mut PhotonImage, op: &Value) {
    let w = img.get_width();
    let h = img.get_height();
    let kind = str_param(op, "gradient_type", "linear");
    let x1 = f64_param(op, "x1", 0.0);
    let y1 = f64_param(op, "y1", 0.0);
    let x2 = f64_param(op, "x2", 0.0);
    let y2 = f64_param(op, "y2", h as f64);
    let dx = x2 - x1;
    let dy = y2 - y1;
    let len2 = (dx * dx + dy * dy).max(1e-6);
    let len = len2.sqrt();
    let opacity = (f64_param(op, "opacity", 100.0) / 100.0).clamp(0.0, 1.0);
    let mode = str_param(op, "blend", "normal");
    let dither = bool_param(op, "dither", true);
    let stops = gradient_stops(op);
    let base = raw(img);
    let mut out = base.clone();
    for y in 0..h {
        for x in 0..w {
            let px = x as f64;
            let py = y as f64;
            let t = match kind {
                "radial" => {
                    let d = ((px - x1).powi(2) + (py - y1).powi(2)).sqrt();
                    d / len
                }
                "angle" => {
                    let ang = (py - y1).atan2(px - x1);
                    (ang + std::f64::consts::PI) / (std::f64::consts::TAU)
                }
                "reflected" => (((px - x1) * dx + (py - y1) * dy) / len2).abs(),
                "diamond" => {
                    let u = ((px - x1) * dx + (py - y1) * dy) / len2;
                    let v = ((px - x1) * -dy + (py - y1) * dx) / len2;
                    (u.abs() + v.abs()).clamp(0.0, 1.0)
                }
                _ => ((px - x1) * dx + (py - y1) * dy) / len2,
            };
            let mut c = sample_gradient(&stops, t);
            if dither {
                let i = (y as usize * w as usize + x as usize) * 4;
                let n = ((base[i] as f64 * 0.13 + base[i + 1] as f64 * 0.07) % 1.0) - 0.5;
                for ch in 0..3 {
                    c[ch] = (c[ch] + n).clamp(0.0, 255.0);
                }
            }
            blend_into(
                &mut out,
                (y as usize * w as usize + x as usize) * 4,
                [c[0], c[1], c[2]],
                (c[3] / 255.0) * opacity,
                mode,
            );
        }
    }
    *img = from_raw(out, w, h);
}

fn fill(img: &mut PhotonImage, op: &Value) {
    let color = color_param(op, "color", [0, 0, 0, 255]);
    let opacity = (f64_param(op, "opacity", 100.0) / 100.0).clamp(0.0, 1.0);
    let mode = str_param(op, "blend", "normal");
    let w = img.get_width();
    let h = img.get_height();
    let mut out = raw(img);
    for i in 0..(w as usize) * (h as usize) {
        blend_into(
            &mut out,
            i * 4,
            [color[0] as f64, color[1] as f64, color[2] as f64],
            (color[3] as f64 / 255.0) * opacity,
            mode,
        );
    }
    *img = from_raw(out, w, h);
}

fn clear(img: &mut PhotonImage) {
    let w = img.get_width();
    let h = img.get_height();
    *img = from_raw(vec![0u8; (w as usize) * (h as usize) * 4], w, h);
}

/// Paste a raw RGBA bitmap (base64, row-major, no filter) at (x, y).
fn paste(img: &mut PhotonImage, op: &Value) {
    use base64::Engine;
    let Some(data) = op.get("data").and_then(|v| v.as_str()) else { return };
    let decoded = match base64::engine::general_purpose::STANDARD.decode(data) {
        Ok(d) => d,
        Err(_) => return,
    };
    let bw = int_param(op, "width", 0).max(0) as u32;
    let bh = int_param(op, "height", 0).max(0) as u32;
    if bw == 0 || bh == 0 || decoded.len() < (bw as usize) * (bh as usize) * 4 {
        return;
    }
    let x0 = int_param(op, "x", 0) as i64;
    let y0 = int_param(op, "y", 0) as i64;
    let opacity = (f64_param(op, "opacity", 100.0) / 100.0).clamp(0.0, 1.0);
    let mode = str_param(op, "blend", "normal");
    let w = img.get_width();
    let h = img.get_height();
    let mut out = raw(img);
    for by in 0..bh {
        let dy = y0 + by as i64;
        if dy < 0 || dy >= h as i64 {
            continue;
        }
        for bx in 0..bw {
            let dx = x0 + bx as i64;
            if dx < 0 || dx >= w as i64 {
                continue;
            }
            let si = ((by * bw + bx) as usize) * 4;
            let sa = (decoded[si + 3] as f64 / 255.0) * opacity;
            if sa <= 0.0 {
                continue;
            }
            blend_into(
                &mut out,
                ((dy as usize) * (w as usize) + dx as usize) * 4,
                [decoded[si] as f64, decoded[si + 1] as f64, decoded[si + 2] as f64],
                sa,
                mode,
            );
        }
    }
    *img = from_raw(out, w, h);
}

/* ── Shape rasterisation ────────────────────────────────────────── */

fn point_in_poly(pts: &[[f64; 2]], x: f64, y: f64) -> bool {
    let mut inside = false;
    let n = pts.len();
    if n < 3 {
        return false;
    }
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = (pts[i][0], pts[i][1]);
        let (xj, yj) = (pts[j][0], pts[j][1]);
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

fn dist_to_seg(px: f64, py: f64, x0: f64, y0: f64, x1: f64, y1: f64) -> f64 {
    let dx = x1 - x0;
    let dy = y1 - y0;
    let l2 = dx * dx + dy * dy;
    let t = if l2 < 1e-9 { 0.0 } else { (((px - x0) * dx + (py - y0) * dy) / l2).clamp(0.0, 1.0) };
    let cx = x0 + t * dx;
    let cy = y0 + t * dy;
    ((px - cx).powi(2) + (py - cy).powi(2)).sqrt()
}

fn dist_to_poly(pts: &[[f64; 2]], x: f64, y: f64) -> f64 {
    let n = pts.len();
    let mut best = f64::MAX;
    for i in 0..n {
        let j = (i + 1) % n;
        best = best.min(dist_to_seg(x, y, pts[i][0], pts[i][1], pts[j][0], pts[j][1]));
    }
    best
}

fn regular_poly(cx: f64, cy: f64, r: f64, sides: usize) -> Vec<[f64; 2]> {
    (0..sides)
        .map(|i| {
            let a = -std::f64::consts::FRAC_PI_2 + i as f64 * std::f64::consts::TAU / sides as f64;
            [cx + r * a.cos(), cy + r * a.sin()]
        })
        .collect()
}

fn star_poly(cx: f64, cy: f64, r: f64, points: usize, ratio: f64) -> Vec<[f64; 2]> {
    let mut out = Vec::with_capacity(points * 2);
    for i in 0..points * 2 {
        let a = -std::f64::consts::FRAC_PI_2 + i as f64 * std::f64::consts::PI / points as f64;
        let rr = if i % 2 == 0 { r } else { r * ratio };
        out.push([cx + rr * a.cos(), cy + rr * a.sin()]);
    }
    out
}

fn shape_vertices(op: &Value) -> Option<Vec<[f64; 2]>> {
    let kind = str_param(op, "kind", "rect");
    let x = f64_param(op, "x", 0.0);
    let y = f64_param(op, "y", 0.0);
    let w = f64_param(op, "width", 100.0);
    let h = f64_param(op, "height", 100.0);
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let (rx, ry) = (w / 2.0, h / 2.0);
    Some(match kind {
        "rect" | "rectangle" | "ellipse" | "circle" | "line" => return None,
        "triangle" => vec![[cx, cy - ry], [cx - rx, cy + ry], [cx + rx, cy + ry]],
        "polygon" => regular_poly(cx, cy, rx.min(ry), int_param(op, "sides", 6).clamp(3, 64) as usize),
        "star" => {
            let points = int_param(op, "points", 5).clamp(3, 64) as usize;
            let ratio = (f64_param(op, "inner_ratio", 50.0) / 100.0).clamp(0.05, 1.0);
            star_poly(cx, cy, rx.min(ry), points, ratio)
        }
        "custom" => op
            .get("points")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|p| {
                        let a = p.as_array()?;
                        Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?])
                    })
                    .collect()
            })
            .unwrap_or_default(),
        _ => return None,
    })
}

fn shape(img: &mut PhotonImage, op: &Value) {
    let kind = str_param(op, "kind", "rect");
    let x = f64_param(op, "x", 0.0);
    let y = f64_param(op, "y", 0.0);
    let w = f64_param(op, "width", 100.0);
    let h = f64_param(op, "height", 100.0);
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let (rx, ry) = ((w / 2.0).max(0.5), (h / 2.0).max(0.5));
    let fill_color = op.get("fill").map(|_| color_param(op, "fill", [0, 0, 0, 255]));
    let stroke_color = op.get("stroke").map(|_| color_param(op, "stroke", [0, 0, 0, 255]));
    let stroke_w = f64_param(op, "stroke_width", 2.0).max(0.5);
    let opacity = (f64_param(op, "opacity", 100.0) / 100.0).clamp(0.0, 1.0);
    let mode = str_param(op, "blend", "normal");
    let verts = shape_vertices(op);
    let img_w = img.get_width();
    let img_h = img.get_height();
    let mut out = raw(img);
    let ss = 3;
    let n = (ss * ss) as f64;
    for py in 0..img_h {
        for px in 0..img_w {
            let mut fill_cov = 0.0;
            let mut stroke_cov = 0.0;
            for sy in 0..ss {
                for sx in 0..ss {
                    let px_ = px as f64 + (sx as f64 + 0.5) / ss as f64;
                    let py_ = py as f64 + (sy as f64 + 0.5) / ss as f64;
                    let (ins, edge) = match kind {
                        "rect" | "rectangle" => {
                            let inside = px_ >= x && px_ <= x + w && py_ >= y && py_ <= y + h;
                            let dx = (px_ - x).min(x + w - px_).abs();
                            let dy = (py_ - y).min(y + h - py_).abs();
                            (inside, dx.min(dy))
                        }
                        "ellipse" | "circle" => {
                            let v = ((px_ - cx) / rx).powi(2) + ((py_ - cy) / ry).powi(2);
                            let d = (v.sqrt() - 1.0).abs() * rx.min(ry);
                            (v <= 1.0, d)
                        }
                        "line" => {
                            let d = dist_to_seg(px_, py_, x, y, x + w, y + h);
                            (false, d)
                        }
                        _ => match &verts {
                            Some(pts) => (point_in_poly(pts, px_, py_), dist_to_poly(pts, px_, py_)),
                            None => (false, f64::MAX),
                        },
                    };
                    if ins {
                        fill_cov += 1.0;
                    }
                    if edge <= stroke_w / 2.0 {
                        stroke_cov += 1.0;
                    }
                }
            }
            let i = (py as usize * img_w as usize + px as usize) * 4;
            if let Some(c) = fill_color {
                let a = (fill_cov / n) * opacity * (c[3] as f64 / 255.0);
                if a > 0.0 {
                    blend_into(&mut out, i, [c[0] as f64, c[1] as f64, c[2] as f64], a, mode);
                }
            }
            if let Some(c) = stroke_color {
                let a = (stroke_cov / n) * opacity * (c[3] as f64 / 255.0);
                if a > 0.0 {
                    blend_into(&mut out, i, [c[0] as f64, c[1] as f64, c[2] as f64], a, mode);
                }
            }
        }
    }
    *img = from_raw(out, img_w, img_h);
}

/* ── Free transform ─────────────────────────────────────────────── */

/// Affine free transform about an origin. `matrix` may be passed directly as
/// `[a,b,c,d,e,f]`, otherwise it is composed from rotate/scale/skew/move.
fn transform(img: &mut PhotonImage, op: &Value) {
    let w = img.get_width();
    let h = img.get_height();
    let (a, b, c, d, e, f) = if let Some(m) = op.get("matrix").and_then(|v| v.as_array()) {
        let g = |i: usize, dv: f64| m.get(i).and_then(|v| v.as_f64()).unwrap_or(dv);
        (g(0, 1.0), g(1, 0.0), g(2, 0.0), g(3, 1.0), g(4, 0.0), g(5, 0.0))
    } else {
        let rot = f64_param(op, "rotate", 0.0).to_radians();
        let sx = f64_param(op, "scale_x", 1.0);
        let sy = f64_param(op, "scale_y", 1.0);
        let kx = f64_param(op, "skew_x", 0.0).to_radians();
        let ky = f64_param(op, "skew_y", 0.0).to_radians();
        let ox = f64_param(op, "origin_x", w as f64 / 2.0);
        let oy = f64_param(op, "origin_y", h as f64 / 2.0);
        let tx = f64_param(op, "move_x", 0.0);
        let ty = f64_param(op, "move_y", 0.0);
        let (sin, cos) = rot.sin_cos();
        let tanx = kx.tan();
        let tany = ky.tan();
        // scale → shear → rotate
        let m0 = cos * sx;
        let m1 = -sin * sx;
        let m2 = cos * tanx * sy + sin * sy;
        let m3 = -sin * tanx * sy + cos * sy + tany;
        (
            m0,
            m1,
            m2,
            m3,
            ox + tx - m0 * ox - m2 * oy,
            oy + ty - m1 * ox - m3 * oy,
        )
    };
    let det = a * d - b * c;
    if det.abs() < 1e-9 {
        return;
    }
    let src = raw(img);
    let mut out = src.clone();
    for y in 0..h {
        for x in 0..w {
            let dx = x as f64 + 0.5 - e;
            let dy = y as f64 + 0.5 - f;
            let sx = (d * dx - c * dy) / det;
            let sy = (-b * dx + a * dy) / det;
            let s = sample_bilinear(&src, w, h, sx - 0.5, sy - 0.5);
            let o = ((y as usize) * (w as usize) + x as usize) * 4;
            out[o..o + 4].copy_from_slice(&s);
        }
    }
    *img = from_raw(out, w, h);
}

fn replace_color(img: &mut PhotonImage, op: &Value) {
    let from = color_param(op, "from", [255, 0, 0, 255]);
    let to = color_param(op, "to", [0, 0, 255, 255]);
    let tolerance = f64_param(op, "tolerance", 40.0).clamp(0.0, 255.0);
    let amount = (f64_param(op, "opacity", 100.0) / 100.0).clamp(0.0, 1.0);
    let w = img.get_width();
    let h = img.get_height();
    let mut px = raw(img);
    for i in 0..(w as usize) * (h as usize) {
        let p = i * 4;
        let d = (0..3)
            .map(|c| (px[p + c] as f64 - from[c] as f64).abs())
            .fold(0.0f64, f64::max);
        if d <= tolerance {
            let mix = amount * (1.0 - d / (tolerance.max(1.0)));
            for c in 0..3 {
                px[p + c] = to_u8(lerp(px[p + c] as f64, to[c] as f64, mix));
            }
        }
    }
    *img = from_raw(px, w, h);
}

/// The luminance of a pixel, used by a few call sites that only need ordering.
#[allow(dead_code)]
fn lum_at(px: &[u8], i: usize) -> f64 {
    luminance(px[i] as f64, px[i + 1] as f64, px[i + 2] as f64)
}

/// Dispatch one paint/geometry operation. Returns `Ok(true)` when handled.
pub fn apply(img: &mut PhotonImage, name: &str, op: &Value) -> Result<bool, String> {
    match name {
        "paint" | "brush" | "pencil" | "eraser" | "dodge" | "burn" | "blur_brush"
        | "sharpen_brush" | "smudge" => paint(img, op),
        "bucket" | "paint_bucket" | "fill_bucket" => bucket(img, op),
        "gradient" | "gradient_fill" => gradient(img, op),
        "fill" => fill(img, op),
        "clear" | "delete_selection" => clear(img),
        "paste" | "place" => paste(img, op),
        "shape" => shape(img, op),
        "transform" | "free_transform" => transform(img, op),
        "replace_color" => replace_color(img, op),
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
    fn fill_replaces_transparent_with_colour() {
        let mut i = img(vec![0, 0, 0, 0], 1, 1);
        apply(&mut i, "fill", &json!({"color": [10, 20, 30, 255]})).unwrap();
        assert_eq!(i.get_raw_pixels(), vec![10, 20, 30, 255]);
    }

    #[test]
    fn clear_empties_the_layer() {
        let mut i = img(vec![1, 2, 3, 255], 1, 1);
        apply(&mut i, "clear", &json!({})).unwrap();
        assert_eq!(i.get_raw_pixels(), vec![0, 0, 0, 0]);
    }

    #[test]
    fn bucket_fills_the_contiguous_region() {
        // 2x1 white row; flood the whole thing.
        let mut i = img(vec![255, 255, 255, 255, 255, 255, 255, 255], 2, 1);
        apply(&mut i, "bucket", &json!({"x": 0, "y": 0, "tolerance": 5, "color": [1, 2, 3, 255]})).unwrap();
        let p = i.get_raw_pixels();
        assert_eq!(&p[0..3], &[1, 2, 3]);
        assert_eq!(&p[4..7], &[1, 2, 3]);
    }

    #[test]
    fn eraser_reduces_alpha() {
        let mut i = img(vec![10, 10, 10, 255], 1, 1);
        apply(
            &mut i,
            "paint",
            &json!({"mode": "eraser", "points": [[0, 0]], "radius": 4, "hardness": 100, "opacity": 100}),
        )
        .unwrap();
        assert!(i.get_raw_pixels()[3] < 20);
    }

    #[test]
    fn rect_shape_fills_its_centre() {
        let mut i = img(vec![0u8; 4 * 100], 10, 10);
        apply(
            &mut i,
            "shape",
            &json!({"kind": "rect", "x": 0, "y": 0, "width": 10, "height": 10, "fill": [255, 0, 0, 255]}),
        )
        .unwrap();
        let p = i.get_raw_pixels();
        // Centre pixel should be fully red.
        let c = (5 * 10 + 5) * 4;
        assert_eq!(&p[c..c + 4], &[255, 0, 0, 255]);
    }

    #[test]
    fn identity_transform_keeps_pixels() {
        let px = vec![9, 8, 7, 255, 6, 5, 4, 255, 3, 2, 1, 255, 12, 13, 14, 255];
        let mut i = img(px.clone(), 2, 2);
        apply(&mut i, "transform", &json!({"matrix": [1, 0, 0, 1, 0, 0]})).unwrap();
        assert_eq!(i.get_raw_pixels(), px);
    }
}

