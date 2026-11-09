//! NV12 frames and the BGRX → NV12 converter.
//!
//! BT.709 limited range with 7-bit coefficients, the same scheme libyuv uses:
//! `vpmaddubsw` multiplies unsigned pixel bytes by signed coefficient bytes
//! and adds neighbours in one instruction, so the coefficients must fit an
//! `i8`. Luma weights sum to 110 = 219/255 · 128; each chroma row sums to 0,
//! so grey stays exactly neutral. Chroma averages each 2×2 block with two
//! rounded byte averages (`vpavgb`), vertical then horizontal.
//!
//! The scalar path does the same arithmetic in the same order, so both paths
//! produce identical bytes; tests check that.

/// Luma weights for B, G, R (7-bit).
const YB: i32 = 8;
const YG: i32 = 79;
const YR: i32 = 23;
/// Chroma weights for B, G, R (7-bit).
const UB: i32 = 56;
const UG: i32 = -43;
const UR: i32 = -13;
const VB: i32 = -5;
const VG: i32 = -51;
const VR: i32 = 56;

/// One NV12 frame: a `w×h` luma plane followed by an interleaved `w×h/2` UV
/// plane. Both planes have a stride of `w` bytes.
#[derive(Clone)]
pub struct Nv12 {
    pub w: usize,
    pub h: usize,
    pub data: Vec<u8>,
}

impl Nv12 {
    /// A black frame. `w` and `h` must be even.
    pub fn new(w: usize, h: usize) -> Self {
        assert!(
            w.is_multiple_of(2) && h.is_multiple_of(2),
            "NV12 needs even dimensions"
        );
        let mut data = vec![16u8; w * h * 3 / 2];
        data[w * h..].fill(128);
        Self { w, h, data }
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn planes(&self) -> (&[u8], &[u8]) {
        self.data.split_at(self.w * self.h)
    }

    pub fn planes_mut(&mut self) -> (&mut [u8], &mut [u8]) {
        self.data.split_at_mut(self.w * self.h)
    }
}

#[inline(always)]
fn word(s: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([s[4 * i], s[4 * i + 1], s[4 * i + 2], s[4 * i + 3]])
}

#[inline(always)]
fn chan(p: u32, k: u32) -> i32 {
    ((p >> (8 * k)) & 255) as i32
}

#[inline(always)]
fn luma(p: u32) -> u8 {
    (((YB * chan(p, 0) + YG * chan(p, 1) + YR * chan(p, 2) + 64) >> 7) + 16) as u8
}

/// Per-channel rounded average of two pixels (`vpavgb`).
#[inline(always)]
fn avg(a: u32, b: u32) -> u32 {
    // (a | b) − ((a ^ b) >> 1) per byte, without carries between bytes.
    (a | b) - (((a ^ b) >> 1) & 0x7f7f_7f7f)
}

/// Chroma from two vertically averaged pixels.
#[inline(always)]
fn chroma(p: u32, q: u32) -> (u8, u8) {
    let su = UB * (chan(p, 0) + chan(q, 0))
        + UG * (chan(p, 1) + chan(q, 1))
        + UR * (chan(p, 2) + chan(q, 2));
    let sv = VB * (chan(p, 0) + chan(q, 0))
        + VG * (chan(p, 1) + chan(q, 1))
        + VR * (chan(p, 2) + chan(q, 2));
    (
        (((su + 128) >> 8) + 128) as u8,
        (((sv + 128) >> 8) + 128) as u8,
    )
}

fn luma_row(src: &[u8], dst: &mut [u8]) {
    for (i, y) in dst.iter_mut().enumerate() {
        *y = luma(word(src, i));
    }
}

fn chroma_row(s0: &[u8], s1: &[u8], dst: &mut [u8]) {
    for i in 0..dst.len() / 2 {
        let p = avg(word(s0, 2 * i), word(s1, 2 * i));
        let q = avg(word(s0, 2 * i + 1), word(s1, 2 * i + 1));
        let (u, v) = chroma(p, q);
        dst[2 * i] = u;
        dst[2 * i + 1] = v;
    }
}

/// 2×2 box average into one pixel: vertical then horizontal `avg`.
fn halve_rows(s0: &[u8], s1: &[u8], dst: &mut [u8]) {
    for i in 0..dst.len() / 4 {
        let a = avg(word(s0, 2 * i), word(s1, 2 * i));
        let b = avg(word(s0, 2 * i + 1), word(s1, 2 * i + 1));
        dst[4 * i..4 * i + 4].copy_from_slice(&avg(a, b).to_le_bytes());
    }
}

#[cfg(target_arch = "x86_64")]
mod avx2 {
    //! Hand-written AVX2 kernels. Each handles whole 16-pixel blocks and
    //! returns how many pixels it did; the scalar code finishes the row.
    use super::*;
    use std::arch::x86_64::*;

