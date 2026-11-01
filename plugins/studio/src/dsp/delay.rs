//! Delay-line primitives, the small shared helpers the effect files use
//! (parameter smoothing, DC blocking, one-pole filters, saturation), and the
//! time-based effects built on them: [`StereoDelay`], [`Chorus`], [`Phaser`].
//!
//! All state is `f64`; samples enter and leave as `f32`. Every public field is
//! a *target*: callers may write it at any time and `tick` glides towards it,
//! so automation never zippers or clicks.

use std::f64::consts::{PI, TAU};

/// Rate assumed by constructors that take no sample rate.
pub(crate) const DEFAULT_SR: f64 = 44100.0;

/// Recursive state below this is flushed to exact zero (no denormals, no
/// perpetual noise floor in decaying tails).
pub(crate) const DENORMAL: f64 = 1e-20;

/// Flush a tiny value to exact zero.
#[inline]
pub(crate) fn flush(x: f64) -> f64 {
    if x.abs() < DENORMAL {
        0.0
    } else {
        x
    }
}

/// Promote an input sample, replacing NaN/inf with silence.
#[inline]
pub(crate) fn sanitize(x: f32) -> f64 {
    if x.is_finite() {
        x as f64
    } else {
        0.0
    }
}

/// Guard a sample rate against nonsense (0, NaN, absurd values).
#[inline]
pub(crate) fn clamp_sr(sr: f64) -> f64 {
    if sr.is_finite() && sr > 0.0 {
        sr.clamp(8000.0, 384_000.0)
    } else {
        DEFAULT_SR
    }
}

/// Clamp a parameter, mapping NaN to `default`.
#[inline]
pub(crate) fn param(x: f64, lo: f64, hi: f64, default: f64) -> f64 {
    if x.is_finite() {
        x.clamp(lo, hi)
    } else {
        default
    }
}

/// Equal-power dry/wet gains for a mix in [0, 1]. The end points are exact.
#[inline]
pub(crate) fn equal_power(mix: f64) -> (f64, f64) {
    if mix <= 0.0 {
        (1.0, 0.0)
    } else if mix >= 1.0 {
        (0.0, 1.0)
    } else {
        let a = mix * PI * 0.5;
        (a.cos(), a.sin())
    }
}

/// Transparent below `t`, then a tanh knee that never exceeds 1.0 (C1 at `t`).
#[inline]
pub(crate) fn soft_limit(x: f64, t: f64) -> f64 {
    let a = x.abs();
    if a <= t {
        x
    } else {
        let k = 1.0 - t;
        (t + k * ((a - t) / k).tanh()).copysign(x)
    }
}

/// One-pole exponential parameter smoother. The first `tick` snaps to the
/// target, so freshly built processors start at their configured values.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Smooth {
    value: f64,
    coef: f64,
    tau_ms: f64,
    fresh: bool,
}

impl Smooth {
    pub(crate) fn new(tau_ms: f64, sr: f64) -> Self {
        let mut s = Self { value: 0.0, coef: 0.0, tau_ms, fresh: true };
        s.set_sample_rate(sr);
        s
    }

    pub(crate) fn set_sample_rate(&mut self, sr: f64) {
        self.coef = (-1.0 / (self.tau_ms.max(0.01) * 0.001 * clamp_sr(sr))).exp();
    }

    /// Advance one sample towards `target` and return the smoothed value.
    #[inline]
    pub(crate) fn tick(&mut self, target: f64) -> f64 {
        if self.fresh {
            self.fresh = false;
            self.value = target;
        } else if self.value != target {
            let v = target + (self.value - target) * self.coef;
            self.value = if (v - target).abs() <= 1e-9 * (1.0 + target.abs()) { target } else { v };
        }
        self.value
    }

    #[inline]
    pub(crate) fn value(&self) -> f64 {
        self.value
    }

    #[inline]
    pub(crate) fn is(&self, target: f64) -> bool {
        !self.fresh && self.value == target
    }
}

/// First-order DC blocker (`y = x - x[-1] + R·y[-1]`), cutoff in Hz.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DcBlock {
    r: f64,
    x1: f64,
    y1: f64,
    fc: f64,
}

impl DcBlock {
    pub(crate) fn new(fc: f64, sr: f64) -> Self {
        let mut d = Self { r: 0.0, x1: 0.0, y1: 0.0, fc };
        d.set_sample_rate(sr);
        d
    }

