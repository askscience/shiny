//! Convolution, distortion and render filters.
//!
//! Each filter mutates a `PhotonImage` in place and returns `Ok(false)` for
//! names it does not own, so the dispatcher can try the other engines.

use photon_rs::PhotonImage;
use serde_json::Value;

use crate::pixels::{from_raw, lerp, luminance, raw, sample_bilinear, to_u8};

fn f64_param(op: &Value, key: &str, default: f64) -> f64 {
    op.get(key).and_then(|v| v.as_f64()).unwrap_or(default)
}

fn int_param(op: &Value, key: &str, default: i64) -> i64 {
    op.get(key).and_then(|v| v.as_i64()).unwrap_or(default)
}

/* ── Blur family ────────────────────────────────────────────────── */

fn box_blur(img: &mut PhotonImage, radius: u32) {
    let w = img.get_width();
    let h = img.get_height();
    let mut px = raw(img);
    crate::pixels::box_blur_u8(&mut px, w, h, 4, radius);
    *img = from_raw(px, w, h);
}

fn motion_blur(img: &mut PhotonImage, angle_deg: f64, distance: f64) {
    let w = img.get_width();
    let h = img.get_height();
    let src = raw(img);
    let steps = distance.abs().round().max(1.0) as i64;
    let rad = angle_deg.to_radians();
    let dx = rad.cos();
    let dy = rad.sin();
    let mut out = src.clone();
    for y in 0..h {
        for x in 0..w {
            let mut acc = [0.0f64; 4];
            let mut n = 0.0;
            for i in -steps..=steps {
                let t = i as f64 * 0.5;
                let s = sample_bilinear(&src, w, h, x as f64 + dx * t, y as f64 + dy * t);
                for c in 0..4 {
                    acc[c] += s[c] as f64;
                }
                n += 1.0;
            }
            let o = ((y as usize) * (w as usize) + x as usize) * 4;
            for c in 0..4 {
                out[o + c] = to_u8(acc[c] / n);
            }
        }
    }
    *img = from_raw(out, w, h);
}

fn radial_blur(img: &mut PhotonImage, amount: f64, method: &str) {
    let w = img.get_width();
    let h = img.get_height();
    let src = raw(img);
    let (cx, cy) = (w as f64 / 2.0, h as f64 / 2.0);
    let max_r = (cx * cx + cy * cy).sqrt().max(1.0);
    let samples = amount.abs().round().clamp(2.0, 32.0) as i64;
    let strength = amount / samples.max(1) as f64;
    let spin = method != "zoom";
    let mut out = src.clone();
    for y in 0..h {
        for x in 0..w {
            let mut acc = [0.0f64; 4];
            for i in 0..samples {
                let t = (i as f64 / samples as f64) - 0.5;
                let (sx, sy) = if spin {
                    let ang = t * strength.to_radians();
                    let (sin, cos) = ang.sin_cos();
                    let dx = x as f64 - cx;
                    let dy = y as f64 - cy;
                    (cx + dx * cos - dy * sin, cy + dx * sin + dy * cos)
                } else {
                    let dx = x as f64 - cx;
                    let dy = y as f64 - cy;
                    let scale = 1.0 + t * strength / max_r * 40.0;
                    (cx + dx * scale, cy + dy * scale)
                };
                let s = sample_bilinear(&src, w, h, sx, sy);
                for c in 0..4 {
                    acc[c] += s[c] as f64;
                }
            }
            let o = ((y as usize) * (w as usize) + x as usize) * 4;
            for c in 0..4 {
                out[o + c] = to_u8(acc[c] / samples as f64);
            }
        }
    }
    *img = from_raw(out, w, h);
}

fn gaussian_blur(img: &mut PhotonImage, radius: u32) {
    photon_rs::conv::gaussian_blur(img, radius as i32);
}

