//! The webcam bubble, composited into the NV12 frame.
//!
//! Per camera frame: one bilinear scale of the centred crop (mirror and zoom
//! come free from the scaler's taps), then the border ring, then the masked
//! blend. Everything that depends only on the shape (mask, ring weights,
//! scaler taps) is computed once per style and reused.
//!
//! The border is exact compositing, not a second pass: with `outer` the
//! bubble's coverage and `inner` the camera's, the colour under a pixel is
//! `(border·(outer − inner) + camera·inner) / outer`, so the bubble's source
//! pixel is `mix(camera, border, (outer − inner)/outer)` and the final blend
//! uses `outer` alone.

use crate::cursor::Patch;
use crate::mask::{blend, mix, Mask};
use crate::nv12::Nv12;
use crate::scale::{cover_crop, Scaler};

#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(default)]
pub struct Style {
    /// Size in output pixels (rounded to even).
    pub w: u32,
    pub h: u32,
    /// 0 square corners … 1 circle (or pill).
    pub roundness: f32,
    pub mirror: bool,
    /// Border width in output pixels (0 = none).
    pub border: u32,
    /// Border colour, `#rrggbb`.
    pub border_color: String,
    /// 1 = whole frame, up to 4 = tight crop on the centre.
    pub zoom: f32,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            w: 320,
            h: 320,
            roundness: 1.0,
            mirror: true,
            border: 0,
            border_color: "#ffffff".into(),
            zoom: 1.0,
        }
    }
}

/// `#rrggbb` → BT.709 limited-range YUV.
pub fn hex_to_yuv(hex: &str) -> [u8; 3] {
    let h = hex.trim_start_matches('#');
    let p =
        |i: usize| u8::from_str_radix(h.get(i..i + 2).unwrap_or("ff"), 16).unwrap_or(255) as f32;
    let (r, g, b) = (p(0), p(2), p(4));
    let y = 16.0 + 0.8588 * (0.2126 * r + 0.7152 * g + 0.0722 * b);
    let u = 128.0 + 0.8784 * (-0.1146 * r - 0.3854 * g + 0.5 * b);
    let v = 128.0 + 0.8784 * (0.5 * r - 0.4542 * g - 0.0458 * b);
    [y.round() as u8, u.round() as u8, v.round() as u8]
}

/// `(index, weight)` of every pixel where the border shows.
fn ring(outer: &[u8], inner: &[u8], w: usize, iw: usize, b: usize) -> Vec<(u32, u8)> {
    let h = outer.len() / w;
    let ih = inner.len() / iw.max(1);
    let mut out = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let o = outer[y * w + x] as u32;
            if o == 0 {
                continue;
            }
            let i = if x >= b && y >= b && x - b < iw && y - b < ih {
                inner[(y - b) * iw + (x - b)] as u32
            } else {
                0
            };
            let wgt = ((o.saturating_sub(i)) * 255 + o / 2) / o;
            if wgt > 0 {
                out.push(((y * w + x) as u32, wgt.min(255) as u8));
            }
        }
    }
    out
}

pub struct Bubble {
    pub style: Style,
    mask: Mask,
    ring_y: Vec<(u32, u8)>,
    ring_uv: Vec<(u32, u8)>,
    border: [u8; 3],
    scaler: Option<(usize, usize, Scaler)>,
    img: Nv12,
    /// The bubble has a camera picture to show.
    pub ready: bool,
}

impl Bubble {
    pub fn new(style: Style) -> Self {
        let w = (style.w.clamp(32, 1920) & !1) as usize;
        let h = (style.h.clamp(32, 1920) & !1) as usize;
        let mask = Mask::rounded(w, h, style.roundness);
        let b = (style.border as usize).min(w.min(h) / 4) & !1;
        let (ring_y, ring_uv) = if b > 0 {
            // Inner shape: concentric, its radius smaller by the border.
            let r = style.roundness.clamp(0.0, 1.0) * (w.min(h) as f32 / 2.0);
            let (iw, ih) = (w - 2 * b, h - 2 * b);
            let ir = (r - b as f32).max(0.0);
            let inner = Mask::rounded(iw, ih, ir / (iw.min(ih) as f32 / 2.0));
            (
                ring(&mask.luma.alpha, &inner.luma.alpha, w, iw, b),
                ring(
                    &mask.chroma.alpha,
                    &inner.chroma.alpha,
                    w / 2,
                    iw / 2,
                    b / 2,
                ),
            )
        } else {
            (Vec::new(), Vec::new())
        };
        Self {
            border: hex_to_yuv(&style.border_color),
            style,
            mask,
            ring_y,
            ring_uv,
            scaler: None,
            img: Nv12::new(w, h),
            ready: false,
        }
    }