    pub(crate) fn set_sample_rate(&mut self, sr: f64) {
        self.r = (-TAU * self.fc / clamp_sr(sr)).exp();
    }

    pub(crate) fn reset(&mut self) {
        self.x1 = 0.0;
        self.y1 = 0.0;
    }

    #[inline]
    pub(crate) fn tick(&mut self, x: f64) -> f64 {
        let y = x - self.x1 + self.r * self.y1;
        self.x1 = x;
        self.y1 = flush(y);
        self.y1
    }
}

/// Detects "input has been exactly silent and the output has decayed below
/// −180 dBFS for a while", so a processor can clear its state and emit exact
/// zeros instead of an endless denormal-free but non-zero tail.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SilenceGate {
    n: usize,
    hold: usize,
}

impl SilenceGate {
    pub(crate) const FLOOR: f64 = 1e-9;

    pub(crate) fn new(sr: f64) -> Self {
        Self { n: 0, hold: (0.02 * clamp_sr(sr)) as usize }
    }

    /// Feed one sample's status; true once the gate may close.
    #[inline]
    pub(crate) fn quiet(&mut self, silent_in: bool, out_peak: f64) -> bool {
        if silent_in && out_peak < Self::FLOOR {
            self.n += 1;
            self.n > self.hold
        } else {
            self.n = 0;
            false
        }
    }

    pub(crate) fn reset(&mut self) {
        self.n = 0;
    }
}

/// One-pole low-pass `y += a·(x − y)`; the coefficient is set by the owner.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct OnePole {
    y: f64,
}

impl OnePole {
    /// Coefficient for a cutoff in Hz (matched-pole, stable up to Nyquist).
    pub(crate) fn coef(fc: f64, sr: f64) -> f64 {
        1.0 - (-TAU * fc.max(0.0) / clamp_sr(sr)).exp()
    }

    #[inline]
    pub(crate) fn lp(&mut self, x: f64, a: f64) -> f64 {
        self.y = flush(self.y + a * (x - self.y));
        self.y
    }

    /// Complementary high-pass (`x − lp(x)`).
    #[inline]
    pub(crate) fn hp(&mut self, x: f64, a: f64) -> f64 {
        x - self.lp(x, a)
    }

    pub(crate) fn reset(&mut self) {
        self.y = 0.0;
    }
}

/// A sine LFO driven by a phase accumulator (frequency changes are seamless).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Lfo {
    phase: f64,
}

impl Lfo {
    pub(crate) fn new(phase: f64) -> Self {
        Self { phase: phase.rem_euclid(1.0) }
    }

    /// Sine at the current phase, then advance by `hz`.
    #[inline]
    pub(crate) fn tick(&mut self, hz: f64, sr: f64) -> f64 {
        let v = (TAU * self.phase).sin();
        self.phase += hz / sr;
        if self.phase >= 1.0 {
            self.phase -= self.phase.floor();
        }
        v
    }
}

// ─────────────────────────── Delay line ───────────────────────────

/// A circular `f64` buffer with fractional reads.
///
/// Offsets are measured from the most recently written sample: `at(0)` is the
/// newest. In a feedback loop (read before write) an offset of `D − 1` gives a
/// loop delay of `D` samples; in a feed-forward tap (write before read) an
/// offset of `D` delays by `D`.
#[derive(Clone)]
pub struct DelayLine {
    buf: Vec<f64>,
    write: usize,
    mask: usize,
}

impl DelayLine {
    pub fn new(max_frames: usize) -> Self {
        let n = (max_frames + 4).next_power_of_two().max(8);
        Self { buf: vec![0.0; n], write: 0, mask: n - 1 }
    }

    /// Longest usable offset.
    pub fn max_offset(&self) -> f64 {
        (self.buf.len() - 4) as f64
    }

    pub fn reset(&mut self) {
        self.buf.iter_mut().for_each(|s| *s = 0.0);
        self.write = 0;
    }

    #[inline]
    pub fn write(&mut self, x: f64) {
        self.buf[self.write] = x;
        self.write = (self.write + 1) & self.mask;
    }

    /// The sample written `k` writes ago (0 = newest).
    #[inline]
    pub fn at(&self, k: usize) -> f64 {
        self.buf[self.write.wrapping_sub(1 + k) & self.mask]
    }