fn unsharp_mask(img: &mut PhotonImage, radius: u32, amount: f64, threshold: f64) {
    let w = img.get_width();
    let h = img.get_height();
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

/// Morphological maximum/minimum per RGB channel (dilate / erode).
fn morph(img: &mut PhotonImage, radius: u32, max: bool) {
    let w = img.get_width();
    let h = img.get_height();
    let src = raw(img);
    let r = radius as i64;
    let mut out = src.clone();
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            for c in 0..3 {
                let mut best = if max { 0u8 } else { 255u8 };
                for dy in -r..=r {
                    for dx in -r..=r {
                        let xx = (x + dx).clamp(0, w as i64 - 1) as usize;
                        let yy = (y + dy).clamp(0, h as i64 - 1) as usize;
                        let v = src[(yy * w as usize + xx) * 4 + c];
                        best = if max { best.max(v) } else { best.min(v) };
                    }
                }
                out[((y as usize) * w as usize + x as usize) * 4 + c] = best;
            }
        }
    }
    *img = from_raw(out, w, h);
}

fn median(img: &mut PhotonImage, radius: u32) {
    let w = img.get_width();
    let h = img.get_height();
    let src = raw(img);
    let r = radius.clamp(0, 6) as i64;
    let mut out = src.clone();
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            let mut buckets: [Vec<u8>; 3] = [Vec::new(), Vec::new(), Vec::new()];
            for dy in -r..=r {
                for dx in -r..=r {
                    let xx = (x + dx).clamp(0, w as i64 - 1) as usize;
                    let yy = (y + dy).clamp(0, h as i64 - 1) as usize;
                    let idx = (yy * w as usize + xx) * 4;
                    for c in 0..3 {
                        buckets[c].push(src[idx + c]);
                    }
                }
            }
            let o = ((y as usize) * w as usize + x as usize) * 4;
            for c in 0..3 {
                buckets[c].sort_unstable();
                out[o + c] = buckets[c][buckets[c].len() / 2];
            }
        }
    }
    *img = from_raw(out, w, h);
}

/// High pass: grey 128 + (original − blurred), original alpha kept.
fn high_pass(img: &mut PhotonImage, radius: u32) {
    let w = img.get_width();
    let h = img.get_height();
    let mut blurred = img.clone();
    photon_rs::conv::gaussian_blur(&mut blurred, radius.max(1) as i32);
    let base = raw(img);
    let blur = raw(&blurred);
    let mut out = base.clone();
    let mut i = 0;
    while i + 3 < out.len() {
        for c in 0..3 {
            let v = 128.0 + (base[i + c] as f64 - blur[i + c] as f64);
            out[i + c] = to_u8(v);
        }
        i += 4;
    }
    *img = from_raw(out, w, h);
}

fn offset(img: &mut PhotonImage, dx: i64, dy: i64, wrap: bool) {
    let w = img.get_width() as i64;
    let h = img.get_height() as i64;
    let src = raw(img);
    let mut out = vec![0u8; src.len()];
    for y in 0..h {
        for x in 0..w {
            let nx = x + dx;
            let ny = y + dy;
            if !wrap && (nx < 0 || ny < 0 || nx >= w || ny >= h) {
                continue; // leave transparent
            }
            let sx = nx.rem_euclid(w);
            let sy = ny.rem_euclid(h);
            let si = ((sy * w + sx) as usize) * 4;
            let di = ((y * w + x) as usize) * 4;
            out[di..di + 4].copy_from_slice(&src[si..si + 4]);
        }
    }
    *img = from_raw(out, w as u32, h as u32);
}

fn pixelate(img: &mut PhotonImage, size: u32) {
    let w = img.get_width();
    let h = img.get_height();
    let src = raw(img);
    let s = size.max(1);
    let mut out = src.clone();
    let mut by = 0;
    while by < h {
        let mut bx = 0;
        while bx < w {
            let x1 = (bx + s).min(w);
            let y1 = (by + s).min(h);
            let mut acc = [0u32; 4];
            let mut n = 0u32;
            for y in by..y1 {
                for x in bx..x1 {
                    let i = ((y as usize) * (w as usize) + x as usize) * 4;
                    for c in 0..4 {
                        acc[c] += src[i + c] as u32;
                    }
                    n += 1;
                }
            }
            if n > 0 {
                for y in by..y1 {
                    for x in bx..x1 {
                        let i = ((y as usize) * (w as usize) + x as usize) * 4;
                        for c in 0..4 {
                            out[i + c] = (acc[c] / n) as u8;
                        }
                    }
                }
            }
            bx += s;
        }
        by += s;
    }
    *img = from_raw(out, w, h);
}