    pub fn size(&self) -> (usize, usize) {
        (self.img.w, self.img.h)
    }

    /// Take a new camera frame.
    pub fn update(&mut self, cam: &Nv12) {
        let (w, h) = (self.img.w, self.img.h);
        if !matches!(&self.scaler, Some((cw, ch, _)) if (*cw, *ch) == (cam.w, cam.h)) {
            let crop = cover_crop(cam.w, cam.h, w, h, self.style.zoom);
            self.scaler = Some((
                cam.w,
                cam.h,
                Scaler::window(cam.w, cam.h, crop, w, h, self.style.mirror),
            ));
        }
        if let Some((_, _, s)) = &mut self.scaler {
            s.run(cam, &mut self.img);
        }
        let [by, bu, bv] = self.border;
        let (yp, uvp) = self.img.planes_mut();
        for &(i, a) in &self.ring_y {
            let p = &mut yp[i as usize];
            *p = mix(*p, by, a);
        }
        for &(i, a) in &self.ring_uv {
            let i = 2 * i as usize;
            uvp[i] = mix(uvp[i], bu, a);
            uvp[i + 1] = mix(uvp[i + 1], bv, a);
        }
        self.ready = true;
    }

    /// Composite at frame pixel `(x, y)` (top-left), saving what it covers.
    pub fn draw(&self, f: &mut Nv12, x: i32, y: i32, patch: &mut Patch) {
        if !self.ready {
            return;
        }
        let (w, h) = (self.img.w as i32, self.img.h as i32);
        let (x, y) = (x & !1, y & !1);
        let x0 = x.max(0) as usize;
        let y0 = y.max(0) as usize;
        let x1 = ((x + w).min(f.w as i32)).max(0) as usize;
        let y1 = ((y + h).min(f.h as i32)).max(0) as usize;
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        patch.save(f, x0, y0, (x1 - x0) & !1, (y1 - y0) & !1);
        blend(f, &self.img, &self.mask, x as isize, y as isize);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn white_is_white() {
        assert_eq!(hex_to_yuv("#ffffff"), [235, 128, 128]);
        assert_eq!(hex_to_yuv("#000000"), [16, 128, 128]);
    }

    #[test]
    fn border_shows_only_at_the_edge() {
        let mut b = Bubble::new(Style {
            w: 64,
            h: 64,
            border: 4,
            border_color: "#ffffff".into(),
            ..Style::default()
        });
        let mut cam = Nv12::new(64, 64);
        cam.data.fill(16);
        let (_, uv) = cam.planes_mut();
        uv.fill(128);
        b.update(&cam);
        let (y, _) = b.img.planes();
        assert_eq!(y[32 * 64 + 32], 16, "centre is camera");
        assert!(y[32 * 64 + 1] > 200, "left edge is border");
    }

    #[test]
    fn draw_then_restore_is_lossless() {
        let mut b = Bubble::new(Style {
            w: 40,
            h: 40,
            ..Style::default()
        });
        let mut cam = Nv12::new(80, 60);
        cam.data.fill(200);
        b.update(&cam);
        let mut f = Nv12::new(100, 80);
        for (i, p) in f.data.iter_mut().enumerate() {
            *p = (i % 251) as u8;
        }
        let before = f.data.clone();
        let mut patch = Patch::default();
        b.draw(&mut f, 70, 50, &mut patch); // clipped at the corner
        assert_ne!(f.data, before);
        patch.restore(&mut f);
        assert_eq!(f.data, before);
    }
}
