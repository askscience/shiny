//! An eight-line feedback-delay-network (FDN) reverb.
//!
//! * Input: smoothed, interpolated pre-delay, then two four-stage all-pass
//!   diffusers (one per input channel) whose lengths scale with both the
//!   sample rate and `size`.
//! * Network: eight mutually-prime delay lines (lengths scale with `size`)
//!   mixed by an orthonormal 8×8 Hadamard matrix (lossless, maximally dense).
//! * Decay: each line has a gain derived from the target RT60 (set by `size`)
//!   and a first-order high-shelf whose Nyquist gain gives a shorter RT60 at
//!   high frequencies (`damping`), so the tail darkens naturally as it decays.
//! * Modulation: each line's read point is swept by its own slow LFO through a
//!   cubic-interpolated read (no truncation crackle), which breaks up modes and
//!   metallic ringing.
//! * Output: two orthogonal tap vectors (decorrelated L/R), DC blocker, M/S
//!   width and an equal-power `mix`.
//!
//! Silent input produces an exactly-zero output once the tail has decayed
//! below −180 dBFS: state is flushed below 1e-20 and the network is cleared.

use std::f64::consts::PI;

use super::delay::{clamp_sr, equal_power, flush, param, sanitize, Allpass, DcBlock, DelayLine, Lfo, Smooth, DEFAULT_SR};

const N: usize = 8;
/// Line lengths in ms at `size = 1` (scaled by `0.35 + 0.65·size`).
const LINE_MS: [f64; N] = [31.7, 37.3, 41.9, 47.3, 53.1, 59.3, 66.7, 73.9];
/// Diffuser lengths in ms at `size = 1`, left and right chains.
const AP_MS: [[f64; 4]; 2] = [[4.77, 3.59, 12.73, 9.31], [5.03, 3.83, 11.87, 8.89]];
const AP_G: [f64; 4] = [0.75, 0.75, 0.625, 0.625];
/// LFO rates for the line modulation (Hz).
const MOD_HZ: [f64; N] = [0.31, 0.37, 0.43, 0.53, 0.61, 0.71, 0.83, 0.97];
/// Peak modulation excursion at `mod_amount = 1`, in ms.
const MOD_MS: f64 = 0.3;
const SIZE_MIN: f64 = 0.05;
const SIZE_MAX: f64 = 1.6;
const PREDELAY_MAX_MS: f64 = 500.0;
/// Injection gain per line and output tap gain.
const IN_GAIN: f64 = 0.5;
const OUT_GAIN: f64 = 0.354; // 1/√8
/// Sign patterns of the output taps (orthogonal → decorrelated L/R).
const TAP_L: [f64; N] = [1.0, -1.0, 1.0, -1.0, 1.0, -1.0, 1.0, -1.0];
const TAP_R: [f64; N] = [1.0, 1.0, -1.0, -1.0, 1.0, 1.0, -1.0, -1.0];
/// Injection signs: left feeds the even lines, right the odd lines.
const INJ: [f64; N] = [1.0, 1.0, -1.0, -1.0, 1.0, -1.0, -1.0, 1.0];
/// Below this the network counts as empty.
const QUIET: f64 = 1e-9;

/// Target RT60 (seconds) for a `size`: 0.05 → 0.3 s, 0.5 → 1.8 s, 1.6 → 8 s,
/// log-linear in between.
fn rt60_for(size: f64) -> f64 {
    let s = param(size, SIZE_MIN, SIZE_MAX, 0.5);
    let (s0, t0, s1, t1) = if s <= 0.5 { (SIZE_MIN, 0.3f64, 0.5, 1.8f64) } else { (0.5, 1.8, SIZE_MAX, 8.0) };
    let f = (s - s0) / (s1 - s0);
    (t0.ln() + (t1.ln() - t0.ln()) * f).exp()
}

fn is_prime(n: usize) -> bool {
    if n < 2 {
        return false;
    }
    if n % 2 == 0 {
        return n == 2;
    }
    let mut d = 3;
    while d * d <= n {
        if n % d == 0 {
            return false;
        }
        d += 2;
    }
    true
}

fn next_prime(mut n: usize) -> usize {
    while !is_prime(n) {
        n += 1;
    }
    n
}