    /// Fractional read with 4-point cubic Hermite interpolation (linear for
    /// offsets below one sample, where no newer neighbour exists).
    #[inline]
    pub fn read(&self, offset: f64) -> f64 {
        let o = if offset.is_finite() { offset.clamp(0.0, self.max_offset()) } else { 0.0 };
        let i = o.floor();
        let f = o - i;
        let i = i as usize;
        if i == 0 {
            let a = self.at(0);
            return a + (self.at(1) - a) * f;
        }
        let xm1 = self.at(i - 1);
        let x0 = self.at(i);
        let x1 = self.at(i + 1);
        let x2 = self.at(i + 2);
        let c1 = 0.5 * (x1 - xm1);
        let c2 = xm1 - 2.5 * x0 + 2.0 * x1 - 0.5 * x2;
        let c3 = 0.5 * (x2 - xm1) + 1.5 * (x0 - x1);
        ((c3 * f + c2) * f + c1) * f + x0
    }
}

/// Schroeder all-pass with a fractional (interpolated) delay, used for the
/// reverb's input diffusion.
#[derive(Clone)]
pub struct Allpass {
    line: DelayLine,
    delay: f64,
    pub g: f64,
}

impl Allpass {
    /// `max_frames` sizes the buffer, `delay` is the initial delay in samples.
    pub fn new(max_frames: usize, delay: f64, g: f64) -> Self {
        let mut a = Self { line: DelayLine::new(max_frames + 2), delay: 2.0, g };
        a.set_delay(delay);
        a
    }

    pub fn set_delay(&mut self, delay: f64) {
        self.delay = delay.clamp(2.0, self.line.max_offset());
    }

    pub fn reset(&mut self) {
        self.line.reset();
    }

    #[inline]
    pub fn tick(&mut self, x: f64) -> f64 {
        let d = self.line.read(self.delay - 1.0);
        let v = flush(x + self.g * d);
        self.line.write(v);
        d - self.g * v
    }
}

// ─────────────────────────── Stereo delay ───────────────────────────

const DELAY_MAX_MS: f64 = 4000.0;
const OFFSET_MAX_MS: f64 = 500.0;
/// Peak wow excursion at `mod_depth = 1`, in ms.
const WOW_MS: f64 = 5.0;
/// Feedback-path high-pass (keeps repeats from piling up mud).
const FB_HP_HZ: f64 = 80.0;
/// Below this the delay counts its lines as empty.
const QUIET: f64 = 1e-8;

/// A stereo tape-style delay.
///
/// * Hermite-interpolated taps with smoothed delay time (automating `time_ms`
///   glides like a tape machine instead of clicking).
/// * Feedback path: damping low-pass, 80 Hz high-pass and a transparent-then-
///   soft limiter, so even feedback 0.95 settles instead of exploding.
/// * `ping_pong` cross-feeds the repeats and, at 1, injects the (mono) input
///   into the left line only so echoes bounce L → R → L.
/// * `offset_ms` delays the right channel further; `mod_depth` adds slow wow.
/// * `mix` is an equal-power dry/wet crossfade.
#[derive(Clone)]
pub struct StereoDelay {
    pub time_ms: f64,
    /// Extra delay on the right channel, in ms.
    pub offset_ms: f64,
    pub feedback: f64,
    pub mix: f64,
    /// 0 = two independent lines, 1 = full ping-pong.
    pub ping_pong: f64,
    /// 0 = bright repeats, 1 = dark repeats.
    pub damp: f64,
    /// Wow depth, 0..1.
    pub mod_depth: f64,
    sr: f64,
    l: DelayLine,
    r: DelayLine,
    time_s: Smooth,
    off_s: Smooth,
    fb_s: Smooth,
    mix_s: Smooth,
    pp_s: Smooth,
    mod_s: Smooth,
    damp_s: Smooth,
    damp_target: f64,
    damp_cached: f64,
    hp_coef: f64,
    lp_l: OnePole,
    lp_r: OnePole,
    hp_l: OnePole,
    hp_r: OnePole,
    wow: Lfo,
    drift: Lfo,
    quiet: usize,
    idle: bool,
}

