//! NV12 scaling for the webcam bubble.
//!
//! The camera is captured at the smallest mode that covers the bubble
//! ([`pick_mode`]), so this only ever shrinks a few hundred pixels. Bilinear
//! with 8-bit fixed-point weights; column taps are computed once per size and
//! reused every frame ([`Scaler`]). An exact 2× ratio takes the box-filter path.

use crate::nv12::Nv12;

/// Pick the capture mode with the fewest pixels that still covers `need`
/// (w, h); the largest mode when none does. Fewer pixels means less USB
/// bandwidth, fewer bytes to copy and less to scale.
pub fn pick_mode(modes: &[(u32, u32)], need: (u32, u32)) -> Option<(u32, u32)> {
    let covers = |m: &&(u32, u32)| m.0 >= need.0 && m.1 >= need.1;
    let area = |m: &&(u32, u32)| m.0 as u64 * m.1 as u64;
    modes
        .iter()
        .filter(covers)
        .min_by_key(area)
        .or_else(|| modes.iter().max_by_key(area))
        .copied()
}

#[derive(Clone, Copy)]
struct Tap {
    i0: u32,
    i1: u32,
    w: u16, // weight of i1, 0..=256
}

/// Taps that sample the window `[off, off + len)` of a `src`-wide line.
fn taps_in(off: usize, len: usize, src: usize, dst: usize) -> Vec<Tap> {
    // Pixel centres: s = off + (d + 0.5)·len/dst − 0.5, in 8.8 fixed point.
    (0..dst)
        .map(|d| {
            let f = (off * 256 + ((2 * d + 1) * len * 256) / (2 * dst)) as i64 - 128;
            let f = f.clamp(0, ((src - 1) * 256) as i64) as usize;
            let i0 = f >> 8;
            Tap {
                i0: i0 as u32,
                i1: (i0 + 1).min(src - 1) as u32,
                w: (f & 255) as u16,
            }
        })
        .collect()
}

/// The largest centred window of a `sw×sh` source with the aspect of
/// `dw×dh` (even offsets and sizes): what the bubble shows of the camera.
pub fn cover_crop(
    sw: usize,
    sh: usize,
    dw: usize,
    dh: usize,
    zoom: f32,
) -> (usize, usize, usize, usize) {
    let zoom = zoom.clamp(1.0, 4.0);
    let (sa, da) = (sw as f64 / sh as f64, dw as f64 / dh as f64);
    let (mut cw, mut ch) = if sa > da {
        ((sh as f64 * da) as usize, sh)
    } else {
        (sw, (sw as f64 / da) as usize)
    };
    cw = ((cw as f32 / zoom) as usize & !1).max(2);
    ch = ((ch as f32 / zoom) as usize & !1).max(2);
    (((sw - cw) / 2) & !1, ((sh - ch) / 2) & !1, cw, ch)
}

#[inline(always)]
fn lerp(a: u8, b: u8, w: u16) -> u16 {
    a as u16 * (256 - w) + b as u16 * w
}

/// A reusable bilinear NV12 scaler for one (source, destination) size pair.
pub struct Scaler {
    sw: usize,
    sh: usize,
    dw: usize,
    dh: usize,
    xl: Vec<Tap>,
    yl: Vec<Tap>,
    xc: Vec<Tap>,
    yc: Vec<Tap>,
    row0: Vec<u16>,
    row1: Vec<u16>,
    plain: bool,
}

impl Scaler {
    pub fn new(sw: usize, sh: usize, dw: usize, dh: usize) -> Self {
        Self::window(sw, sh, (0, 0, sw, sh), dw, dh, false)
    }

