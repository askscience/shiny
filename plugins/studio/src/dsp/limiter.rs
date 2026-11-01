//! Dynamics: a soft-knee stereo compressor, a true look-ahead brickwall
//! limiter, and the master bus (gain, optional oversampled drive, width).

use super::delay::{clamp_sr, flush, param, sanitize, DcBlock, SilenceGate, Smooth, DEFAULT_SR};
use super::fx::Oversampler;
use super::util::db_to_gain;

#[inline]
fn ms_coef(ms: f64, sr: f64) -> f64 {
    (-1.0 / (ms.max(0.01) * 0.001 * sr)).exp()
}

// ─────────────────────────── Compressor ───────────────────────────

/// Feed-forward, stereo-linked compressor.
///
/// Peak level (max of |L|, |R|) is converted to dB, run through a soft-knee
/// gain computer, and the resulting gain reduction is smoothed in the log
/// domain with separate attack/release ballistics. `mix` blends the
/// compressed and dry signal in parallel (linear, they are phase-coherent).
#[derive(Clone)]
pub struct Compressor {
    pub threshold_db: f64,
    pub ratio: f64,
    pub attack_ms: f64,
    pub release_ms: f64,
    pub knee_db: f64,
    pub makeup_db: f64,
    pub mix: f64,
    /// Current gain reduction in dB (≤ 0), for metering.
    pub reduction_db: f64,
    sr: f64,
    env_db: f64,
    att: f64,
    rel: f64,
    cached: (f64, f64),
    makeup_s: Smooth,
    mix_s: Smooth,
}

impl Compressor {
    pub fn new(threshold_db: f64, ratio: f64, attack_ms: f64, release_ms: f64) -> Self {
        let sr = DEFAULT_SR;
        let mut c = Self {
            threshold_db,
            ratio,
            attack_ms,
            release_ms,
            knee_db: 6.0,
            makeup_db: 0.0,
            mix: 1.0,
            reduction_db: 0.0,
            sr,
            env_db: 0.0,
            att: 0.0,
            rel: 0.0,
            cached: (f64::NAN, f64::NAN),
            makeup_s: Smooth::new(20.0, sr),
            mix_s: Smooth::new(20.0, sr),
        };
        c.set_sample_rate(sr);
        c
    }

    pub fn set_sample_rate(&mut self, sr: f64) {
        self.sr = clamp_sr(sr);
        self.makeup_s.set_sample_rate(self.sr);
        self.mix_s.set_sample_rate(self.sr);
        self.recompute();
    }

    fn recompute(&mut self) {
        let a = param(self.attack_ms, 0.01, 2000.0, 10.0);
        let r = param(self.release_ms, 1.0, 10000.0, 150.0);
        self.att = ms_coef(a, self.sr);
        self.rel = ms_coef(r, self.sr);
        self.cached = (self.attack_ms, self.release_ms);
    }

    pub fn reset(&mut self) {
        self.env_db = 0.0;
        self.reduction_db = 0.0;
    }

    /// Static curve: gain change in dB (≤ 0) for an input level in dB.
    #[inline]
    fn gain_computer(&self, level_db: f64) -> f64 {
        let thr = param(self.threshold_db, -120.0, 24.0, -18.0);
        let slope = 1.0 / param(self.ratio, 1.0, 1000.0, 4.0) - 1.0;
        let knee = param(self.knee_db, 0.0, 48.0, 6.0);
        let over = level_db - thr;
        if knee > 1e-6 && 2.0 * over.abs() <= knee {
            let x = over + 0.5 * knee;
            slope * x * x / (2.0 * knee)
        } else if over > 0.0 {
            slope * over
        } else {
            0.0
        }
    }