impl StereoDelay {
    pub fn new(time_ms: f64, feedback: f64, mix: f64) -> Self {
        let sr = DEFAULT_SR;
        let mut d = Self {
            time_ms,
            offset_ms: 0.0,
            feedback,
            mix,
            ping_pong: 0.0,
            damp: 0.35,
            mod_depth: 0.15,
            sr,
            l: DelayLine::new(8),
            r: DelayLine::new(8),
            time_s: Smooth::new(90.0, sr),
            off_s: Smooth::new(90.0, sr),
            fb_s: Smooth::new(20.0, sr),
            mix_s: Smooth::new(20.0, sr),
            pp_s: Smooth::new(20.0, sr),
            mod_s: Smooth::new(50.0, sr),
            damp_s: Smooth::new(20.0, sr),
            damp_target: 0.0,
            damp_cached: f64::NAN,
            hp_coef: 0.0,
            lp_l: OnePole::default(),
            lp_r: OnePole::default(),
            hp_l: OnePole::default(),
            hp_r: OnePole::default(),
            wow: Lfo::new(0.0),
            drift: Lfo::new(0.37),
            quiet: 0,
            idle: true,
        };
        d.set_sample_rate(sr);
        d
    }

    /// Re-derive every coefficient and resize the lines for `sr`.
    pub fn set_sample_rate(&mut self, sr: f64) {
        let sr = clamp_sr(sr);
        self.sr = sr;
        let frames = ((DELAY_MAX_MS + OFFSET_MAX_MS + WOW_MS * 2.0) * 0.001 * sr).ceil() as usize + 16;
        self.l = DelayLine::new(frames);
        self.r = DelayLine::new(frames);
        for s in [
            &mut self.time_s,
            &mut self.off_s,
            &mut self.fb_s,
            &mut self.mix_s,
            &mut self.pp_s,
            &mut self.mod_s,
            &mut self.damp_s,
        ] {
            s.set_sample_rate(sr);
        }
        // Delay-time smoothers hold samples: rescale them with the rate.
        self.time_s = Smooth::new(90.0, sr);
        self.off_s = Smooth::new(90.0, sr);
        self.hp_coef = OnePole::coef(FB_HP_HZ, sr);
        self.damp_cached = f64::NAN;
        self.update_damping();
        self.reset();
    }

    /// Recompute the feedback low-pass from `damp` (also done automatically
    /// when `tick` sees `damp` change).
    pub fn update_damping(&mut self) {
        let d = param(self.damp, 0.0, 1.0, 0.35);
        // 0 → 16 kHz (open), 1 → 900 Hz (dark), exponential in between.
        let fc = (16000.0 * (900.0f64 / 16000.0).powf(d)).min(0.45 * self.sr);
        self.damp_target = OnePole::coef(fc, self.sr);
        self.damp_cached = self.damp;
    }

    pub fn reset(&mut self) {
        self.l.reset();
        self.r.reset();
        self.lp_l.reset();
        self.lp_r.reset();
        self.hp_l.reset();
        self.hp_r.reset();
        self.quiet = 0;
        self.idle = true;
    }

