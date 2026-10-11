//! Small pixel helpers shared by the adjustment and filter engines.
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

/// Rec. 709 luminance of sRGB values in 0..=255.
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