    /// Scale the window `(x, y, w, h)` of the source (even values), mirrored
    /// left to right when `mirror`. Both come free: the window offsets the
    /// taps and the mirror reverses their order.
    pub fn window(
        sw: usize,
        sh: usize,
        (cx, cy, cw, ch): (usize, usize, usize, usize),
        dw: usize,
        dh: usize,
        mirror: bool,
    ) -> Self {
        let mut xl = taps_in(cx, cw, sw, dw);
        let mut xc = taps_in(cx / 2, cw / 2, sw / 2, dw / 2);
        if mirror {
            xl.reverse();
            xc.reverse();
        }
        Self {
            sw,
            sh,
            dw,
            dh,
            xl,
            yl: taps_in(cy, ch, sh, dh),
            xc,
            yc: taps_in(cy / 2, ch / 2, sh / 2, dh / 2),
            row0: vec![0; dw],
            row1: vec![0; dw],
            plain: (cx, cy, cw, ch) == (0, 0, sw, sh) && !mirror,
        }
    }

    pub fn matches(&self, src: &Nv12, dst: &Nv12) -> bool {
        (self.sw, self.sh, self.dw, self.dh) == (src.w, src.h, dst.w, dst.h)
    }

    pub fn run(&mut self, src: &Nv12, dst: &mut Nv12) {
        assert!(self.matches(src, dst));
        let (sy, suv) = src.planes();
        let (dy, duv) = dst.planes_mut();
        if self.plain && self.sw == 2 * self.dw && self.sh == 2 * self.dh {
            halve_plane(sy, self.sw, dy, self.dw, self.dh, 1);
            halve_plane(suv, self.sw / 2, duv, self.dw / 2, self.dh / 2, 2);
            return;
        }
        let (sw, dw) = (self.sw, self.dw);
        plane::<1>(
            sy,
            sw,
            dy,
            dw,
            &self.xl,
            &self.yl,
            &mut self.row0,
            &mut self.row1,
        );
        plane::<2>(
            suv,
            sw / 2,
            duv,
            dw / 2,
            &self.xc,
            &self.yc,
            &mut self.row0,
            &mut self.row1,
        );
    }
}

/// Horizontal pass of one source row into 8.8 fixed point.
// Const-generic chunk sizes: `as_chunks` cannot take `2 * B` on stable.
#[allow(clippy::chunks_exact_to_as_chunks)]
#[inline(always)]
fn hpass<const B: usize>(src: &[u8], xt: &[Tap], out: &mut [u16]) {
    for (o, t) in out.chunks_exact_mut(B).zip(xt) {
        let (i0, i1, w) = (t.i0 as usize * B, t.i1 as usize * B, t.w);
        let (a, b) = (&src[i0..i0 + B], &src[i1..i1 + B]);
        for k in 0..B {
            o[k] = lerp(a[k], b[k], w);
        }
    }
}

/// Separable bilinear: each source row is filtered horizontally at most once
/// (the two most recent rows are kept), then rows are blended vertically,
/// which is a straight loop the compiler vectorises.
#[allow(clippy::too_many_arguments)]
fn plane<const B: usize>(
    src: &[u8],
    sw: usize,
    dst: &mut [u8],
    dw: usize,
    xt: &[Tap],
    yt: &[Tap],
    r0: &mut Vec<u16>,
    r1: &mut Vec<u16>,
) {
    let n = dw * B;
    r0.resize(n, 0);
    r1.resize(n, 0);
    let row = |i: u32| &src[i as usize * sw * B..][..sw * B];
    let (mut have0, mut have1) = (u32::MAX, u32::MAX);
    for (dy, ty) in yt.iter().enumerate() {
        if have0 != ty.i0 {
            if have1 == ty.i0 {
                std::mem::swap(r0, r1);
                std::mem::swap(&mut have0, &mut have1);
            } else {
                hpass::<B>(row(ty.i0), xt, &mut r0[..n]);
                have0 = ty.i0;
            }
        }
        if have1 != ty.i1 {
            hpass::<B>(row(ty.i1), xt, &mut r1[..n]);
            have1 = ty.i1;
        }
        let w = ty.w as u32;
        let out = &mut dst[dy * n..][..n];
        for ((o, &p), &q) in out.iter_mut().zip(&r0[..n]).zip(&r1[..n]) {
            *o = ((p as u32 * (256 - w) + q as u32 * w + 32768) >> 16) as u8;
        }
    }
}