    /// Seconds until the repeats have decayed by 80 dB (capped at 20 s).
    pub fn tail_seconds(&self) -> f64 {
        let t = (param(self.time_ms, 1.0, DELAY_MAX_MS, 250.0) + param(self.offset_ms, 0.0, OFFSET_MAX_MS, 0.0))
            * 0.001;
        let fb = param(self.feedback, 0.0, 0.99, 0.0);
        if fb < 1e-3 {
            return (t + 0.01).min(20.0);
        }
        (t + t * (1e-4f64.ln() / fb.ln())).min(20.0)
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
        if self.damp != self.damp_cached && !(self.damp.is_nan() && self.damp_cached.is_nan()) {
            self.update_damping();
        }

        let ms = 0.001 * self.sr;
        let t = self.time_s.tick(param(self.time_ms, 1.0, DELAY_MAX_MS, 250.0) * ms);
        let o = self.off_s.tick(param(self.offset_ms, 0.0, OFFSET_MAX_MS, 0.0) * ms);
        let fb = self.fb_s.tick(param(self.feedback, 0.0, 0.99, 0.0));
        let c = self.pp_s.tick(param(self.ping_pong, 0.0, 1.0, 0.0));
        let depth = self.mod_s.tick(param(self.mod_depth, 0.0, 1.0, 0.0)) * WOW_MS * ms;
        let mix = self.mix_s.tick(param(self.mix, 0.0, 1.0, 0.0));
        let a = self.damp_s.tick(self.damp_target);

        // Slow wow (0.45 Hz) plus a slower drift, in quadrature between sides.
        let (pw, pd) = (self.wow.phase, self.drift.phase);
        self.wow.tick(0.45, self.sr);
        self.drift.tick(0.13, self.sr);
        let wl = 0.75 * (TAU * pw).sin() + 0.25 * (TAU * pd).sin();
        let wr = 0.75 * (TAU * (pw + 0.25)).sin() + 0.25 * (TAU * (pd + 0.25)).sin();
        let dl = (t + 0.5 * depth * wl).max(2.0);
        let dr = (t + o + 0.5 * depth * wr).max(2.0);

        // Read-before-write: offset D − 1 gives a loop of exactly D samples.
        let yl = self.l.read(dl - 1.0);
        let yr = self.r.read(dr - 1.0);

        let hp = self.hp_coef;
        let fl = self.hp_l.hp(self.lp_l.lp(yl, a), hp);
        let fr = self.hp_r.hp(self.lp_r.lp(yr, a), hp);
        let bl = fl * (1.0 - c) + fr * c;
        let br = fr * (1.0 - c) + fl * c;

        let mono = 0.5 * (xl + xr);
        let il = xl * (1.0 - c) + mono * c;
        let ir = xr * (1.0 - c);
        let w_l = flush(il + soft_limit(fb * bl, 0.8));
        let w_r = flush(ir + soft_limit(fb * br, 0.8));
        self.l.write(w_l);
        self.r.write(w_r);

        // Once the lines have been quiet for longer than the longest tap,
        // clear them so the tail ends in exact zeros.
        if silent_in && w_l.abs() < QUIET && w_r.abs() < QUIET && yl.abs() < QUIET && yr.abs() < QUIET {
            self.quiet += 1;
            if self.quiet as f64 > dl.max(dr) + 8.0 {
                self.reset();
                return (0.0, 0.0);
            }
        } else {
            self.quiet = 0;
        }

        let (dry, wet) = equal_power(mix);
        ((xl * dry + yl * wet) as f32, (xr * dry + yr * wet) as f32)
    }
}

// ─────────────────────────── Chorus ───────────────────────────

/// Base delays of the three voices, in ms.
const CHORUS_BASE_MS: [f64; 3] = [7.1, 10.3, 13.7];
/// Peak modulation excursion at `depth = 1`, in ms.
const CHORUS_DEPTH_MS: f64 = 3.0;

/// A three-voice stereo chorus/ensemble.
///
/// Each voice is a Hermite-interpolated tap swept by its own phase of one
/// sine LFO (voices 120° apart); `spread` offsets the right channel's phases
/// by up to half a cycle. `mix` is equal-power (0.5 ≈ the classic 50/50).
#[derive(Clone)]
pub struct Chorus {
    pub rate: f64,
    pub depth: f64,
    pub mix: f64,
    pub spread: f64,
    sr: f64,
    l: DelayLine,
    r: DelayLine,
    lfo: Lfo,
    depth_s: Smooth,
    mix_s: Smooth,
    spread_s: Smooth,
    rate_s: Smooth,
}

impl Chorus {
    pub fn new(rate: f64, depth: f64, mix: f64) -> Self {
        let sr = DEFAULT_SR;
        let mut c = Self {
            rate,
            depth,
            mix,
            spread: 0.6,
            sr,
            l: DelayLine::new(8),
            r: DelayLine::new(8),
            lfo: Lfo::new(0.0),
            depth_s: Smooth::new(30.0, sr),
            mix_s: Smooth::new(20.0, sr),
            spread_s: Smooth::new(30.0, sr),
            rate_s: Smooth::new(30.0, sr),
        };
        c.set_sample_rate(sr);
        c
    }

    pub fn set_sample_rate(&mut self, sr: f64) {
        let sr = clamp_sr(sr);
        self.sr = sr;
        let frames = ((CHORUS_BASE_MS[2] + CHORUS_DEPTH_MS + 2.0) * 0.001 * sr).ceil() as usize + 8;
        self.l = DelayLine::new(frames);
        self.r = DelayLine::new(frames);
        for s in [&mut self.depth_s, &mut self.mix_s, &mut self.spread_s, &mut self.rate_s] {
            s.set_sample_rate(sr);
        }
    }