/// Crystallize: average each cell but jitter the cell grid so the blocks read
/// as irregular crystals rather than a strict mosaic.
fn crystallize(img: &mut PhotonImage, size: u32) {
    let w = img.get_width();
    let h = img.get_height();
    let src = raw(img);
    let s = size.max(2) as i64;
    let mut out = src.clone();
    // Precompute a jittered centroid per cell.
    for by in (0..h as i64).step_by(s as usize) {
        for bx in (0..w as i64).step_by(s as usize) {
            let jx = ((bx * 2654435761u64 as i64).wrapping_abs() % s) - s / 2;
            let jy = ((by * 40503u64 as i64).wrapping_abs() % s) - s / 2;
            let cxp = (bx + s / 2 + jx).clamp(0, w as i64 - 1) as usize;
            let cyp = (by + s / 2 + jy).clamp(0, h as i64 - 1) as usize;
            let ci = (cyp * w as usize + cxp) * 4;
            let mut acc = [0u32; 4];
            let mut n = 0u32;
            for y in by..(by + s).min(h as i64) {
                for x in bx..(bx + s).min(w as i64) {
                    let i = (y as usize * w as usize + x as usize) * 4;
                    for c in 0..4 {
                        acc[c] += src[i + c] as u32;
                    }
                    n += 1;
                }
            }
            if n == 0 {
                continue;
            }
            let avg = [acc[0] / n, acc[1] / n, acc[2] / n, acc[3] / n];
            for y in by..(by + s).min(h as i64) {
                for x in bx..(bx + s).min(w as i64) {
                    let i = (y as usize * w as usize + x as usize) * 4;
                    // Blend cell average with the centre pixel to keep detail.
                    for c in 0..4 {
                        out[i + c] = to_u8(lerp(src[ci + c] as f64, avg[c] as f64, 0.7));
                    }
                }
            }
        }
    }
    *img = from_raw(out, w, h);
}

/// Fragment: average four half-offset copies of the image.
fn fragment(img: &mut PhotonImage, dx: i64, dy: i64) {
    let w = img.get_width();
    let h = img.get_height();
    let src = raw(img);
    let mut out = src.clone();
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            let mut acc = [0.0f64; 4];
            let offs = [(0i64, 0i64), (dx, 0), (0, dy), (dx, dy)];
            for (ox, oy) in offs {
                let s = sample_bilinear(
                    &src,
                    w,
                    h,
                    (x + ox) as f64,
                    (y + oy) as f64,
                );
                for c in 0..4 {
                    acc[c] += s[c] as f64;
                }
            }
            let o = (y as usize * w as usize + x as usize) * 4;
            for c in 0..4 {
                out[o + c] = to_u8(acc[c] / 4.0);
            }
        }
    }
    *img = from_raw(out, w, h);
}

fn despeckle(img: &mut PhotonImage, radius: u32) {
    median(img, radius.max(1));
}

/* ── Distort family ─────────────────────────────────────────────── */

/// Remap every pixel through `f(x, y) -> (sx, sy)` with bilinear sampling.
fn remap<F: Fn(f64, f64) -> (f64, f64)>(img: &mut PhotonImage, f: F) {
    let w = img.get_width();
    let h = img.get_height();
    let src = raw(img);
    let mut out = src.clone();
    for y in 0..h {
        for x in 0..w {
            let (sx, sy) = f(x as f64, y as f64);
            let s = sample_bilinear(&src, w, h, sx, sy);
            let o = ((y as usize) * (w as usize) + x as usize) * 4;
            out[o..o + 4].copy_from_slice(&s);
        }
    }
    *img = from_raw(out, w, h);
}