fn halve_plane(src: &[u8], sw: usize, dst: &mut [u8], dw: usize, dh: usize, bpp: usize) {
    if bpp == 1 {
        halve::<1>(src, sw, dst, dw, dh)
    } else {
        halve::<2>(src, sw, dst, dw, dh)
    }
}

/// 2× box filter, rounded like `vpavgb` twice (vertical, then horizontal).
#[allow(clippy::chunks_exact_to_as_chunks)]
fn halve<const B: usize>(src: &[u8], sw: usize, dst: &mut [u8], dw: usize, dh: usize) {
    let avg = |a: u8, b: u8| ((a as u16 + b as u16 + 1) >> 1) as u8;
    let (sn, dn) = (sw * B, dw * B);
    for y in 0..dh {
        let a = &src[2 * y * sn..][..sn];
        let b = &src[(2 * y + 1) * sn..][..sn];
        let out = &mut dst[y * dn..][..dn];
        for ((o, pa), pb) in out
            .chunks_exact_mut(B)
            .zip(a.chunks_exact(2 * B))
            .zip(b.chunks_exact(2 * B))
        {
            for k in 0..B {
                o[k] = avg(avg(pa[k], pb[k]), avg(pa[k + B], pb[k + B]));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirror_reverses_rows_and_keeps_chroma_pairs() {
        let mut src = Nv12::new(4, 2);
        src.data = vec![1, 2, 3, 4, 5, 6, 7, 8, 10, 20, 30, 40];
        let mut dst = Nv12::new(4, 2);
        Scaler::window(4, 2, (0, 0, 4, 2), 4, 2, true).run(&src, &mut dst);
        assert_eq!(dst.data, vec![4, 3, 2, 1, 8, 7, 6, 5, 30, 40, 10, 20]);
    }

    #[test]
    fn cover_crop_keeps_the_centre() {
        assert_eq!(cover_crop(1280, 720, 320, 320, 1.0), (280, 0, 720, 720));
        assert_eq!(cover_crop(640, 480, 640, 360, 1.0), (0, 60, 640, 360));
        let (x, y, w, h) = cover_crop(1280, 720, 320, 320, 2.0);
        assert_eq!((w, h), (360, 360));
        assert_eq!((x, y), (460, 180));
    }

    #[test]
    fn picks_smallest_covering_mode() {
        let modes = [(1280, 720), (640, 480)];
        assert_eq!(pick_mode(&modes, (320, 320)), Some((640, 480)));
        assert_eq!(pick_mode(&modes, (600, 600)), Some((1280, 720)));
        assert_eq!(pick_mode(&modes, (2000, 2000)), Some((1280, 720)));
    }

    #[test]
    fn flat_frame_stays_flat_at_any_ratio() {
        let mut src = Nv12::new(640, 480);
        let (y, uv) = src.planes_mut();
        y.fill(180);
        uv.fill(90);
        for (w, h) in [(320, 240), (300, 300), (642, 482), (96, 96)] {
            let mut dst = Nv12::new(w, h);
            Scaler::new(640, 480, w, h).run(&src, &mut dst);
            let (y, uv) = dst.planes();
            assert!(y.iter().all(|&v| v == 180), "{w}x{h}");
            assert!(uv.iter().all(|&v| v == 90), "{w}x{h}");
        }
    }

    #[test]
    fn gradient_is_preserved() {
        let mut src = Nv12::new(256, 2);
        for (i, v) in src.planes_mut().0.iter_mut().enumerate() {
            *v = (i % 256) as u8;
        }
        let mut dst = Nv12::new(128, 2);
        Scaler::new(256, 2, 128, 2).run(&src, &mut dst);
        let y = dst.planes().0;
        assert!((y[64] as i32 - 129).abs() <= 1, "{}", y[64]);
    }
}