    pub fn reset(&mut self) {
        self.l.reset();
        self.r.reset();
    }

    #[inline]
    pub fn tick(&mut self, in_l: f32, in_r: f32) -> (f32, f32) {
        let xl = sanitize(in_l);
        let xr = sanitize(in_r);
        self.l.write(xl);
        self.r.write(xr);

        let rate = self.rate_s.tick(param(self.rate, 0.01, 12.0, 0.6));
        let depth = self.depth_s.tick(param(self.depth, 0.0, 1.0, 0.5)) * CHORUS_DEPTH_MS;
        let spread = self.spread_s.tick(param(self.spread, 0.0, 1.0, 0.6)) * 0.5;
        let mix = self.mix_s.tick(param(self.mix, 0.0, 1.0, 0.4));
        let ms = 0.001 * self.sr;
        let ph = self.lfo.phase;
        self.lfo.tick(rate, self.sr);

        let mut wl = 0.0;
        let mut wr = 0.0;
        for (k, base) in CHORUS_BASE_MS.iter().enumerate() {
            let p = ph + k as f64 / 3.0;
            let ml = (TAU * p).sin();
            let mr = (TAU * (p + spread)).sin();
            wl += self.l.read((base + depth * ml) * ms);
            wr += self.r.read((base + depth * mr) * ms);
        }
        // Three partly-correlated voices: ~unity loudness against the dry.
        let norm = 0.45;
        let (dry, wet) = equal_power(mix);
        ((xl * dry + wl * norm * wet) as f32, (xr * dry + wr * norm * wet) as f32)
    }
}

// ─────────────────────────── Phaser ───────────────────────────

const PHASER_MAX_STAGES: usize = 6;
/// All-pass sections per stage (two first-order sections → one notch).
const AP_PER_STAGE: usize = 2;
const AP_N: usize = PHASER_MAX_STAGES * AP_PER_STAGE;
/// Sweep centre (geometric) and half-range in octaves at depth 1.
const PHASER_CENTRE_HZ: f64 = 800.0;
const PHASER_OCTAVES: f64 = 2.0;

/// A 1–6 stage phaser: each stage is two first-order all-passes, the sweep is
/// a sine LFO in log-frequency (200 Hz – 3.2 kHz at full depth), feedback up to
/// 0.95 is power-normalised and soft-limited, and the stage count crossfades
/// between taps of one 12-section chain so changing it never clicks. The right
/// channel's LFO runs 90° ahead. `mix` is linear (0.5 gives the deepest notches).
#[derive(Clone)]
pub struct Phaser {
    pub rate: f64,
    pub depth: f64,
    pub mix: f64,
    pub feedback: f64,
    pub stages: usize,
    sr: f64,
    ap_l: [f64; AP_N],
    ap_r: [f64; AP_N],
    fb_l: f64,
    fb_r: f64,
    lfo: Lfo,
    rate_s: Smooth,
    depth_s: Smooth,
    mix_s: Smooth,
    fb_s: Smooth,
    stages_s: Smooth,
}

impl Phaser {
    pub fn new(rate: f64, depth: f64, mix: f64) -> Self {
        let sr = DEFAULT_SR;
        Self {
            rate,
            depth,
            mix,
            feedback: 0.4,
            stages: 4,
            sr,
            ap_l: [0.0; AP_N],
            ap_r: [0.0; AP_N],
            fb_l: 0.0,
            fb_r: 0.0,
            lfo: Lfo::new(0.0),
            rate_s: Smooth::new(30.0, sr),
            depth_s: Smooth::new(30.0, sr),
            mix_s: Smooth::new(20.0, sr),
            fb_s: Smooth::new(20.0, sr),
            stages_s: Smooth::new(25.0, sr),
        }
    }

    pub fn set_sample_rate(&mut self, sr: f64) {
        let sr = clamp_sr(sr);
        self.sr = sr;
        for s in [&mut self.rate_s, &mut self.depth_s, &mut self.mix_s, &mut self.fb_s, &mut self.stages_s] {
            s.set_sample_rate(sr);
        }
        self.reset();
    }

    pub fn reset(&mut self) {
        self.ap_l = [0.0; AP_N];
        self.ap_r = [0.0; AP_N];
        self.fb_l = 0.0;
        self.fb_r = 0.0;
    }