    #[inline]
    pub fn tick(&mut self, l: f32, r: f32) -> (f32, f32) {
        let xl = sanitize(l);
        let xr = sanitize(r);
        if self.cached != (self.attack_ms, self.release_ms) {
            self.recompute();
        }
        let peak = xl.abs().max(xr.abs());
        let level_db = if peak > 1e-10 { 20.0 * peak.log10() } else { -200.0 };
        let target = self.gain_computer(level_db);
        // Log-domain branching smoother: attack while reduction deepens.
        let c = if target < self.env_db { self.att } else { self.rel };
        self.env_db = flush(target + (self.env_db - target) * c);
        self.reduction_db = self.env_db;

        let makeup = self.makeup_s.tick(param(self.makeup_db, -48.0, 48.0, 0.0));
        let mix = self.mix_s.tick(param(self.mix, 0.0, 1.0, 1.0));
        let db = self.env_db + makeup;
        let g = if db == 0.0 { 1.0 } else { 10f64.powf(db / 20.0) };
        let k = (1.0 - mix) + mix * g;
        ((xl * k) as f32, (xr * k) as f32)
    }
}

// ─────────────────────────── Limiter ───────────────────────────

/// Look-ahead time; the reported latency is this many samples.
const LOOKAHEAD_MS: f64 = 2.0;

/// True look-ahead brickwall limiter.
///
/// For every input sample the gain needed to keep it under the ceiling is
/// computed; a sliding-window minimum over the look-ahead window holds that
/// gain, a release smoother lets it recover, and a box filter of the same
/// length turns every gain drop into a linear ramp that *finishes* by the
/// time the peak leaves the delay line. Because the box average only ever
/// sees values ≤ the requirement of the sample being output, `|out| ≤
/// ceiling` holds by construction; a final clamp guards against rounding.
/// The audio is delayed by [`Limiter::latency_frames`] samples.
#[derive(Clone)]
pub struct Limiter {
    pub ceiling_db: f64,
    ceiling: f64,
    release_ms: f64,
    sr: f64,
    /// Look-ahead delay (= window − 1) in samples.
    la: usize,
    delay_l: Vec<f64>,
    delay_r: Vec<f64>,
    dpos: usize,
    /// Monotonic deque for the sliding minimum: (sample index, gain).
    dq_t: Vec<u64>,
    dq_v: Vec<f64>,
    dq_head: usize,
    dq_len: usize,
    n: u64,
    /// Box filter over the released hold gain.
    boxbuf: Vec<f64>,
    bpos: usize,
    bsum: f64,
    rel_gain: f64,
    rel_coef: f64,
    /// Current applied gain (≤ 1), for metering.
    pub gain: f64,
}

impl Limiter {
    pub fn new(ceiling_db: f64, release_ms: f64, sr: f64) -> Self {
        let mut l = Self {
            ceiling_db,
            ceiling: 1.0,
            release_ms,
            sr: clamp_sr(sr),
            la: 1,
            delay_l: Vec::new(),
            delay_r: Vec::new(),
            dpos: 0,
            dq_t: Vec::new(),
            dq_v: Vec::new(),
            dq_head: 0,
            dq_len: 0,
            n: 0,
            boxbuf: Vec::new(),
            bpos: 0,
            bsum: 0.0,
            rel_gain: 1.0,
            rel_coef: 0.0,
            gain: 1.0,
        };
        l.set_ceiling(ceiling_db);
        l.rebuild();
        l
    }

    fn rebuild(&mut self) {
        self.la = ((LOOKAHEAD_MS * 0.001 * self.sr).round() as usize).max(1);
        let w = self.la + 1;
        self.delay_l = vec![0.0; self.la];
        self.delay_r = vec![0.0; self.la];
        self.dq_t = vec![0; w + 1];
        self.dq_v = vec![1.0; w + 1];
        self.boxbuf = vec![1.0; w];
        self.rel_coef = ms_coef(param(self.release_ms, 1.0, 5000.0, 120.0), self.sr);
        self.reset();
    }

    pub fn set_sample_rate(&mut self, sr: f64) {
        self.sr = clamp_sr(sr);
        self.rebuild();
    }

    /// Change the output ceiling (dBFS, clamped to −60…0).
    pub fn set_ceiling(&mut self, ceiling_db: f64) {
        self.ceiling_db = param(ceiling_db, -60.0, 0.0, -0.4);
        self.ceiling = (db_to_gain(self.ceiling_db) as f64).min(1.0);
    }

    /// Delay the limiter adds to the signal, in samples.
    pub fn latency_frames(&self) -> usize {
        self.la
    }