fn twirl(img: &mut PhotonImage, angle_deg: f64) {
    let w = img.get_width() as f64;
    let h = img.get_height() as f64;
    let (cx, cy) = (w / 2.0, h / 2.0);
    let rmax = (cx.min(cy)).max(1.0);
    // Convert the output pixel back to input: rotate by -theta(r).
    remap(img, |x, y| {
        let dx = x - cx;
        let dy = y - cy;
        let r = (dx * dx + dy * dy).sqrt();
        let t = (1.0 - (r / rmax)).clamp(0.0, 1.0);
        let theta = angle_deg.to_radians() * t;
        let (sin, cos) = theta.sin_cos();
        (cx + dx * cos - dy * sin, cy + dx * sin + dy * cos)
    });
}

fn ripple(img: &mut PhotonImage, amount: f64, size: f64) {
    let size = size.max(1.0);
    let w = img.get_width() as f64;
    let h = img.get_height() as f64;
    let (cx, cy) = (w / 2.0, h / 2.0);
    remap(img, |x, y| {
        let dx = x - cx;
        let dy = y - cy;
        let r = (dx * dx + dy * dy).sqrt();
        let shift = amount * (r / size).sin();
        if r < 1e-6 {
            (x, y)
        } else {
            (x + dx / r * shift, y + dy / r * shift)
        }
    });
}

fn wave(img: &mut PhotonImage, amplitude: f64, wavelength: f64, phase_deg: f64) {
    let wl = wavelength.max(1.0);
    let phase = phase_deg.to_radians();
    remap(img, |x, y| {
        let sx = x + amplitude * (std::f64::consts::TAU * y / wl + phase).sin();
        let sy = y + amplitude * (std::f64::consts::TAU * x / wl + phase).cos();
        (sx, sy)
    });
}

fn pinch(img: &mut PhotonImage, amount: f64) {
    let w = img.get_width() as f64;
    let h = img.get_height() as f64;
    let (cx, cy) = (w / 2.0, h / 2.0);
    let rmax = cx.min(cy).max(1.0);
    let k = (amount / 100.0).clamp(-0.99, 0.99);
    remap(img, |x, y| {
        let dx = x - cx;
        let dy = y - cy;
        let r = (dx * dx + dy * dy).sqrt();
        if r < 1e-6 || r > rmax {
            return (x, y);
        }
        let t = r / rmax;
        let factor = (t.powf(1.0 + k)).max(1e-6) / t.max(1e-6);
        (cx + dx * factor, cy + dy * factor)
    });
}

fn spherize(img: &mut PhotonImage, amount: f64) {
    let w = img.get_width() as f64;
    let h = img.get_height() as f64;
    let (cx, cy) = (w / 2.0, h / 2.0);
    let rmax = cx.min(cy).max(1.0);
    let k = (amount / 100.0).clamp(-1.0, 1.0);
    remap(img, |x, y| {
        let dx = (x - cx) / rmax;
        let dy = (y - cy) / rmax;
        let r = (dx * dx + dy * dy).sqrt();
        if r >= 1.0 || r < 1e-6 {
            return (x, y);
        }
        let t = r.acos() / std::f64::consts::FRAC_PI_2;
        let factor = (t * (1.0 - k) + r * k) / r;
        (cx + dx * rmax * factor, cy + dy * rmax * factor)
    });
}

fn polar_coordinates(img: &mut PhotonImage, to_polar: bool) {
    let w = img.get_width() as f64;
    let h = img.get_height() as f64;
    let (cx, cy) = (w / 2.0, h / 2.0);
    let rmax = cx.min(cy).max(1.0);
    remap(img, |x, y| {
        if to_polar {
            // Rectangular input → polar output.
            let dx = x - cx;
            let dy = y - cy;
            let r = (dx * dx + dy * dy).sqrt();
            let theta = dy.atan2(dx);
            let sx = theta / std::f64::consts::TAU * w + cx;
            let sy = r / rmax * h;
            (sx, sy)
        } else {
            let theta = (x - cx) / w * std::f64::consts::TAU;
            let r = (y / h) * rmax;
            (cx + r * theta.cos(), cy + r * theta.sin())
        }
    });
}

/* ── Stylize / render ───────────────────────────────────────────── */

