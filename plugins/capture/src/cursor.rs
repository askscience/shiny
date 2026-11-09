//! The mouse pointer, drawn into the NV12 frame without dirtying it.
//!
//! The screen grab does not include the hardware cursor. The engine keeps a
//! clean frame (only damaged areas are reconverted), so drawing the cursor
//! into it would leave trails. Instead the pixels under the cursor are saved
//! in a small [`Patch`], the cursor is blended, the frame is sent, and the
//! patch is put back before the next update: a few hundred bytes per frame
//! instead of a full-frame copy.

use crate::mask::mix;
use crate::nv12::Nv12;

/// A cursor image converted to straight-alpha YUV, cached by X cursor serial.
pub struct CursorImage {
    pub serial: u32,
    pub w: usize,
    pub h: usize,
    pub xhot: i32,
    pub yhot: i32,
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
    a: Vec<u8>,
}

impl CursorImage {
    /// From XFixes' premultiplied ARGB pixels.
    pub fn from_argb(serial: u32, w: usize, h: usize, xhot: i32, yhot: i32, argb: &[u32]) -> Self {
        let n = w * h;
        let (mut y, mut u, mut v, mut a) = (vec![0; n], vec![0; n], vec![0; n], vec![0; n]);
        for (i, &p) in argb.iter().take(n).enumerate() {
            let al = p >> 24;
            if al == 0 {
                continue;
            }
            let un = |c: u32| ((c * 255 + al / 2) / al).min(255) as f32;
            let (r, g, b) = (un((p >> 16) & 255), un((p >> 8) & 255), un(p & 255));
            y[i] = (16.0 + 0.8588 * (0.2126 * r + 0.7152 * g + 0.0722 * b)).round() as u8;
            u[i] = (128.0 + 0.8784 * (-0.1146 * r - 0.3854 * g + 0.5 * b)).round() as u8;
            v[i] = (128.0 + 0.8784 * (0.5 * r - 0.4542 * g - 0.0458 * b)).round() as u8;
            a[i] = al as u8;
        }
        Self {
            serial,
            w,
            h,
            xhot,
            yhot,
            y,
            u,
            v,
            a,
        }
    }
}

/// Saved frame pixels under the cursor (an even-aligned rectangle).
#[derive(Default)]
pub struct Patch {
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    luma: Vec<u8>,
    chroma: Vec<u8>,
}

impl Patch {
    pub fn save(&mut self, f: &Nv12, x: usize, y: usize, w: usize, h: usize) {
        (self.x, self.y, self.w, self.h) = (x, y, w, h);
        self.luma.clear();
        self.chroma.clear();
        let (yp, uvp) = f.planes();
        for r in y..y + h {
            self.luma.extend_from_slice(&yp[r * f.w + x..][..w]);
        }
        for r in (y / 2)..(y + h) / 2 {
            self.chroma.extend_from_slice(&uvp[r * f.w + x..][..w]);
        }
    }

    /// Put the saved pixels back. Idempotent.
    pub fn restore(&mut self, f: &mut Nv12) {
        if self.w == 0 {
            return;
        }
        let fw = f.w;
        let (yp, uvp) = f.planes_mut();
        for (i, r) in (self.y..self.y + self.h).enumerate() {
            yp[r * fw + self.x..][..self.w].copy_from_slice(&self.luma[i * self.w..][..self.w]);
        }
        for (i, r) in (self.y / 2..(self.y + self.h) / 2).enumerate() {
            uvp[r * fw + self.x..][..self.w].copy_from_slice(&self.chroma[i * self.w..][..self.w]);
        }
        self.w = 0;
    }
}

/// Draw `c` with its hotspot at frame pixel (`px`, `py`) after `scale`
/// (1 or 2: a HiDPI source shows the cursor at half size), saving what it
/// covers into `patch` first.
pub fn draw(f: &mut Nv12, c: &CursorImage, px: i32, py: i32, scale: usize, patch: &mut Patch) {
    let s = scale as i32;
    let (cw, ch) = ((c.w as i32 + s - 1) / s, (c.h as i32 + s - 1) / s);
    let (ox, oy) = (px - c.xhot / s, py - c.yhot / s);
    let x0 = (ox.max(0) & !1) as usize;
    let y0 = (oy.max(0) & !1) as usize;
    let x1 = (((ox + cw).min(f.w as i32) + 1) & !1).max(0) as usize;
    let y1 = (((oy + ch).min(f.h as i32) + 1) & !1).max(0) as usize;
    let (x1, y1) = (x1.min(f.w), y1.min(f.h));
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    patch.save(f, x0, y0, x1 - x0, y1 - y0);

    // Sample the cursor at a frame pixel (nearest; the cursor is tiny).
    let at = |fx: usize, fy: usize| -> Option<usize> {
        let (cx, cy) = ((fx as i32 - ox) * s, (fy as i32 - oy) * s);
        (cx >= 0 && cy >= 0 && (cx as usize) < c.w && (cy as usize) < c.h)
            .then(|| cy as usize * c.w + cx as usize)
    };
    let fw = f.w;
    let (yp, uvp) = f.planes_mut();
    for fy in y0..y1 {
        for fx in x0..x1 {
            if let Some(i) = at(fx, fy) {
                let d = &mut yp[fy * fw + fx];
                *d = mix(*d, c.y[i], c.a[i]);
            }
        }
    }
    for fy in (y0..y1).step_by(2) {
        for fx in (x0..x1).step_by(2) {
            // Alpha-weighted average of the 2×2 block.
            let (mut sa, mut su, mut sv) = (0u32, 0u32, 0u32);
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                if let Some(i) = at(fx + dx, fy + dy) {
                    let a = c.a[i] as u32;
                    sa += a;
                    su += a * c.u[i] as u32;
                    sv += a * c.v[i] as u32;
                }
            }
            if sa == 0 {
                continue;
            }
            let a = ((sa + 2) / 4) as u8;
            let o = (fy / 2) * fw + fx;
            uvp[o] = mix(uvp[o], (su / sa) as u8, a);
            uvp[o + 1] = mix(uvp[o + 1], (sv / sa) as u8, a);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draw_then_restore_is_lossless() {
        let mut f = Nv12::new(32, 32);
        for (i, b) in f.data.iter_mut().enumerate() {
            *b = (i * 7) as u8;
        }
        let before = f.data.clone();
        let white = vec![0xffff_ffffu32; 8 * 8];
        let c = CursorImage::from_argb(1, 8, 8, 0, 0, &white);
        let mut p = Patch::default();
        draw(&mut f, &c, 29, 3, 1, &mut p); // partly outside on the right
        assert_ne!(f.data, before);
        p.restore(&mut f);
        assert_eq!(f.data, before);
    }

    #[test]
    fn white_cursor_turns_luma_white() {
        let mut f = Nv12::new(16, 16);
        let c = CursorImage::from_argb(1, 4, 4, 0, 0, &[0xffff_ffff; 16]);
        draw(&mut f, &c, 4, 4, 1, &mut Patch::default());
        assert_eq!(f.planes().0[5 * 16 + 5], 235);
    }
}