    pub fn reset(&mut self) {
        self.delay_l.iter_mut().for_each(|s| *s = 0.0);
        self.delay_r.iter_mut().for_each(|s| *s = 0.0);
        self.boxbuf.iter_mut().for_each(|s| *s = 1.0);
        self.bsum = self.boxbuf.len() as f64;
        self.dpos = 0;
        self.bpos = 0;
        self.dq_head = 0;
        self.dq_len = 0;
        self.n = 0;
        self.rel_gain = 1.0;
        self.gain = 1.0;
    }

    /// Push a gain requirement and return the minimum over the window.
    #[inline]
    fn window_min(&mut self, v: f64) -> f64 {
        let cap = self.dq_t.len();
        let w = (self.la + 1) as u64;
        // Drop larger values from the back.
        while self.dq_len > 0 {
            let back = (self.dq_head + self.dq_len - 1) % cap;
            if self.dq_v[back] >= v {
                self.dq_len -= 1;
            } else {
                break;
            }
        }
        let slot = (self.dq_head + self.dq_len) % cap;
        self.dq_t[slot] = self.n;
        self.dq_v[slot] = v;
        self.dq_len += 1;
        // Drop expired entries from the front.
        while self.dq_t[self.dq_head] + w <= self.n {
            self.dq_head = (self.dq_head + 1) % cap;
            self.dq_len -= 1;
        }
        self.n += 1;
        self.dq_v[self.dq_head]
    }

    #[inline]
    pub fn tick(&mut self, l: f32, r: f32) -> (f32, f32) {
        let xl = sanitize(l);
        let xr = sanitize(r);
        let c = self.ceiling;
        let peak = xl.abs().max(xr.abs());
        let need = if peak > c { c / peak } else { 1.0 };

        let hold = self.window_min(need);
        // Release: instant down, exponential up (never above the hold).
        self.rel_gain = if hold < self.rel_gain { hold } else { hold + (self.rel_gain - hold) * self.rel_coef };
        // Box filter → linear attack ramp across the look-ahead.
        let w = self.boxbuf.len();
        self.bsum += self.rel_gain - self.boxbuf[self.bpos];
        self.boxbuf[self.bpos] = self.rel_gain;
        self.bpos += 1;
        if self.bpos == w {
            self.bpos = 0;
            // Re-sum once per window so rounding never drifts.
            self.bsum = self.boxbuf.iter().sum();
        }
        let g = (self.bsum / w as f64).min(1.0);
        self.gain = g;

        let dl = self.delay_l[self.dpos];
        let dr = self.delay_r[self.dpos];
        self.delay_l[self.dpos] = xl;
        self.delay_r[self.dpos] = xr;
        self.dpos = (self.dpos + 1) % self.la;

        (((dl * g).clamp(-c, c)) as f32, ((dr * g).clamp(-c, c)) as f32)
    }
}

// ─────────────────────────── Master bus ───────────────────────────

/// The master bus: gain, optional saturation, M/S width and a DC blocker.
///
/// `drive` (0..1) engages a soft saturator `tanh(a·x)/tanh(a)` with
/// `a = 3·drive` (unity at full scale, up to +9.5 dB of small-signal density).
/// It runs oversampled when the bus was built with `oversample > 1`, and is
/// crossfaded in/out so engaging it never clicks; at `drive = 0` the stage is
/// bypassed entirely.
#[derive(Clone)]
pub struct MasterBus {
    pub drive: f64,
    pub width: f64,
    pub gain: f64,
    sr: f64,
    factor: usize,
    os_l: Oversampler,
    os_r: Oversampler,
    dc_l: DcBlock,
    dc_r: DcBlock,
    gain_s: Smooth,
    drive_s: Smooth,
    width_s: Smooth,
    engage_s: Smooth,
    gate: SilenceGate,
    idle: bool,
}

impl Default for MasterBus {
    fn default() -> Self {
        Self::new(DEFAULT_SR, 1)
    }
}

