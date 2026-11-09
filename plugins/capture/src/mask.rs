//! The webcam bubble: an anti-aliased rounded-rectangle mask and an NV12
//! compositor that uses it.
//!
//! The mask comes from the signed distance to a rounded box, evaluated once per
//! size and roundness (never per frame). Coverage is `clamp(0.5 − d, 0, 1)`,
//! which is the exact area of a pixel cut by a straight edge and a very good
//! one along an arc. Each row is also reduced to four offsets so compositing
//! copies the opaque middle with `copy_from_slice` and only blends the few
//! edge pixels: a 320×320 circle blends about 2% of its pixels.

use crate::nv12::Nv12;

/// Per-row layout of a mask row: `[a, b)` left edge, `[b, c)` opaque,
/// `[c, d)` right edge. Pixels outside `[a, d)` are fully transparent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Span {
    pub a: u16,
    pub b: u16,
    pub c: u16,
    pub d: u16,
}

/// Coverage for one plane, plus its row spans.
#[derive(Clone, Debug)]
pub struct PlaneMask {
    pub w: usize,
    pub h: usize,
    pub alpha: Vec<u8>,
    pub spans: Vec<Span>,
}

impl PlaneMask {
    fn from_alpha(w: usize, h: usize, alpha: Vec<u8>) -> Self {
        let spans = alpha
            .chunks_exact(w)
            .map(|row| {
                let Some(a) = row.iter().position(|&v| v > 0) else {
                    return Span::default();
                };
                let d = row.iter().rposition(|&v| v > 0).unwrap() + 1;
                let (b, c) = match row.iter().position(|&v| v == 255) {
                    Some(b) => (b, row.iter().rposition(|&v| v == 255).unwrap() + 1),
                    None => (d, d),
                };
                Span {
                    a: a as u16,
                    b: b as u16,
                    c: c as u16,
                    d: d as u16,
                }
            })
            .collect();
        Self { w, h, alpha, spans }
    }

    /// Fraction of covered pixels that need blending (for the stats).
    pub fn edge_ratio(&self) -> f64 {
        let (edge, all) = self.spans.iter().fold((0usize, 0usize), |(e, t), s| {
            let edge = (s.b - s.a + s.d - s.c) as usize;
            (e + edge, t + (s.d - s.a) as usize)
        });
        edge as f64 / all.max(1) as f64
    }
}

/// Luma and chroma masks for an NV12 bubble.
#[derive(Clone, Debug)]
pub struct Mask {
    pub luma: PlaneMask,
    pub chroma: PlaneMask,
    pub roundness: f32,
}