    #[inline(always)]
    unsafe fn coef(b: i32, g: i32, r: i32) -> __m256i {
        let w = (b as u8 as i32) | ((g as u8 as i32) << 8) | ((r as u8 as i32) << 16);
        _mm256_set1_epi32(w)
    }

    /// Interleaved dword order after the lane-wise hadd/pack below.
    #[inline(always)]
    unsafe fn order() -> __m256i {
        _mm256_setr_epi32(0, 4, 1, 5, 2, 6, 3, 7)
    }

    #[target_feature(enable = "avx2")]
    pub unsafe fn luma_row(src: &[u8], dst: &mut [u8]) -> usize {
        let n = dst.len() / 16 * 16;
        let (k, round, sixteen, perm) = (
            coef(YB, YG, YR),
            _mm256_set1_epi16(64),
            _mm256_set1_epi16(16),
            order(),
        );
        let (s, d) = (src.as_ptr(), dst.as_mut_ptr());
        let mut i = 0;
        while i < n {
            let a = _mm256_maddubs_epi16(_mm256_loadu_si256(s.add(4 * i) as *const __m256i), k);
            let b =
                _mm256_maddubs_epi16(_mm256_loadu_si256(s.add(4 * i + 32) as *const __m256i), k);
            let y = _mm256_add_epi16(
                _mm256_srli_epi16(_mm256_add_epi16(_mm256_hadd_epi16(a, b), round), 7),
                sixteen,
            );
            let y = _mm256_permutevar8x32_epi32(_mm256_packus_epi16(y, y), perm);
            _mm_storeu_si128(d.add(i) as *mut __m128i, _mm256_castsi256_si128(y));
            i += 16;
        }
        n
    }

    #[target_feature(enable = "avx2")]
    pub unsafe fn chroma_row(s0: &[u8], s1: &[u8], dst: &mut [u8]) -> usize {
        let n = (dst.len() / 2) / 8 * 16; // pixels per row handled
        let (ku, kv, perm) = (coef(UB, UG, UR), coef(VB, VG, VR), order());
        let (round, mid) = (_mm256_set1_epi16(128), _mm256_set1_epi16(128));
        // Within each lane: U0 V0 U1 V1 U2 V2 U3 V3 from U0..U3 V0..V3.
        let inter = _mm256_setr_epi8(
            0, 4, 1, 5, 2, 6, 3, 7, -1, -1, -1, -1, -1, -1, -1, -1, 0, 4, 1, 5, 2, 6, 3, 7, -1, -1,
            -1, -1, -1, -1, -1, -1,
        );
        let (a, b, d) = (s0.as_ptr(), s1.as_ptr(), dst.as_mut_ptr());
        let mut i = 0;
        while i < n {
            let ld =
                |p: *const u8, o: usize| _mm256_loadu_si256(p.add(4 * (i + o)) as *const __m256i);
            let v0 = _mm256_avg_epu8(ld(a, 0), ld(b, 0));
            let v1 = _mm256_avg_epu8(ld(a, 8), ld(b, 8));
            let hu = _mm256_hadd_epi16(_mm256_maddubs_epi16(v0, ku), _mm256_maddubs_epi16(v1, ku));
            let hv = _mm256_hadd_epi16(_mm256_maddubs_epi16(v0, kv), _mm256_maddubs_epi16(v1, kv));
            let uv = _mm256_add_epi16(
                _mm256_srai_epi16(_mm256_add_epi16(_mm256_hadd_epi16(hu, hv), round), 8),
                mid,
            );
            let uv = _mm256_shuffle_epi8(_mm256_packus_epi16(uv, uv), inter);
            let uv = _mm256_permutevar8x32_epi32(uv, perm);
            _mm_storeu_si128(d.add(i) as *mut __m128i, _mm256_castsi256_si128(uv));
            i += 16;
        }
        n
    }

    #[target_feature(enable = "avx2")]
    pub unsafe fn halve_rows(s0: &[u8], s1: &[u8], dst: &mut [u8]) -> usize {
        let n = (dst.len() / 4) / 8 * 8; // output pixels
        let (a, b, d) = (s0.as_ptr(), s1.as_ptr(), dst.as_mut_ptr());
        let mut i = 0;
        while i < n {
            let ld = |p: *const u8, o: usize| {
                _mm256_loadu_si256(p.add(8 * i + 32 * o) as *const __m256i)
            };
            let x = _mm256_castsi256_ps(_mm256_avg_epu8(ld(a, 0), ld(b, 0)));
            let y = _mm256_castsi256_ps(_mm256_avg_epu8(ld(a, 1), ld(b, 1)));
            let even = _mm256_castps_si256(_mm256_shuffle_ps(x, y, 0b10_00_10_00));
            let odd = _mm256_castps_si256(_mm256_shuffle_ps(x, y, 0b11_01_11_01));
            let h = _mm256_permute4x64_epi64(_mm256_avg_epu8(even, odd), 0b11_01_10_00);
            _mm256_storeu_si256(d.add(4 * i) as *mut __m256i, h);
            i += 8;
        }
        n
    }
}

/// Row kernels, picked once per call.
#[derive(Clone, Copy)]
struct Kernels {
    simd: bool,
}

impl Kernels {
    fn new(simd: bool) -> Self {
        #[cfg(target_arch = "x86_64")]
        return Self {
            simd: simd && std::is_x86_feature_detected!("avx2"),
        };
        #[cfg(not(target_arch = "x86_64"))]
        Self {
            simd: {
                let _ = simd;
                false
            },
        }
    }