impl MasterBus {
    /// `oversample` is rounded to 1, 2, 4 or 8.
    pub fn new(sr: f64, oversample: usize) -> Self {
        let sr = clamp_sr(sr);
        let factor = match oversample {
            0 | 1 => 1,
            2 | 3 => 2,
            4..=7 => 4,
            _ => 8,
        };
        Self {
            drive: 0.0,
            width: 1.0,
            gain: 1.0,
            sr,
            factor,
            os_l: Oversampler::new(factor),
            os_r: Oversampler::new(factor),
            dc_l: DcBlock::new(3.0, sr),
            dc_r: DcBlock::new(3.0, sr),
            gain_s: Smooth::new(20.0, sr),
            drive_s: Smooth::new(20.0, sr),
            width_s: Smooth::new(20.0, sr),
            engage_s: Smooth::new(5.0, sr),
            gate: SilenceGate::new(sr),
            idle: true,
        }
    }

    pub fn set_sample_rate(&mut self, sr: f64) {
        let sr = clamp_sr(sr);
        self.sr = sr;
        self.dc_l = DcBlock::new(3.0, sr);
        self.dc_r = DcBlock::new(3.0, sr);
        for s in [&mut self.gain_s, &mut self.drive_s, &mut self.width_s, &mut self.engage_s] {
            s.set_sample_rate(sr);
        }
        self.gate = SilenceGate::new(sr);
        self.reset();
    }

    pub fn reset(&mut self) {
        self.dc_l.reset();
        self.dc_r.reset();
        self.os_l.reset();
        self.os_r.reset();
        self.gate.reset();
        self.idle = true;
    }

    #[inline]
    fn sat(x: f64, a: f64) -> f64 {
        if a < 1e-4 {
            x
        } else {
            (a * x).tanh() / a.tanh()
        }
    }