/// In-place orthonormal fast Walsh–Hadamard transform of size 8.
#[inline]
fn hadamard(x: &mut [f64; N]) {
    let mut h = 1;
    while h < N {
        let mut i = 0;
        while i < N {
            for j in i..i + h {
                let a = x[j];
                let b = x[j + h];
                x[j] = a + b;
                x[j + h] = a - b;
            }
            i += 2 * h;
        }
        h *= 2;
    }
    let k = 1.0 / (N as f64).sqrt();
    for v in x.iter_mut() {
        *v *= k;
    }
}

/// Per-line absorption: gain `g0` times a first-order shelf (1 at DC, `r` at
/// Nyquist), bilinear-designed at a corner frequency.
#[derive(Clone, Copy, Default)]
struct Absorb {
    g0: f64,
    b0: f64,
    b1: f64,
    a1: f64,
    x1: f64,
    y1: f64,
}

impl Absorb {
    #[inline]
    fn tick(&mut self, x: f64) -> f64 {
        let y = flush(self.b0 * x + self.b1 * self.x1 - self.a1 * self.y1);
        self.x1 = x;
        self.y1 = y;
        y * self.g0
    }
}

#[derive(Clone)]
pub struct Reverb {
    pub size: f64,
    pub damping: f64,
    pub mix: f64,
    pub predelay_ms: f64,
    pub width: f64,
    pub mod_amount: f64,
    sr: f64,
    lines: [DelayLine; N],
    len_s: [Smooth; N],
    len_target: [f64; N],
    absorb: [Absorb; N],
    ap: [[Allpass; 4]; 2],
    ap_s: [[Smooth; 4]; 2],
    ap_target: [[f64; 4]; 2],
    pre_l: DelayLine,
    pre_r: DelayLine,
    pre_s: Smooth,
    mods: [Lfo; N],
    mod_s: Smooth,
    width_s: Smooth,
    mix_s: Smooth,
    dc_l: DcBlock,
    dc_r: DcBlock,
    size_cached: f64,
    damping_cached: f64,
    /// Shelf corner and HF/DC RT60 ratio derived from `damping`.
    damp_fc: f64,
    damp_ratio: f64,
    rt: f64,
    decay_countdown: u32,
    quiet: usize,
    idle: bool,
}

impl Reverb {
    pub fn new(size: f64, damping: f64, mix: f64) -> Self {
        let sr = DEFAULT_SR;
        let mut r = Self {
            size,
            damping,
            mix,
            predelay_ms: 12.0,
            width: 1.0,
            mod_amount: 0.6,
            sr,
            lines: std::array::from_fn(|_| DelayLine::new(8)),
            len_s: [Smooth::new(60.0, sr); N],
            len_target: [0.0; N],
            absorb: [Absorb::default(); N],
            ap: std::array::from_fn(|_| std::array::from_fn(|_| Allpass::new(8, 2.0, 0.5))),
            ap_s: [[Smooth::new(60.0, sr); 4]; 2],
            ap_target: [[0.0; 4]; 2],
            pre_l: DelayLine::new(8),
            pre_r: DelayLine::new(8),
            pre_s: Smooth::new(60.0, sr),
            mods: std::array::from_fn(|i| Lfo::new(i as f64 / N as f64)),
            mod_s: Smooth::new(50.0, sr),
            width_s: Smooth::new(20.0, sr),
            mix_s: Smooth::new(20.0, sr),
            dc_l: DcBlock::new(5.0, sr),
            dc_r: DcBlock::new(5.0, sr),
            size_cached: f64::NAN,
            damping_cached: f64::NAN,
            damp_fc: 8000.0,
            damp_ratio: 0.5,
            rt: 1.8,
            decay_countdown: 0,
            quiet: 0,
            idle: true,
        };
        r.set_sample_rate(sr);
        r
    }