    fn luma(self, src: &[u8], dst: &mut [u8]) {
        let mut done = 0;
        #[cfg(target_arch = "x86_64")]
        if self.simd {
            // SAFETY: AVX2 checked in `new`; the kernel stays inside both slices.
            done = unsafe { avx2::luma_row(src, dst) };
        }
        luma_row(&src[done * 4..], &mut dst[done..]);
    }

    fn chroma(self, s0: &[u8], s1: &[u8], dst: &mut [u8]) {
        let mut done = 0;
        #[cfg(target_arch = "x86_64")]
        if self.simd {
            // SAFETY: as above.
            done = unsafe { avx2::chroma_row(s0, s1, dst) };
        }
        chroma_row(&s0[done * 4..], &s1[done * 4..], &mut dst[done..]);
    }

    fn halve(self, s0: &[u8], s1: &[u8], dst: &mut [u8]) {
        let mut done = 0;
        #[cfg(target_arch = "x86_64")]
        if self.simd {
            // SAFETY: as above.
            done = unsafe { avx2::halve_rows(s0, s1, dst) };
        }
        halve_rows(&s0[done * 8..], &s1[done * 8..], &mut dst[done * 4..]);
    }
}

/// A destination rectangle inside a frame: plane slices that start at the
/// rectangle's origin, plus the frame stride.
struct Dst<'a> {
    y: &'a mut [u8],
    uv: &'a mut [u8],
    stride: usize,
}

fn convert(src: &[u8], sstride: usize, w: usize, h: usize, d: Dst<'_>, half: bool, simd: bool) {
    let k = Kernels::new(simd);
    let mut scratch = vec![0u8; if half { w * 8 } else { 0 }];
    let (t0, t1) = scratch.split_at_mut(if half { w * 4 } else { 0 });
    for r in (0..h).step_by(2) {
        let (s0, s1): (&[u8], &[u8]) = if half {
            let base = 2 * r * sstride;
            let row = |k: usize| &src[base + k * sstride..][..w * 8];
            k.halve(row(0), row(1), t0);
            k.halve(row(2), row(3), t1);
            (&*t0, &*t1)
        } else {
            (
                &src[r * sstride..][..w * 4],
                &src[(r + 1) * sstride..][..w * 4],
            )
        };
        k.luma(s0, &mut d.y[r * d.stride..][..w]);
        k.luma(s1, &mut d.y[(r + 1) * d.stride..][..w]);
        k.chroma(s0, s1, &mut d.uv[(r / 2) * d.stride..][..w]);
    }
}

/// How a block of BGRX pixels lands in the frame.
#[derive(Clone, Copy, Debug)]
pub struct Convert {
    /// Destination origin in frame pixels (even).
    pub dx: usize,
    pub dy: usize,
    /// Destination size in frame pixels (even).
    pub w: usize,
    pub h: usize,
    /// The source is 2× the destination in both axes (box-filtered down).
    pub half: bool,
    /// Worker threads (row bands). 1 runs inline.
    pub threads: usize,
    /// Use the AVX2 build when available.
    pub simd: bool,
}

