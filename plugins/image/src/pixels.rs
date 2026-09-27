//! Small pixel helpers shared by the adjustment, filter and paint engines.
//!
//! Everything works on **straight (non-premultiplied) RGBA** `u8` buffers, the
//! same layout `PhotonImage` exposes, so a result can be handed back to the
//! compositor without a codec round-trip.

use photon_rs::PhotonImage;

/// Clamp + round a float into the `u8` range (rounding avoids the downward
/// bias a plain `as u8` truncation would introduce).
#[inline]
pub fn to_u8(v: f64) -> u8 {
    (v.max(0.0).min(255.0).round()) as u8
}

#[inline]
pub fn clamp01(v: f32) -> f32 {
    if v < 0.0 {
        0.0
    } else if v > 1.0 {
        1.0
    } else {
        v
    }
}

#[inline]
pub fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}

#[inline]
pub fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// Rec. 709 luminance of linear-ish sRGB values in 0..=255.
#[inline]
pub fn luminance(r: f64, g: f64, b: f64) -> f64 {
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

/// sRGB (0..=255) → HSL (`h` 0..360, `s`/`l` 0..1).
pub fn rgb_to_hsl(r: f64, g: f64, b: f64) -> (f64, f64, f64) {
    let (r, g, b) = (r / 255.0, g / 255.0, b / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    if d.abs() < 1e-9 {
        return (0.0, 0.0, l);
    }
    let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
    let h = if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h * 60.0, s, l)
}

/// HSL → sRGB (0..=255).
pub fn hsl_to_rgb(h: f64, s: f64, l: f64) -> (f64, f64, f64) {
    let h = h.rem_euclid(360.0) / 360.0;
    let (s, l) = (clamp(s, 0.0, 1.0), clamp(l, 0.0, 1.0));
    if s <= 1e-9 {
        let v = l * 255.0;
        return (v, v, v);
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let f = |mut t: f64| {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        let v = if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 1.0 / 2.0 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        };
        v * 255.0
    };
    (f(h + 1.0 / 3.0), f(h), f(h - 1.0 / 3.0))
}

/// Raw RGBA pixels of an image.
#[inline]
pub fn raw(img: &PhotonImage) -> Vec<u8> {
    img.get_raw_pixels()
}

/// Build a `PhotonImage` from raw RGBA.
#[inline]
pub fn from_raw(px: Vec<u8>, w: u32, h: u32) -> PhotonImage {
    PhotonImage::new(px, w, h)
}

/// Bilinear RGBA sample; out-of-range reads clamp to the edge.
pub fn sample_bilinear(px: &[u8], w: u32, h: u32, x: f64, y: f64) -> [u8; 4] {
    if w == 0 || h == 0 {
        return [0, 0, 0, 0];
    }
    let x = clamp(x, 0.0, (w - 1) as f64);
    let y = clamp(y, 0.0, (h - 1) as f64);
    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    let x1 = (x0 + 1).min(w - 1);
    let y1 = (y0 + 1).min(h - 1);
    let fx = x - x0 as f64;
    let fy = y - y0 as f64;
    let idx = |xx: u32, yy: u32| ((yy as usize) * (w as usize) + xx as usize) * 4;
    let mut out = [0u8; 4];
    for c in 0..4 {
        let a = px[idx(x0, y0) + c] as f64;
        let b = px[idx(x1, y0) + c] as f64;
        let cc = px[idx(x0, y1) + c] as f64;
        let d = px[idx(x1, y1) + c] as f64;
        let top = lerp(a, b, fx);
        let bottom = lerp(cc, d, fx);
        out[c] = to_u8(lerp(top, bottom, fy));
    }
    out
}

/// Run `f(r, g, b, a)` per pixel and write the returned RGBA back.
pub fn map_pixels<F: FnMut(u8, u8, u8, u8) -> (u8, u8, u8, u8)>(img: &mut PhotonImage, mut f: F) {
    let w = img.get_width();
    let h = img.get_height();
    let mut px = img.get_raw_pixels();
    let mut i = 0;
    while i + 3 < px.len() {
        let (r, g, b, a) = (px[i], px[i + 1], px[i + 2], px[i + 3]);
        let (nr, ng, nb, na) = f(r, g, b, a);
        px[i] = nr;
        px[i + 1] = ng;
        px[i + 2] = nb;
        px[i + 3] = na;
        i += 4;
    }
    *img = PhotonImage::new(px, w, h);
}

/// A separable box blur over an arbitrary stride-1 `u8` buffer with `channels`
/// interleaved components. Used for masks (1 channel) and feathering.
pub fn box_blur_u8(buf: &mut [u8], w: u32, h: u32, channels: usize, radius: u32) {
    if radius == 0 || w == 0 || h == 0 {
        return;
    }
    let (w, h, r) = (w as usize, h as usize, radius as usize);
    let mut tmp = buf.to_vec();
    // Horizontal.
    for y in 0..h {
        for x in 0..w {
            let lo = x.saturating_sub(r);
            let hi = (x + r).min(w - 1);
            let n = (hi - lo + 1) as u32;
            for c in 0..channels {
                let mut sum = 0u32;
                for xx in lo..=hi {
                    sum += buf[(y * w + xx) * channels + c] as u32;
                }
                tmp[(y * w + x) * channels + c] = (sum / n) as u8;
            }
        }
    }
    // Vertical.
    for y in 0..h {
        let lo = y.saturating_sub(r);
        let hi = (y + r).min(h - 1);
        let n = (hi - lo + 1) as u32;
        for x in 0..w {
            for c in 0..channels {
                let mut sum = 0u32;
                for yy in lo..=hi {
                    sum += tmp[(yy * w + x) * channels + c] as u32;
                }
                buf[(y * w + x) * channels + c] = (sum / n) as u8;
            }
        }
    }
}