impl Mask {
    /// A `w×h` (even) mask. `roundness` 0 is a square corner, 1 makes the
    /// corner radius half the short side: a circle when `w == h`, a pill
    /// otherwise.
    pub fn rounded(w: usize, h: usize, roundness: f32) -> Self {
        assert!(w.is_multiple_of(2) && h.is_multiple_of(2) && w > 0 && h > 0);
        let roundness = roundness.clamp(0.0, 1.0);
        let (hw, hh) = (w as f32 / 2.0, h as f32 / 2.0);
        let r = roundness * hw.min(hh);
        let (ex, ey) = (hw - r, hh - r);
        let mut alpha = vec![0u8; w * h];
        // The shape is symmetric in both axes: evaluate one quadrant, mirror it.
        for y in 0..h.div_ceil(2) {
            let py = (y as f32 + 0.5 - hh).abs() - ey;
            for x in 0..w.div_ceil(2) {
                let px = (x as f32 + 0.5 - hw).abs() - ex;
                let outside = px.max(0.0).hypot(py.max(0.0));
                let inside = px.max(py).min(0.0);
                let d = outside + inside - r;
                let a = ((0.5 - d).clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                for (yy, xx) in [
                    (y, x),
                    (y, w - 1 - x),
                    (h - 1 - y, x),
                    (h - 1 - y, w - 1 - x),
                ] {
                    alpha[yy * w + xx] = a;
                }
            }
        }
        let (cw, ch) = (w / 2, h / 2);
        let mut ca = vec![0u8; cw * ch];
        for y in 0..ch {
            for x in 0..cw {
                let i = 2 * y * w + 2 * x;
                let s = alpha[i] as u16
                    + alpha[i + 1] as u16
                    + alpha[i + w] as u16
                    + alpha[i + w + 1] as u16;
                ca[y * cw + x] = ((s + 2) >> 2) as u8;
            }
        }
        Self {
            luma: PlaneMask::from_alpha(w, h, alpha),
            chroma: PlaneMask::from_alpha(cw, ch, ca),
            roundness,
        }
    }
}

/// `d + (s − d)·a/255`, exact rounding without a division.
#[inline(always)]
pub fn mix(d: u8, s: u8, a: u8) -> u8 {
    let t = s as u32 * a as u32 + d as u32 * (255 - a as u32) + 128;
    ((t + (t >> 8)) >> 8) as u8
}

/// Blend one plane. `bpp` is 1 for luma, 2 for interleaved UV.
#[allow(clippy::too_many_arguments)]
fn blend_plane(
    dst: &mut [u8],
    dw: usize,
    dh: usize,
    src: &[u8],
    m: &PlaneMask,
    x: isize,
    y: isize,
    bpp: usize,
) {
    for (r, span) in m.spans.iter().enumerate() {
        let dy = y + r as isize;
        if dy < 0 || dy >= dh as isize || span.a == span.d {
            continue;
        }
        // Clip the row to the destination.
        let lo = (-x).max(0) as usize;
        let hi = ((dw as isize - x).max(0) as usize).min(m.w);
        let drow = &mut dst[dy as usize * dw * bpp..][..dw * bpp];
        let srow = &src[r * m.w * bpp..][..m.w * bpp];
        let arow = &m.alpha[r * m.w..][..m.w];
        let at = |c: usize| (x + c as isize) as usize * bpp;
        let edge = |from: usize, to: usize, drow: &mut [u8]| {
            for c in from.max(lo)..to.min(hi) {
                let a = arow[c];
                for k in 0..bpp {
                    let i = at(c) + k;
                    drow[i] = mix(drow[i], srow[c * bpp + k], a);
                }
            }
        };
        edge(span.a as usize, span.b as usize, drow);
        let (b, c) = ((span.b as usize).max(lo), (span.c as usize).min(hi));
        if b < c {
            drow[at(b)..at(c)].copy_from_slice(&srow[b * bpp..c * bpp]);
        }
        edge(span.c as usize, span.d as usize, drow);
    }
}

/// Composite `src` (same size as `mask`) onto `dst` with its top-left corner at
/// `(x, y)`; the corner is rounded down to even so chroma stays aligned. Parts
/// outside `dst` are clipped.
pub fn blend(dst: &mut Nv12, src: &Nv12, mask: &Mask, x: isize, y: isize) {
    assert_eq!((src.w, src.h), (mask.luma.w, mask.luma.h));
    let (x, y) = (x & !1, y & !1);
    let (dw, dh) = (dst.w, dst.h);
    let (sy, suv) = src.planes();
    let (dy_, duv) = dst.planes_mut();
    blend_plane(dy_, dw, dh, sy, &mask.luma, x, y, 1);
    blend_plane(duv, dw / 2, dh / 2, suv, &mask.chroma, x / 2, y / 2, 2);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mix_is_exact() {
        for d in [0u8, 17, 128, 255] {
            for s in [0u8, 99, 255] {
                for a in 0..=255u8 {
                    let want = ((s as f64 * a as f64 + d as f64 * (255 - a) as f64) / 255.0).round()
                        as i32;
                    assert!((mix(d, s, a) as i32 - want).abs() <= 1);
                }
                assert_eq!(mix(d, s, 0), d);
                assert_eq!(mix(d, s, 255), s);
            }
        }
    }

    #[test]
    fn square_is_fully_opaque() {
        let m = Mask::rounded(8, 6, 0.0);
        assert!(m.luma.alpha.iter().all(|&a| a == 255));
        assert_eq!(m.luma.edge_ratio(), 0.0);
    }

    #[test]
    fn circle_area_matches_pi_r_squared() {
        let n = 200;
        let m = Mask::rounded(n, n, 1.0);
        let area: f64 = m.luma.alpha.iter().map(|&a| a as f64 / 255.0).sum();
        let want = std::f64::consts::PI * (n as f64 / 2.0).powi(2);
        assert!((area / want - 1.0).abs() < 0.002, "{area} vs {want}");
        assert_eq!(m.luma.alpha[0], 0);
        assert_eq!(m.luma.alpha[(n / 2) * n + n / 2], 255);
        assert!(m.luma.edge_ratio() < 0.03);
    }

    #[test]
    fn blend_copies_inside_and_keeps_outside() {
        let mut dst = Nv12::new(16, 16);
        let mut src = Nv12::new(8, 8);
        src.data.fill(200);
        let m = Mask::rounded(8, 8, 1.0);
        blend(&mut dst, &src, &m, 4, 4);
        let (y, _) = dst.planes();
        assert_eq!(y[8 * 16 + 8], 200); // centre
        assert_eq!(y[4 * 16 + 4], 16); // bubble corner stays transparent
        assert_eq!(y[0], 16);
    }

    #[test]
    fn blend_clips_at_every_edge() {
        let mut dst = Nv12::new(10, 10);
        let mut src = Nv12::new(8, 8);
        src.data.fill(200);
        let m = Mask::rounded(8, 8, 0.0);
        for (x, y) in [(-6, -6), (6, 6), (-6, 6), (6, -6), (-100, 0), (100, 100)] {
            blend(&mut dst, &src, &m, x, y);
        }
        assert_eq!(dst.planes().0[0], 200);
        assert_eq!(dst.planes().0[99], 200);
    }
}
