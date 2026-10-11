//! Curated looks — one-tap effects applied to the whole document.
//!
//! Each look is a short recipe built from the shared engine (tone/colour
//! primitives) plus photon-rs building blocks (duotone, halftone, oil,
//! pixelize) so the result reads intentional rather than a stock preset. The
//! window shows a preview strip of every look applied to the open image
//! (`POST /api/images/:id/looks`) and applies one with `{ "op": "look" }`.

use photon_rs::colour_spaces::mix_with_colour;
use photon_rs::effects::{duotone, halftone, oil, pixelize};
use photon_rs::filters::duotone_violette;
use photon_rs::monochrome;
use photon_rs::PhotonImage;
use photon_rs::Rgb;
use serde_json::json;

use crate::pixels::{clamp, hsl_to_rgb, luminance, map_pixels, rgb_to_hsl, to_u8};

/// Edge length (px) of one look thumbnail in the preview strip.
pub const TILE: u32 = 96;

/// One look: `key` is the wire name, `name` the human label (used in tooltips
/// and the skills doc only — the window shows thumbnails).
pub struct Look {
    pub key: &'static str,
    pub name: &'static str,
    pub apply: fn(&mut PhotonImage),
}

pub const LOOKS: &[Look] = &[
    Look { key: "portra", name: "Portra", apply: look_portra },
    Look { key: "faded", name: "Faded", apply: look_faded },
    Look { key: "noir", name: "Noir", apply: look_noir },
    Look { key: "silver", name: "Silver", apply: look_silver },
    Look { key: "sepia", name: "Sepia", apply: look_sepia },
    Look { key: "kodachrome", name: "Kodachrome", apply: look_kodachrome },
    Look { key: "cinestill", name: "Cinestill", apply: look_cinestill },
    Look { key: "golden", name: "Golden hour", apply: look_golden },
    Look { key: "blue_hour", name: "Blue hour", apply: look_blue_hour },
    Look { key: "neon", name: "Neon", apply: look_neon },
    Look { key: "rose", name: "Rosé", apply: look_rose },
    Look { key: "dream", name: "Dream", apply: look_dream },
    Look { key: "oil", name: "Oil paint", apply: look_oil },
    Look { key: "halftone", name: "Halftone", apply: look_halftone },
    Look { key: "pixel", name: "Pixel", apply: look_pixel },
    Look { key: "violette", name: "Violette", apply: look_violette },
];

/// Find a look by key (case-insensitive, trimmed).
pub fn find(name: &str) -> Option<&'static Look> {
    let needle = name.trim().to_ascii_lowercase();
    LOOKS.iter().find(|l| l.key == needle)
}

/// Apply a look by key. Unknown names list the valid ones.
pub fn apply(img: &mut PhotonImage, name: &str) -> Result<(), String> {
    let Some(look) = find(name) else {
        let keys: Vec<&str> = LOOKS.iter().map(|l| l.key).collect();
        return Err(format!(
            "unknown look \"{name}\" — choose one of: {}",
            keys.join(", ")
        ));
    };
    (look.apply)(img);
    Ok(())
}

/* ── Building blocks ────────────────────────────────────────────── */

/// Remap `in_black..in_white` onto `out_black..out_white` with a gamma.
fn levels(img: &mut PhotonImage, in_black: f64, in_white: f64, gamma: f64, out_black: f64, out_white: f64) {
    let span = (in_white - in_black).max(1.0);
    let inv = 1.0 / gamma.max(0.01);
    map_pixels(img, |r, g, b, a| {
        let m = |v: u8| {
            let t = clamp((v as f64 - in_black) / span, 0.0, 1.0).powf(inv);
            to_u8(out_black + t * (out_white - out_black))
        };
        (m(r), m(g), m(b), a)
    });
}

/// Film-style contrast: blend toward an S-curve (`smoothstep`).
fn filmic(img: &mut PhotonImage, amount: f64) {
    map_pixels(img, |r, g, b, a| {
        let m = |v: u8| {
            let t = v as f64 / 255.0;
            let s = t * t * (3.0 - 2.0 * t);
            to_u8((t + (s - t) * amount) * 255.0)
        };
        (m(r), m(g), m(b), a)
    });
}