fn find_edges(img: &mut PhotonImage) {
    let w = img.get_width();
    let h = img.get_height();
    let src = raw(img);
    let lum = |x: i64, y: i64| -> f64 {
        let xx = x.clamp(0, w as i64 - 1) as usize;
        let yy = y.clamp(0, h as i64 - 1) as usize;
        let i = (yy * w as usize + xx) * 4;
        luminance(src[i] as f64, src[i + 1] as f64, src[i + 2] as f64)
    };
    let mut out = src.clone();
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            let gx = -lum(x - 1, y - 1) - 2.0 * lum(x - 1, y) - lum(x - 1, y + 1)
                + lum(x + 1, y - 1) + 2.0 * lum(x + 1, y) + lum(x + 1, y + 1);
            let gy = -lum(x - 1, y - 1) - 2.0 * lum(x, y - 1) - lum(x + 1, y - 1)
                + lum(x - 1, y + 1) + 2.0 * lum(x, y + 1) + lum(x + 1, y + 1);
            let mag = (gx * gx + gy * gy).sqrt();
            // Edges are dark on white, like a pencil sketch.
            let v = to_u8(255.0 - mag);
            let o = (y as usize * w as usize + x as usize) * 4;
            out[o] = v;
            out[o + 1] = v;
            out[o + 2] = v;
        }
    }
    *img = from_raw(out, w, h);
}