    /// Run the full chain, returning the output after `n` sections for every
    /// `n` (taps[k] = after k+1 sections).
    #[inline]
    fn chain(state: &mut [f64; AP_N], x: f64, a: f64, taps: &mut [f64; AP_N]) {
        let mut v = x;
        for (s, t) in state.iter_mut().zip(taps.iter_mut()) {
            // H(z) = (a + z⁻¹) / (1 + a·z⁻¹)
            let y = a * v + *s;
            *s = flush(v - a * y);
            v = y;
            *t = y;
        }
    }

    #[inline]
    fn coef(&self, lfo: f64, depth: f64) -> f64 {
        let f = (PHASER_CENTRE_HZ * (PHASER_OCTAVES * depth * lfo).exp2()).clamp(20.0, 0.45 * self.sr);
        let t = (PI * f / self.sr).tan();
        (t - 1.0) / (t + 1.0)
    }

    #[inline]
    pub fn tick(&mut self, in_l: f32, in_r: f32) -> (f32, f32) {
        let xl = sanitize(in_l);
        let xr = sanitize(in_r);
        let rate = self.rate_s.tick(param(self.rate, 0.01, 8.0, 0.3));
        let depth = self.depth_s.tick(param(self.depth, 0.0, 1.0, 0.7));
        let mix = self.mix_s.tick(param(self.mix, 0.0, 1.0, 0.5));
        let fb = self.fb_s.tick(param(self.feedback, 0.0, 0.95, 0.4));
        let n = self.stages_s.tick(self.stages.clamp(1, PHASER_MAX_STAGES) as f64);

        let ph = self.lfo.phase;
        let ll = self.lfo.tick(rate, self.sr);
        let lr = (TAU * (ph + 0.25)).sin();
        let al = self.coef(ll, depth);
        let ar = self.coef(lr, depth);

        // Input scaled so average loop power stays ~constant with feedback.
        let g_in = (1.0 - fb * fb).sqrt();
        let mut taps = [0.0; AP_N];
        Self::chain(&mut self.ap_l, xl * g_in + soft_limit(fb * self.fb_l, 0.9), al, &mut taps);
        let wl = Self::tap(&taps, n);
        Self::chain(&mut self.ap_r, xr * g_in + soft_limit(fb * self.fb_r, 0.9), ar, &mut taps);
        let wr = Self::tap(&taps, n);
        self.fb_l = flush(wl);
        self.fb_r = flush(wr);

        ((xl * (1.0 - mix) + wl * mix) as f32, (xr * (1.0 - mix) + wr * mix) as f32)
    }