/// Scale HSL saturation by `1 + amount`.
fn saturate(img: &mut PhotonImage, amount: f64) {
    map_pixels(img, |r, g, b, a| {
        let (h, s, l) = rgb_to_hsl(r as f64, g as f64, b as f64);
        let s = clamp(s * (1.0 + amount), 0.0, 1.0);
        let (nr, ng, nb) = hsl_to_rgb(h, s, l);
        (to_u8(nr), to_u8(ng), to_u8(nb), a)
    });
}

/// Push shadows toward `shadow` and highlights toward `highlight`
/// (colour values around 128 are neutral).
fn split_tone(img: &mut PhotonImage, shadow: [f64; 3], highlight: [f64; 3], amount: f64) {
    map_pixels(img, |r, g, b, a| {
        let l = luminance(r as f64, g as f64, b as f64) / 255.0;
        let ws = (1.0 - l) * (1.0 - l);
        let wh = l * l;
        let ch = [r as f64, g as f64, b as f64];
        let out = [
            ch[0] + amount * ((shadow[0] - 128.0) * ws + (highlight[0] - 128.0) * wh),
            ch[1] + amount * ((shadow[1] - 128.0) * ws + (highlight[1] - 128.0) * wh),
            ch[2] + amount * ((shadow[2] - 128.0) * ws + (highlight[2] - 128.0) * wh),
        ];
        (to_u8(out[0]), to_u8(out[1]), to_u8(out[2]), a)
    });
}

/// Screen-blend a blurred copy back over the image (soft glow / bloom).
fn soft_glow(img: &mut PhotonImage, radius: u32, mix: f64) {
    let mut blurred = img.clone();
    if !crate::filter::apply(&mut blurred, "blur", &json!({ "radius": radius })).unwrap_or(false) {
        return;
    }
    let base = img.get_raw_pixels();
    let soft = blurred.get_raw_pixels();
    let mut out = base.clone();
    let mut i = 0;
    while i + 3 < out.len() {
        for c in 0..3 {
            let a = base[i + c] as f64 / 255.0;
            let b = soft[i + c] as f64 / 255.0;
            let screen = 1.0 - (1.0 - a) * (1.0 - b);
            out[i + c] = to_u8((a * (1.0 - mix) + screen * mix) * 255.0);
        }
        i += 4;
    }
    *img = PhotonImage::new(out, img.get_width(), img.get_height());
}

/// Deterministic, luminance-neutral film grain (± `amount` levels).
fn grain(img: &mut PhotonImage, amount: f64) {
    let mut px = img.get_raw_pixels();
    let mut noise = 0x9e3779b9u32;
    let mut i = 0;
    while i + 3 < px.len() {
        noise = noise.wrapping_mul(1664525).wrapping_add(1013904223);
        let n = ((noise >> 16) & 0xff) as f64 / 255.0 - 0.5;
        let d = n * amount * 2.0;
        for c in 0..3 {
            px[i + c] = to_u8(px[i + c] as f64 + d);
        }
        i += 4;
    }
    *img = PhotonImage::new(px, img.get_width(), img.get_height());
}

fn vignette(img: &mut PhotonImage, amount: f64, size: f64) {
    let _ = crate::filter::apply(img, "vignette", &json!({ "amount": amount, "size": size }));
}

/* ── The looks ──────────────────────────────────────────────────── */

fn look_portra(img: &mut PhotonImage) {
    filmic(img, 0.30);
    levels(img, 0.0, 255.0, 1.03, 6.0, 252.0);
    split_tone(img, [110.0, 118.0, 138.0], [255.0, 226.0, 196.0], 0.20);
    saturate(img, 0.06);
    grain(img, 2.5);
}

fn look_faded(img: &mut PhotonImage) {
    levels(img, 10.0, 250.0, 1.06, 26.0, 242.0);
    saturate(img, -0.24);
    split_tone(img, [120.0, 124.0, 130.0], [246.0, 232.0, 210.0], 0.14);
    grain(img, 3.0);
}

fn look_noir(img: &mut PhotonImage) {
    monochrome::grayscale_human_corrected(img);
    filmic(img, 0.50);
    levels(img, 6.0, 249.0, 1.0, 4.0, 249.0);
    vignette(img, 45.0, 32.0);
    grain(img, 3.0);
}