fn glowing_edges(img: &mut PhotonImage, color: [u8; 3], intensity: f64) {
    let w = img.get_width();
    let h = img.get_height();
    find_edges(img);
    let edge = raw(img);
    let mut out = edge.clone();
    let mut i = 0;
    while i + 3 < out.len() {
        // Re-map the sketch (dark lines) to bright colour on dark background.
        let e = (255.0 - edge[i] as f64) / 255.0;
        let e = (e * intensity / 50.0).clamp(0.0, 1.0);
        out[i] = to_u8(color[0] as f64 * e);
        out[i + 1] = to_u8(color[1] as f64 * e);
        out[i + 2] = to_u8(color[2] as f64 * e);
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

/// Fractal value noise clouds, blended with the given colour.
fn clouds(img: &mut PhotonImage, scale: f64, seed: u32, color: [f64; 3]) {
    let w = img.get_width();
    let h = img.get_height();
    let mut out = raw(img);
    let seed = seed as f64;
    let hash = |x: f64, y: f64| -> f64 {
        let n = (x * 374761393.0 + y * 668265263.0 + seed * 1442695040888963407.0).sin() * 43758.5453;
        n - n.floor()
    };
    let value = |x: f64, y: f64| -> f64 {
        let xi = x.floor();
        let yi = y.floor();
        let xf = x - xi;
        let yf = y - yi;
        let u = xf * xf * (3.0 - 2.0 * xf);
        let v = yf * yf * (3.0 - 2.0 * yf);
        let a = hash(xi, yi);
        let b = hash(xi + 1.0, yi);
        let c = hash(xi, yi + 1.0);
        let d = hash(xi + 1.0, yi + 1.0);
        lerp(lerp(a, b, u), lerp(c, d, u), v)
    };
    for y in 0..h {
        for x in 0..w {
            let mut n = 0.0;
            let mut amp = 0.5;
            let mut freq = 1.0 / scale.max(1.0);
            for _ in 0..5 {
                n += value(x as f64 * freq, y as f64 * freq) * amp;
                amp *= 0.5;
                freq *= 2.0;
            }
            let i = ((y as usize) * (w as usize) + x as usize) * 4;
            for c in 0..3 {
                let base = out[i + c] as f64;
                let cloud = color[c] * (n.clamp(0.0, 1.0) * 255.0);
                out[i + c] = to_u8(lerp(base, cloud, 1.0));
            }
            out[i + 3] = 255;
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
        "gaussian_blur" | "gaussian" => gaussian_blur(img, radius(op, 2).max(1)),
        "box_blur" => box_blur(img, radius(op, 2).max(1)),
        "motion_blur" => motion_blur(
            img,
            f64_param(op, "angle", 0.0),
            f64_param(op, "distance", 10.0),
        ),
        "radial_blur" => radial_blur(
            img,
            f64_param(op, "amount", 10.0),
            op.get("method").and_then(|v| v.as_str()).unwrap_or("spin"),
        ),
        "zoom_blur" => radial_blur(img, f64_param(op, "amount", 10.0), "zoom"),
        "unsharp_mask" => unsharp_mask(
            img,
            radius(op, 1).max(1),
            f64_param(op, "amount", 1.0),
            f64_param(op, "threshold", 0.0),
        ),
        "sharpen_more" => unsharp_mask(img, 2, 1.5, 0.0),
        "median" => median(img, radius(op, 1)),
        "despeckle" => despeckle(img, radius(op, 1)),
        "high_pass" => high_pass(img, radius(op, 10).max(1)),
        "maximum" => morph(img, radius(op, 2), true),
        "minimum" => morph(img, radius(op, 2), false),
        "offset" => offset(
            img,
            int_param(op, "x", 0),
            int_param(op, "y", 0),
            op.get("wrap").and_then(|v| v.as_bool()).unwrap_or(true),
        ),
        "pixelate" | "mosaic" => pixelate(img, int_param(op, "size", 8).clamp(1, 512) as u32),
        "crystallize" => crystallize(img, int_param(op, "size", 10).clamp(2, 512) as u32),
        "fragment" => fragment(
            img,
            int_param(op, "x", 4),
            int_param(op, "y", 4),
        ),
        "twirl" => twirl(img, f64_param(op, "angle", 50.0)),
        "ripple" => ripple(img, f64_param(op, "amount", 10.0), f64_param(op, "size", 40.0)),
        "wave" => wave(
            img,
            f64_param(op, "amplitude", 10.0),
            f64_param(op, "wavelength", 40.0),
            f64_param(op, "phase", 0.0),
        ),
        "pinch" => pinch(img, f64_param(op, "amount", 50.0)),
        "spherize" => spherize(img, f64_param(op, "amount", 50.0)),
        "polar_coordinates" => polar_coordinates(
            img,
            op.get("type").and_then(|v| v.as_str()).unwrap_or("polar") == "polar",
        ),
        "find_edges" => find_edges(img),
        "glowing_edges" => glowing_edges(
            img,
            [
                int_param(op, "r", 0).clamp(0, 255) as u8,
                int_param(op, "g", 255).clamp(0, 255) as u8,
                int_param(op, "b", 0).clamp(0, 255) as u8,
            ],
            f64_param(op, "intensity", 50.0),
        ),
        "vignette" => vignette(
            img,
            f64_param(op, "amount", 50.0),
            f64_param(op, "size", 30.0),
            f64_param(op, "roundness", 0.0),
        ),
        "clouds" => clouds(
            img,
            f64_param(op, "scale", 64.0),
            int_param(op, "seed", 1) as u32,
            [
                f64_param(op, "r", 255.0),
                f64_param(op, "g", 255.0),
                f64_param(op, "b", 255.0),
            ],
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
    fn box_blur_of_uniform_is_identity() {
        let px = vec![50u8; 4 * 16];
        let mut i = img(px.clone(), 4, 4);
        apply(&mut i, "box_blur", &json!({"radius": 2})).unwrap();
        assert_eq!(i.get_raw_pixels(), px);
    }

    #[test]
    fn pixelate_makes_blocks_uniform() {
        // 2x2 with distinct values; pixelate size 2 averages them.
        let px = vec![0, 0, 0, 255, 255, 255, 255, 255, 0, 0, 0, 255, 255, 255, 255, 255];
        let mut i = img(px, 2, 2);
        apply(&mut i, "pixelate", &json!({"size": 2})).unwrap();
        let p = i.get_raw_pixels();
        assert_eq!(&p[0..3], &p[8..11]);
    }

    #[test]
    fn offset_wraps() {
        let mut i = img(vec![1, 1, 1, 255, 2, 2, 2, 255], 2, 1);
        apply(&mut i, "offset", &json!({"x": 1, "y": 0, "wrap": true})).unwrap();
        let p = i.get_raw_pixels();
        assert_eq!(p[0], 2);
        assert_eq!(p[4], 1);
    }

    #[test]
    fn unknown_filter_is_not_handled() {
        let mut i = img(vec![0, 0, 0, 255], 1, 1);
        assert_eq!(apply(&mut i, "definitely_not_a_filter", &json!({})).unwrap(), false);
    }
}