    /// Resize every buffer for `sr` and re-derive all coefficients.
    pub fn set_sample_rate(&mut self, sr: f64) {
        let sr = clamp_sr(sr);
        self.sr = sr;
        let ms = 0.001 * sr;
        let max_scale = 0.35 + 0.65 * SIZE_MAX;
        let line_frames = ((LINE_MS[N - 1] * max_scale + MOD_MS * 2.0) * ms) as usize + 64;
        self.lines = std::array::from_fn(|_| DelayLine::new(line_frames));
        let ap_frames = (14.0 * 1.3 * ms) as usize + 16;
        for (c, chain) in self.ap.iter_mut().enumerate() {
            for (k, ap) in chain.iter_mut().enumerate() {
                *ap = Allpass::new(ap_frames, 2.0, AP_G[k] * if c == 0 { 1.0 } else { 0.98 });
            }
        }
        let pre_frames = (PREDELAY_MAX_MS * ms) as usize + 8;
        self.pre_l = DelayLine::new(pre_frames);
        self.pre_r = DelayLine::new(pre_frames);
        // Length smoothers hold samples: rebuild them so they snap.
        self.len_s = [Smooth::new(60.0, sr); N];
        self.ap_s = [[Smooth::new(60.0, sr); 4]; 2];
        self.pre_s = Smooth::new(60.0, sr);
        for s in [&mut self.mod_s, &mut self.width_s, &mut self.mix_s] {
            s.set_sample_rate(sr);
        }
        self.dc_l = DcBlock::new(5.0, sr);
        self.dc_r = DcBlock::new(5.0, sr);
        self.size_cached = f64::NAN;
        self.retarget();
        self.reset();
    }

    /// Re-derive the damping (also done automatically when `tick` sees
    /// `damping` or `size` change).
    pub fn update_damping(&mut self) {
        self.retarget();
    }

    /// The decay time (seconds to −60 dB at low/mid frequencies) for the
    /// current `size`. High frequencies decay faster as `damping` rises.
    pub fn rt60(&self) -> f64 {
        rt60_for(self.size)
    }

    pub fn reset(&mut self) {
        for l in self.lines.iter_mut() {
            l.reset();
        }
        for a in self.ap.iter_mut().flatten() {
            a.reset();
        }
        for a in self.absorb.iter_mut() {
            a.x1 = 0.0;
            a.y1 = 0.0;
        }
        self.pre_l.reset();
        self.pre_r.reset();
        self.dc_l.reset();
        self.dc_r.reset();
        self.quiet = 0;
        self.idle = true;
    }

    fn retarget(&mut self) {
        let size = param(self.size, SIZE_MIN, SIZE_MAX, 0.5);
        let ms = 0.001 * self.sr;
        let scale = 0.35 + 0.65 * size;
        for i in 0..N {
            self.len_target[i] = next_prime((LINE_MS[i] * scale * ms).round() as usize) as f64;
        }
        let ds = (0.3 + 0.7 * size).min(1.3);
        for c in 0..2 {
            for k in 0..4 {
                self.ap_target[c][k] = (AP_MS[c][k] * ds * ms).max(2.0);
            }
        }
        let d = param(self.damping, 0.0, 1.0, 0.5);
        self.damp_fc = (10000.0 * (1200.0f64 / 10000.0).powf(d)).min(0.45 * self.sr);
        self.damp_ratio = 0.9 * (0.12f64 / 0.9).powf(d);
        self.rt = rt60_for(size);
        self.size_cached = self.size;
        self.damping_cached = self.damping;
        self.update_decay();
    }

    /// Per-line gains/shelves from the *current* (possibly gliding) lengths.
    fn update_decay(&mut self) {
        let k = (PI * self.damp_fc / self.sr).tan();
        for i in 0..N {
            let len = if self.len_s[i].value() > 0.0 { self.len_s[i].value() } else { self.len_target[i] };
            let secs = len / self.sr;
            let g0 = 10f64.powf(-3.0 * secs / self.rt);
            let r = 10f64.powf(-3.0 * secs / self.rt * (1.0 / self.damp_ratio - 1.0));
            let a = &mut self.absorb[i];
            a.g0 = g0;
            a.b0 = (k + r) / (k + 1.0);
            a.b1 = (k - r) / (k + 1.0);
            a.a1 = (k - 1.0) / (k + 1.0);
        }
    }