fn look_silver(img: &mut PhotonImage) {
    monochrome::grayscale(img);
    levels(img, 4.0, 252.0, 1.04, 10.0, 246.0);
    filmic(img, 0.22);
    split_tone(img, [112.0, 118.0, 132.0], [255.0, 253.0, 248.0], 0.10);
    grain(img, 1.5);
}

fn look_sepia(img: &mut PhotonImage) {
    monochrome::sepia(img);
    filmic(img, 0.25);
    levels(img, 0.0, 255.0, 1.0, 6.0, 246.0);
    vignette(img, 35.0, 34.0);
    grain(img, 2.5);
}

fn look_kodachrome(img: &mut PhotonImage) {
    levels(img, 8.0, 250.0, 1.0, 0.0, 255.0);
    saturate(img, 0.32);
    filmic(img, 0.35);
    split_tone(img, [104.0, 110.0, 124.0], [255.0, 238.0, 210.0], 0.12);
}

fn look_cinestill(img: &mut PhotonImage) {
    split_tone(img, [88.0, 120.0, 132.0], [255.0, 196.0, 150.0], 0.25);
    filmic(img, 0.28);
    soft_glow(img, 14, 0.16);
    grain(img, 2.5);
}

fn look_golden(img: &mut PhotonImage) {
    mix_with_colour(img, Rgb::new(255, 190, 96), 0.18);
    split_tone(img, [118.0, 108.0, 96.0], [255.0, 214.0, 150.0], 0.15);
    saturate(img, 0.12);
    soft_glow(img, 18, 0.22);
    vignette(img, 22.0, 38.0);
}

fn look_blue_hour(img: &mut PhotonImage) {
    mix_with_colour(img, Rgb::new(66, 106, 170), 0.20);
    saturate(img, -0.14);
    levels(img, 0.0, 255.0, 1.0, 12.0, 248.0);
    split_tone(img, [92.0, 112.0, 148.0], [226.0, 234.0, 246.0], 0.16);
}

fn look_neon(img: &mut PhotonImage) {
    duotone(img, Rgb::new(255, 42, 150), Rgb::new(0, 224, 255));
    filmic(img, 0.30);
    saturate(img, 0.15);
}

fn look_rose(img: &mut PhotonImage) {
    mix_with_colour(img, Rgb::new(255, 176, 198), 0.22);
    levels(img, 0.0, 255.0, 1.0, 14.0, 250.0);
    filmic(img, 0.12);
    saturate(img, -0.04);
    grain(img, 1.5);
}

fn look_dream(img: &mut PhotonImage) {
    soft_glow(img, 22, 0.38);
    levels(img, 0.0, 255.0, 1.06, 12.0, 250.0);
    saturate(img, -0.12);
}

fn look_oil(img: &mut PhotonImage) {
    oil(img, 4, 55.0);
    filmic(img, 0.18);
    saturate(img, 0.10);
}

fn look_halftone(img: &mut PhotonImage) {
    halftone(img);
    filmic(img, 0.20);
}

fn look_pixel(img: &mut PhotonImage) {
    pixelize(img, 8);
}

fn look_violette(img: &mut PhotonImage) {
    duotone_violette(img);
    filmic(img, 0.15);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(w: u32, h: u32) -> PhotonImage {
        let mut px = vec![0u8; (w * h * 4) as usize];
        for (i, p) in px.chunks_mut(4).enumerate() {
            let x = (i as u32 % w) as u8;
            let y = (i as u32 / w) as u8;
            p[0] = x.wrapping_mul(3);
            p[1] = y.wrapping_mul(3);
            p[2] = 128;
            p[3] = 255;
        }
        PhotonImage::new(px, w, h)
    }

    #[test]
    fn every_look_runs_and_keeps_dimensions() {
        for look in LOOKS {
            let mut i = img(8, 8);
            (look.apply)(&mut i);
            assert_eq!((i.get_width(), i.get_height()), (8, 8), "look {}", look.key);
            assert_eq!(i.get_raw_pixels().len(), 8 * 8 * 4, "look {}", look.key);
        }
    }

    #[test]
    fn unknown_look_lists_the_keys() {
        let mut i = img(4, 4);
        let err = apply(&mut i, "nope").unwrap_err();
        assert!(err.contains("portra"), "{err}");
    }

    #[test]
    fn look_keys_are_unique() {
        let mut keys: Vec<&str> = LOOKS.iter().map(|l| l.key).collect();
        keys.sort_unstable();
        let before = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), before);
    }
}