    #[inline]
    pub fn tick(&mut self, l: f32, r: f32) -> (f32, f32) {
        let (xl, xr) = (sanitize(l), sanitize(r));
        let silent_in = xl == 0.0 && xr == 0.0;
        if self.idle && silent_in {
            return (0.0, 0.0);
        }
        self.idle = false;
        let g = self.gain_s.tick(param(self.gain, 0.0, 16.0, 1.0));
        let mut a = xl * g;
        let mut b = xr * g;

        let drive = param(self.drive, 0.0, 1.0, 0.0);
        let engage = self.engage_s.tick(if drive > 0.0 { 1.0 } else { 0.0 });
        let d = self.drive_s.tick(drive);
        if engage > 0.0 {
            let k = 3.0 * d;
            let (wa, wb) = if self.factor > 1 {
                let mut bl = [0.0; 8];
                let mut br = [0.0; 8];
                self.os_l.up(a, &mut bl);
                self.os_r.up(b, &mut br);
                for i in 0..self.factor {
                    bl[i] = Self::sat(bl[i], k);
                    br[i] = Self::sat(br[i], k);
                }
                (self.os_l.down(&bl), self.os_r.down(&br))
            } else {
                (Self::sat(a, k), Self::sat(b, k))
            };
            a += (wa - a) * engage;
            b += (wb - b) * engage;
        } else if self.factor > 1 {
            // Fully bypassed: start the oversampler clean next time.
            self.os_l.reset();
            self.os_r.reset();
        }

        let w = self.width_s.tick(param(self.width, 0.0, 2.0, 1.0));
        if w != 1.0 {
            let mid = 0.5 * (a + b);
            let side = 0.5 * (a - b) * w;
            a = mid + side;
            b = mid - side;
        }
        let (a, b) = (self.dc_l.tick(a), self.dc_r.tick(b));
        if self.gate.quiet(silent_in, a.abs().max(b.abs())) {
            self.reset();
            return (0.0, 0.0);
        }
        (a as f32, b as f32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::TAU;

    #[test]
    fn limiter_never_exceeds_the_ceiling() {
        for sr in [44100.0, 96000.0] {
            let mut lim = Limiter::new(-1.0, 120.0, sr);
            let c = db_to_gain(-1.0);
            let amp = db_to_gain(12.0) as f64;
            let n = (sr * 1.5) as usize;
            for i in 0..n {
                // Silence, then a +12 dB burst with an abrupt start, then silence.
                let on = i > n / 5 && i < n * 3 / 5;
                let x = if on { ((TAU * 997.0 * i as f64 / sr).sin() * amp) as f32 } else { 0.0 };
                let x2 = if i == n / 2 { 4.0 * amp as f32 } else { x }; // single-sample spike
                let (l, r) = lim.tick(x2, -x);
                assert!(l.abs() <= c && r.abs() <= c, "sr {sr} sample {i}: {l} {r} > {c}");
            }
        }
    }

    #[test]
    fn limiter_latency_matches_the_delay() {
        for sr in [44100.0, 48000.0, 192000.0] {
            let mut lim = Limiter::new(0.0, 100.0, sr);
            let la = lim.latency_frames();
            assert!(la as f64 >= 0.0015 * sr && la as f64 <= 0.005 * sr);
            let out: Vec<f32> = (0..la + 10).map(|i| lim.tick(if i == 0 { 0.5 } else { 0.0 }, 0.0).0).collect();
            let at = out.iter().position(|&s| s != 0.0).unwrap();
            assert_eq!(at, la);
            assert_eq!(out[la], 0.5, "below the ceiling the limiter is transparent");
        }
    }

    #[test]
    fn limiter_gain_is_smooth() {
        let sr = 48000.0;
        let mut lim = Limiter::new(-0.5, 80.0, sr);
        let mut prev_g = 1.0;
        let mut max_dg = 0.0f64;
        for i in 0..48000 {
            let x = ((TAU * 100.0 * i as f64 / sr).sin() * if i > 10000 { 3.0 } else { 0.3 }) as f32;
            lim.tick(x, x);
            max_dg = max_dg.max((lim.gain - prev_g).abs());
            prev_g = lim.gain;
        }
        // The attack ramp spans the whole look-ahead (≈96 samples here).
        assert!(max_dg < 0.02, "gain step {max_dg}");
    }

    #[test]
    fn compressor_reduces_loud_and_passes_quiet() {
        let sr = 44100.0;
        let mut c = Compressor::new(-20.0, 4.0, 5.0, 100.0);
        c.set_sample_rate(sr);
        c.knee_db = 0.0;
        let run = |c: &mut Compressor, amp: f64| {
            let mut peak = 0.0f64;
            for i in 0..22050 {
                let x = ((TAU * 440.0 * i as f64 / sr).sin() * amp) as f32;
                let (l, _) = c.tick(x, x);
                assert!(l.is_finite());
                if i > 11025 {
                    peak = peak.max(l.abs() as f64);
                }
            }
            peak
        };
        let quiet = run(&mut c, 0.01);
        assert!((quiet / 0.01 - 1.0).abs() < 1e-3);
        let loud = run(&mut c, 1.0);
        // 0 dBFS, threshold −20, ratio 4 → −15 dBFS (peak detector).
        let db = 20.0 * loud.log10();
        assert!((db + 15.0).abs() < 1.5, "out {db:.2} dB");
        for _ in 0..44100 {
            c.tick(0.0, 0.0);
        }
        assert_eq!(c.tick(0.0, 0.0), (0.0, 0.0));
    }

    #[test]
    fn master_bus_is_transparent_without_drive_and_bounded_with_it() {
        for os in [1, 2, 4, 8] {
            let mut bus = MasterBus::new(44100.0, os);
            let mut err = 0.0f64;
            for i in 0..4410 {
                let x = ((TAU * 1000.0 * i as f64 / 44100.0).sin() * 0.5) as f32;
                let (l, _) = bus.tick(x, x);
                if i > 2000 {
                    err = err.max((l - x).abs() as f64);
                }
            }
            assert!(err < 0.005, "os {os}: {err}");
            bus.drive = 1.0;
            for i in 0..44100 {
                let x = ((TAU * 100.0 * i as f64 / 44100.0).sin() * 1.0) as f32;
                let (l, r) = bus.tick(x, x);
                assert!(l.is_finite() && r.is_finite() && l.abs() < 1.1);
            }
            bus.drive = 0.0;
            for _ in 0..88200 {
                bus.tick(0.0, 0.0);
            }
            assert_eq!(bus.tick(0.0, 0.0), (0.0, 0.0));
        }
    }
}