    #[inline]
    pub fn tick(&mut self, in_l: f32, in_r: f32) -> (f32, f32) {
        let xl = sanitize(in_l);
        let xr = sanitize(in_r);
        let silent_in = xl == 0.0 && xr == 0.0;
        if self.idle && silent_in {
            return (0.0, 0.0);
        }
        self.idle = false;
        if (self.size != self.size_cached || self.damping != self.damping_cached)
            && !(self.size.is_nan() || self.damping.is_nan())
        {
            self.retarget();
            self.decay_countdown = 1;
        }

        let ms = 0.001 * self.sr;

        // Pre-delay (write first, so 0 ms is a straight wire).
        self.pre_l.write(xl);
        self.pre_r.write(xr);
        let pre = self.pre_s.tick(param(self.predelay_ms, 0.0, PREDELAY_MAX_MS, 12.0) * ms);
        let mut dl = self.pre_l.read(pre);
        let mut dr = self.pre_r.read(pre);

        // Input diffusion.
        for k in 0..4 {
            let t = self.ap_s[0][k].tick(self.ap_target[0][k]);
            self.ap[0][k].set_delay(t);
            dl = self.ap[0][k].tick(dl);
            let t = self.ap_s[1][k].tick(self.ap_target[1][k]);
            self.ap[1][k].set_delay(t);
            dr = self.ap[1][k].tick(dr);
        }

        // Read the network (modulated, interpolated) and absorb.
        let depth = self.mod_s.tick(param(self.mod_amount, 0.0, 1.0, 0.6)) * MOD_MS * ms;
        let mut moving = false;
        let mut f = [0.0; N];
        for i in 0..N {
            let len = self.len_s[i].tick(self.len_target[i]);
            moving |= !self.len_s[i].is(self.len_target[i]);
            let m = self.mods[i].tick(MOD_HZ[i], self.sr) * depth;
            let v = self.lines[i].read((len + m - 1.0).max(1.0));
            f[i] = self.absorb[i].tick(v);
        }
        if moving || self.decay_countdown > 0 {
            // Re-derive gains for gliding lengths at a 32-sample control rate.
            if self.decay_countdown <= 1 {
                self.update_decay();
                self.decay_countdown = if moving { 32 } else { 0 };
            } else {
                self.decay_countdown -= 1;
            }
        }

        let mut wl = 0.0;
        let mut wr = 0.0;
        for i in 0..N {
            wl += TAP_L[i] * f[i];
            wr += TAP_R[i] * f[i];
        }
        wl *= OUT_GAIN;
        wr *= OUT_GAIN;

        hadamard(&mut f);
        let mut loudest = 0.0f64;
        for i in 0..N {
            let inj = if i % 2 == 0 { dl } else { dr };
            let w = flush(f[i] + IN_GAIN * INJ[i] * inj);
            loudest = loudest.max(w.abs());
            self.lines[i].write(w);
        }

        // Clear the network once it has been empty for a full cycle.
        if silent_in && loudest < QUIET && dl.abs() < QUIET && dr.abs() < QUIET {
            self.quiet += 1;
            let span = self.len_target.iter().fold(0.0f64, |a, &b| a.max(b))
                + self.pre_s.value()
                + self.ap_target.iter().flatten().sum::<f64>()
                + 64.0;
            if self.quiet as f64 > span {
                self.reset();
                return (0.0, 0.0);
            }
        } else {
            self.quiet = 0;
        }

        let wl = self.dc_l.tick(wl);
        let wr = self.dc_r.tick(wr);
        let w = self.width_s.tick(param(self.width, 0.0, 2.0, 1.0));
        let mid = 0.5 * (wl + wr);
        let side = 0.5 * (wl - wr) * w;
        let (dry, wet) = equal_power(self.mix_s.tick(param(self.mix, 0.0, 1.0, 0.2)));
        ((xl * dry + (mid + side) * wet) as f32, (xr * dry + (mid - side) * wet) as f32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Wet impulse response (mono energy per sample).
    fn ir(size: f64, damping: f64, sr: f64, secs: f64) -> Vec<f64> {
        let mut r = Reverb::new(size, damping, 1.0);
        r.set_sample_rate(sr);
        r.predelay_ms = 0.0;
        r.mod_amount = 0.0;
        let n = (secs * sr) as usize;
        (0..n)
            .map(|i| {
                let x = if i == 0 { 1.0 } else { 0.0 };
                let (l, rr) = r.tick(x, x);
                assert!(l.is_finite() && rr.is_finite());
                (l as f64).powi(2) + (rr as f64).powi(2)
            })
            .collect()
    }

    /// T30 from the Schroeder backward integral (−5 → −35 dB, ×2).
    fn measure_rt60(e: &[f64], sr: f64) -> f64 {
        let mut acc = 0.0;
        let mut sch = vec![0.0; e.len()];
        for i in (0..e.len()).rev() {
            acc += e[i];
            sch[i] = acc;
        }
        let total = sch[0];
        let db = |i: usize| 10.0 * (sch[i] / total).log10();
        let t5 = (0..e.len()).find(|&i| db(i) < -5.0).unwrap();
        let t35 = (0..e.len()).find(|&i| db(i) < -35.0).expect("decay did not reach -35 dB");
        (t35 - t5) as f64 / sr * 2.0
    }

    #[test]
    fn rt60_mapping_matches_size() {
        assert!((rt60_for(0.05) - 0.3).abs() < 1e-9);
        assert!((rt60_for(0.5) - 1.8).abs() < 1e-9);
        assert!((rt60_for(1.6) - 8.0).abs() < 1e-9);
        for (size, secs) in [(0.05, 1.0), (0.5, 3.0), (1.6, 9.0)] {
            let sr = 44100.0;
            // Measure on the low/mid decay (damping 0 keeps HF close to it).
            let m = measure_rt60(&ir(size, 0.0, sr, secs), sr);
            let target = rt60_for(size);
            assert!((m / target - 1.0).abs() < 0.25, "size {size}: measured {m:.3} s, target {target:.3} s");
        }
    }

    #[test]
    fn rt60_is_sample_rate_independent() {
        let a = measure_rt60(&ir(0.3, 0.3, 32000.0, 2.5), 32000.0);
        let b = measure_rt60(&ir(0.3, 0.3, 96000.0, 2.5), 96000.0);
        assert!((a / b - 1.0).abs() < 0.12, "{a} vs {b}");
    }

    #[test]
    fn damping_shortens_the_tail() {
        let sr = 44100.0;
        let bright = measure_rt60(&ir(0.5, 0.0, sr, 3.0), sr);
        let dark = measure_rt60(&ir(0.5, 1.0, sr, 3.0), sr);
        assert!(dark < bright, "{dark} vs {bright}");
    }

    #[test]
    fn silence_in_gives_exact_silence_out() {
        let mut r = Reverb::new(0.3, 0.5, 0.5);
        for _ in 0..1000 {
            assert_eq!(r.tick(0.0, 0.0), (0.0, 0.0));
        }
        for i in 0..4410 {
            let x = ((i as f64) * 0.07).sin() as f32;
            r.tick(x, -x);
        }
        let n = (r.rt60() * 4.0 * 44100.0) as usize;
        for _ in 0..n {
            r.tick(0.0, 0.0);
        }
        for _ in 0..100 {
            assert_eq!(r.tick(0.0, 0.0), (0.0, 0.0));
        }
    }

    #[test]
    fn wet_level_is_sane_and_size_automation_is_stable() {
        let mut r = Reverb::new(0.5, 0.5, 1.0);
        let mut seed = 7u32;
        let (mut ein, mut eout) = (0.0, 0.0);
        for i in 0..88200 {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let x = ((seed >> 8) as f32 / 8_388_608.0 - 1.0) * 0.3;
            if i % 4410 == 0 {
                r.size = if (i / 4410) % 2 == 0 { 1.4 } else { 0.1 };
            }
            let (l, rr) = r.tick(x, x);
            assert!(l.is_finite() && rr.is_finite() && l.abs() < 4.0);
            if i > 44100 {
                ein += (x as f64).powi(2) * 2.0;
                eout += (l as f64).powi(2) + (rr as f64).powi(2);
            }
        }
        let db = 10.0 * (eout / ein).log10();
        assert!(db > -12.0 && db < 9.0, "wet level {db:.1} dB");
    }
}