/// Convert packed BGRX (`sstride` bytes per row) into `frame` at `c`.
pub fn bgrx_into(frame: &mut Nv12, src: &[u8], sstride: usize, c: Convert) {
    assert!(
        c.dx.is_multiple_of(2)
            && c.dy.is_multiple_of(2)
            && c.w.is_multiple_of(2)
            && c.h.is_multiple_of(2)
    );
    assert!(c.dx + c.w <= frame.w && c.dy + c.h <= frame.h);
    if c.w == 0 || c.h == 0 {
        return;
    }
    let fw = frame.w;
    let (yp, uvp) = frame.planes_mut();
    let mut y = &mut yp[c.dy * fw + c.dx..];
    let mut uv = &mut uvp[(c.dy / 2) * fw + c.dx..];
    let scale = if c.half { 2 } else { 1 };

    // Bands of an even number of rows; tiny jobs stay on this thread.
    let threads = c.threads.clamp(1, 16).min((c.w * c.h / 65_536).max(1));
    let band = ((c.h / threads) + 1) & !1;
    if threads == 1 || band >= c.h {
        return convert(
            src,
            sstride,
            c.w,
            c.h,
            Dst { y, uv, stride: fw },
            c.half,
            c.simd,
        );
    }
    std::thread::scope(|s| {
        let mut r = 0;
        while r < c.h {
            let rows = band.min(c.h - r);
            let last = r + rows >= c.h;
            let (yt, ut) = (std::mem::take(&mut y), std::mem::take(&mut uv));
            let (yb, yr) = if last {
                (yt, &mut [][..])
            } else {
                yt.split_at_mut(rows * fw)
            };
            let (ub, ur) = if last {
                (ut, &mut [][..])
            } else {
                ut.split_at_mut(rows / 2 * fw)
            };
            y = yr;
            uv = ur;
            let s0 = r * scale * sstride;
            let srcb = &src[s0..];
            let (w, half, simd) = (c.w, c.half, c.simd);
            s.spawn(move || {
                convert(
                    srcb,
                    sstride,
                    w,
                    rows,
                    Dst {
                        y: yb,
                        uv: ub,
                        stride: fw,
                    },
                    half,
                    simd,
                )
            });
            r += rows;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: usize, h: usize, b: u8, g: u8, r: u8) -> Vec<u8> {
        (0..w * h).flat_map(|_| [b, g, r, 0]).collect()
    }

    fn full(w: usize, h: usize) -> Convert {
        Convert {
            dx: 0,
            dy: 0,
            w,
            h,
            half: false,
            threads: 1,
            simd: true,
        }
    }

    #[test]
    fn primaries_match_bt709_limited() {
        // Reference values from BT.709 limited range, rounded.
        for (bgr, yuv) in [
            ([0, 0, 0], (16, 128, 128)),
            ([255, 255, 255], (235, 128, 128)),
            ([0, 0, 255], (63, 102, 240)),
            ([0, 255, 0], (173, 42, 26)),
            ([255, 0, 0], (32, 240, 118)),
        ] {
            let mut f = Nv12::new(2, 2);
            bgrx_into(&mut f, &solid(2, 2, bgr[0], bgr[1], bgr[2]), 8, full(2, 2));
            let (y, uv) = f.planes();
            let got = (y[0] as i32, uv[0] as i32, uv[1] as i32);
            let want = (yuv.0, yuv.1, yuv.2);
            assert!(
                (got.0 - want.0).abs() <= 1
                    && (got.1 - want.1).abs() <= 1
                    && (got.2 - want.2).abs() <= 1,
                "{bgr:?}: {got:?} vs {want:?}"
            );
        }
    }

    #[test]
    fn simd_threads_and_scalar_agree() {
        let (w, h) = (640, 360);
        let src: Vec<u8> = (0..w * h * 4)
            .map(|i| ((i * 2654435761usize) >> 13) as u8)
            .collect();
        let mut a = Nv12::new(w, h);
        let mut b = Nv12::new(w, h);
        bgrx_into(
            &mut a,
            &src,
            w * 4,
            Convert {
                simd: false,
                ..full(w, h)
            },
        );
        bgrx_into(
            &mut b,
            &src,
            w * 4,
            Convert {
                threads: 4,
                ..full(w, h)
            },
        );
        assert!(a.data == b.data);
    }

    #[test]
    fn partial_update_touches_only_its_rect() {
        let mut f = Nv12::new(8, 8);
        bgrx_into(
            &mut f,
            &solid(4, 2, 255, 255, 255),
            16,
            Convert {
                dx: 2,
                dy: 4,
                ..full(4, 2)
            },
        );
        let (y, uv) = f.planes();
        for row in 0..8 {
            for col in 0..8 {
                let inside = (2..6).contains(&col) && (4..6).contains(&row);
                assert_eq!(y[row * 8 + col], if inside { 235 } else { 16 });
            }
        }
        assert_eq!(uv.iter().filter(|&&v| v != 128).count(), 0);
    }

    #[test]
    fn half_scale_box_filters() {
        // A 4×4 checker of black/white averages to mid grey.
        let mut src = Vec::new();
        for r in 0..4 {
            for c in 0..4 {
                let v = if (r + c) % 2 == 0 { 255 } else { 0 };
                src.extend_from_slice(&[v, v, v, 0]);
            }
        }
        let mut f = Nv12::new(2, 2);
        bgrx_into(
            &mut f,
            &src,
            16,
            Convert {
                half: true,
                ..full(2, 2)
            },
        );
        assert!((f.planes().0[0] as i32 - 126).abs() <= 1);
    }
}