    /// Output after `n` (fractional while crossfading) stages.
    #[inline]
    fn tap(taps: &[f64; AP_N], n: f64) -> f64 {
        let lo = n.floor().clamp(1.0, PHASER_MAX_STAGES as f64);
        let f = (n - lo).clamp(0.0, 1.0);
        let i = lo as usize * AP_PER_STAGE - 1;
        if f == 0.0 || i + AP_PER_STAGE >= AP_N {
            taps[i]
        } else {
            taps[i] * (1.0 - f) + taps[i + AP_PER_STAGE] * f
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn impulse_run<F: FnMut(f32, f32) -> (f32, f32)>(mut f: F, n: usize) -> Vec<(f32, f32)> {
        (0..n).map(|i| if i == 0 { f(1.0, 1.0) } else { f(0.0, 0.0) }).collect()
    }

    #[test]
    fn hermite_reads_exact_on_integers_and_smooth_between() {
        let mut d = DelayLine::new(64);
        for i in 0..32 {
            d.write(i as f64);
        }
        assert_eq!(d.at(0), 31.0);
        assert!((d.read(3.0) - 28.0).abs() < 1e-12);
        assert!((d.read(3.5) - 27.5).abs() < 1e-12); // linear data stays linear
        assert!((d.read(0.25) - 30.75).abs() < 1e-12);
    }

    #[test]
    fn delay_echo_lands_on_the_right_sample_and_scales_with_rate() {
        for sr in [48000.0, 96000.0] {
            let mut d = StereoDelay::new(250.0, 0.0, 1.0);
            d.set_sample_rate(sr);
            d.mod_depth = 0.0;
            d.damp = 0.0;
            let n = (0.25 * sr) as usize;
            let out = impulse_run(|l, r| d.tick(l, r), n + 10);
            let peak = out.iter().enumerate().max_by(|a, b| a.1 .0.abs().total_cmp(&b.1 .0.abs())).unwrap().0;
            assert_eq!(peak, n, "echo at sr {sr}");
            assert!(out[n].0 > 0.9);
            assert!(out[..n].iter().all(|s| s.0.abs() < 1e-9));
        }
    }

    #[test]
    fn delay_offset_shifts_the_right_channel() {
        let mut d = StereoDelay::new(100.0, 0.0, 1.0);
        d.set_sample_rate(48000.0);
        d.mod_depth = 0.0;
        d.offset_ms = 20.0;
        let out = impulse_run(|l, r| d.tick(l, r), 6000);
        assert!(out[4800].0 > 0.9);
        assert!(out[4800 + 960].1 > 0.9);
    }

    #[test]
    fn delay_high_feedback_is_bounded_and_ends_in_silence() {
        let mut d = StereoDelay::new(30.0, 0.95, 0.5);
        d.ping_pong = 1.0;
        d.damp = 0.0;
        let mut peak = 0.0f32;
        // Loud noise burst into a hot loop.
        let mut seed = 1u32;
        for _ in 0..44100 {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let x = (seed >> 8) as f32 / 8_388_608.0 - 1.0;
            let (l, r) = d.tick(x, -x);
            assert!(l.is_finite() && r.is_finite());
            peak = peak.max(l.abs()).max(r.abs());
        }
        assert!(peak < 4.0, "peak {peak}");
        let tail = (d.tail_seconds() * 44100.0) as usize;
        assert!(d.tail_seconds() <= 20.0);
        for _ in 0..tail * 3 {
            let (l, r) = d.tick(0.0, 0.0);
            assert!(l.is_finite() && r.is_finite());
        }
        let (l, r) = d.tick(0.0, 0.0);
        assert_eq!((l, r), (0.0, 0.0), "exact silence after the tail");
    }

    #[test]
    fn delay_time_automation_does_not_click() {
        let mut d = StereoDelay::new(100.0, 0.5, 1.0);
        d.mod_depth = 0.0;
        let sr = 44100.0;
        let mut prev = 0.0f32;
        let mut max_step = 0.0f32;
        for i in 0..(sr as usize * 2) {
            if i == 30000 {
                d.time_ms = 400.0;
            }
            let x = (TAU * 220.0 * i as f64 / sr).sin() as f32 * 0.5;
            let (l, _) = d.tick(x, x);
            if i > 5000 {
                max_step = max_step.max((l - prev).abs());
            }
            prev = l;
        }
        // A 220 Hz sine at ≤ ~1.0 moves at most ~0.03/sample; a click would be ≫.
        assert!(max_step < 0.12, "max step {max_step}");
    }

    #[test]
    fn chorus_and_phaser_are_finite_and_silent_after_input() {
        let mut c = Chorus::new(0.6, 0.5, 0.5);
        c.set_sample_rate(48000.0);
        let mut p = Phaser::new(0.3, 0.7, 0.5);
        p.set_sample_rate(48000.0);
        p.feedback = 0.95;
        p.stages = 6;
        for i in 0..48000 {
            let x = if i < 24000 { ((i as f64 * 0.05).sin() * 0.8) as f32 } else { 0.0 };
            if i == 12000 {
                p.stages = 2;
            }
            let (a, b) = c.tick(x, x);
            let (e, f) = p.tick(x, x);
            assert!(a.is_finite() && b.is_finite() && e.is_finite() && f.is_finite());
            assert!(e.abs() < 8.0 && f.abs() < 8.0);
        }
        for _ in 0..96000 {
            c.tick(0.0, 0.0);
            p.tick(0.0, 0.0);
        }
        assert_eq!(c.tick(0.0, 0.0), (0.0, 0.0));
        assert_eq!(p.tick(0.0, 0.0), (0.0, 0.0));
    }

    #[test]
    fn mix_zero_is_dry() {
        let mut d = StereoDelay::new(10.0, 0.5, 0.0);
        let mut c = Chorus::new(1.0, 1.0, 0.0);
        for i in 0..2000 {
            let x = ((i as f64) * 0.01).sin() as f32;
            assert_eq!(d.tick(x, x).0, x);
            assert_eq!(c.tick(x, x).0, x);
        }
    }
}
