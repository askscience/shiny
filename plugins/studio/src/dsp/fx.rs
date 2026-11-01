//! Insert effects.
//!
//! Every effect is stereo-in / stereo-out so a voice's chain is a straight
//! line, and every parameter is a plain `f64` set by catalog key — which is
//! what lets both the automation lanes and the UI drive them uniformly. All
//! parameters that would click when changed abruptly are smoothed inside
//! `tick`, so callers may also write the public fields directly.
//!
//! This file also hosts the f64 biquad and the polyphase half-band
//! oversampler shared with the master bus.

use std::collections::HashMap;
use std::f64::consts::{PI, TAU};

use super::delay::{
    clamp_sr, flush, param, sanitize, Chorus, DcBlock, Phaser, SilenceGate, Smooth, StereoDelay, DEFAULT_SR,
};
use super::filter::{Filter, FilterKind};
use super::limiter::Compressor;
use super::reverb::Reverb;

/// The effect kinds the engine understands (mirrors `crate::fx::EFFECT_KINDS`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectKind {
    Distortion,
    Filter,
    Eq,
    Compressor,
    Delay,
    Reverb,
    Chorus,
    Phaser,
    Bitcrush,
}

impl EffectKind {
    pub fn from_name(kind: &str) -> Option<EffectKind> {
        Some(match kind {
            "distortion" | "drive" | "saturator" => EffectKind::Distortion,
            "filter" => EffectKind::Filter,
            "eq" => EffectKind::Eq,
            "compressor" | "comp" => EffectKind::Compressor,
            "delay" => EffectKind::Delay,
            "reverb" => EffectKind::Reverb,
            "chorus" | "ensemble" => EffectKind::Chorus,
            "phaser" => EffectKind::Phaser,
            "bitcrush" | "crush" => EffectKind::Bitcrush,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            EffectKind::Distortion => "distortion",
            EffectKind::Filter => "filter",
            EffectKind::Eq => "eq",
            EffectKind::Compressor => "compressor",
            EffectKind::Delay => "delay",
            EffectKind::Reverb => "reverb",
            EffectKind::Chorus => "chorus",
            EffectKind::Phaser => "phaser",
            EffectKind::Bitcrush => "bitcrush",
        }
    }
}

fn v(params: &HashMap<String, f64>, keys: &[&str], default: f64) -> f64 {
    for k in keys {
        if let Some(x) = params.get(*k) {
            if x.is_finite() {
                return *x;
            }
        }
    }
    default
}

// ─────────────────────────── Biquad (f64) ───────────────────────────

/// Transposed direct-form-II biquad with RBJ cookbook designs, all in f64.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    s1: f64,
    s2: f64,
}

impl Default for Biquad {
    fn default() -> Self {
        Self { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0, s1: 0.0, s2: 0.0 }
    }
}

impl Biquad {
    #[inline]
    fn w0(f: f64, sr: f64) -> (f64, f64) {
        let w = TAU * f.clamp(1.0, 0.45 * sr) / sr;
        (w.cos(), w.sin())
    }

    fn set(&mut self, b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64) {
        self.b0 = b0 / a0;
        self.b1 = b1 / a0;
        self.b2 = b2 / a0;
        self.a1 = a1 / a0;
        self.a2 = a2 / a0;
    }

    pub(crate) fn set_lowpass(&mut self, f: f64, q: f64, sr: f64) {
        let (c, s) = Self::w0(f, sr);
        let al = s / (2.0 * q);
        self.set((1.0 - c) * 0.5, 1.0 - c, (1.0 - c) * 0.5, 1.0 + al, -2.0 * c, 1.0 - al);
    }

    pub(crate) fn set_peak(&mut self, f: f64, gain_db: f64, q: f64, sr: f64) {
        let a = 10f64.powf(gain_db / 40.0);
        let (c, s) = Self::w0(f, sr);
        let al = s / (2.0 * q);
        self.set(1.0 + al * a, -2.0 * c, 1.0 - al * a, 1.0 + al / a, -2.0 * c, 1.0 - al / a);
    }

    pub(crate) fn set_low_shelf(&mut self, f: f64, gain_db: f64, sr: f64) {
        let a = 10f64.powf(gain_db / 40.0);
        let (c, s) = Self::w0(f, sr);
        let k = 2.0 * a.sqrt() * (s / std::f64::consts::SQRT_2); // S = 1
        self.set(
            a * ((a + 1.0) - (a - 1.0) * c + k),
            2.0 * a * ((a - 1.0) - (a + 1.0) * c),
            a * ((a + 1.0) - (a - 1.0) * c - k),
            (a + 1.0) + (a - 1.0) * c + k,
            -2.0 * ((a - 1.0) + (a + 1.0) * c),
            (a + 1.0) + (a - 1.0) * c - k,
        );
    }

    pub(crate) fn set_high_shelf(&mut self, f: f64, gain_db: f64, sr: f64) {
        let a = 10f64.powf(gain_db / 40.0);
        let (c, s) = Self::w0(f, sr);
        let k = 2.0 * a.sqrt() * (s / std::f64::consts::SQRT_2);
        self.set(
            a * ((a + 1.0) + (a - 1.0) * c + k),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * c),
            a * ((a + 1.0) + (a - 1.0) * c - k),
            (a + 1.0) - (a - 1.0) * c + k,
            2.0 * ((a - 1.0) - (a + 1.0) * c),
            (a + 1.0) - (a - 1.0) * c - k,
        );
    }

    pub(crate) fn reset(&mut self) {
        self.s1 = 0.0;
        self.s2 = 0.0;
    }

    #[inline]
    pub(crate) fn tick(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.s1;
        self.s1 = flush(self.b1 * x - self.a1 * y + self.s2);
        self.s2 = flush(self.b2 * x - self.a2 * y);
        y
    }
}

// ─────────────────────────── Oversampling ───────────────────────────

/// Coefficients of a polyphase half-band IIR (two parallel all-pass chains),
/// designed from an elliptic prototype (after L. de Soras' HIIR).
/// `transition` is the normalised transition bandwidth (0 < tbw < 0.5).
fn halfband_coefs(n: usize, transition: f64) -> Vec<f64> {
    let mut k = ((1.0 - transition * 2.0) * PI / 4.0).tan();
    k *= k;
    let kksqrt = (1.0 - k * k).powf(0.25);
    let e = 0.5 * (1.0 - kksqrt) / (1.0 + kksqrt);
    let e2 = e * e;
    let e4 = e2 * e2;
    let q = e * (1.0 + e4 * (2.0 + e4 * (15.0 + 150.0 * e4)));
    let order = (n * 2 + 1) as f64;
    (0..n)
        .map(|index| {
            let c = (index + 1) as f64;
            let mut num = 0.0;
            let mut i = 0i32;
            let mut sign = 1.0;
            loop {
                let term = q.powi(i * (i + 1)) * (((i * 2 + 1) as f64) * c * PI / order).sin() * sign;
                num += term;
                sign = -sign;
                i += 1;
                if term.abs() <= 1e-100 || i > 64 {
                    break;
                }
            }
            num *= q.powf(0.25);
            let mut den = 0.0;
            let mut i = 1i32;
            let mut sign = -1.0;
            loop {
                let term = q.powi(i * i) * ((i * 2) as f64 * c * PI / order).cos() * sign;
                den += term;
                sign = -sign;
                i += 1;
                if term.abs() <= 1e-100 || i > 64 {
                    break;
                }
            }
            den = den + 0.5;
            let ww = num / den;
            let wwsq = ww * ww;
            let x = ((1.0 - wwsq * k) * (1.0 - wwsq / k)).sqrt() / (1.0 + wwsq);
            (1.0 - x) / (1.0 + x)
        })
        .collect()
}

/// One 2× half-band stage (up and down share the design, not the state).
#[derive(Clone)]
struct Halfband {
    c: Vec<f64>,
    x: Vec<f64>,
    y: Vec<f64>,
}

impl Halfband {
    fn new(c: Vec<f64>) -> Self {
        let n = c.len();
        Self { c, x: vec![0.0; n], y: vec![0.0; n] }
    }

    fn reset(&mut self) {
        self.x.iter_mut().for_each(|s| *s = 0.0);
        self.y.iter_mut().for_each(|s| *s = 0.0);
    }

    /// Run the two all-pass chains: even coefficients on `a`, odd on `b`.
    #[inline]
    fn chains(&mut self, mut a: f64, mut b: f64) -> (f64, f64) {
        let n = self.c.len();
        let mut i = 0;
        while i < n {
            let t = (a - self.y[i]) * self.c[i] + self.x[i];
            self.x[i] = a;
            self.y[i] = flush(t);
            a = t;
            if i + 1 < n {
                let t = (b - self.y[i + 1]) * self.c[i + 1] + self.x[i + 1];
                self.x[i + 1] = b;
                self.y[i + 1] = flush(t);
                b = t;
            }
            i += 2;
        }
        (a, b)
    }

    /// One input sample → two output samples (in time order).
    #[inline]
    fn up(&mut self, x: f64) -> (f64, f64) {
        self.chains(x, x)
    }

    /// Two input samples (older, newer) → one output sample.
    #[inline]
    fn down(&mut self, older: f64, newer: f64) -> f64 {
        let (a, b) = self.chains(newer, older);
        0.5 * (a + b)
    }
}

/// 1×/2×/4×/8× oversampler built from cascaded polyphase half-band IIRs:
/// a steep first stage (≈ 0.02 transition, > 100 dB rejection) and relaxed
/// later stages, all in f64. Latency is a few samples of (non-linear) phase.
#[derive(Clone)]
pub(crate) struct Oversampler {
    factor: usize,
    up: Vec<Halfband>,
    down: Vec<Halfband>,
}

impl Oversampler {
    /// `factor` is rounded to 1, 2, 4 or 8.
    pub(crate) fn new(factor: usize) -> Self {
        let stages = match factor {
            0 | 1 => 0,
            2 | 3 => 1,
            4..=7 => 2,
            _ => 3,
        };
        let design = |s: usize| match s {
            0 => halfband_coefs(12, 0.02),
            1 => halfband_coefs(6, 0.1),
            _ => halfband_coefs(4, 0.2),
        };
        Self {
            factor: 1 << stages,
            up: (0..stages).map(|s| Halfband::new(design(s))).collect(),
            down: (0..stages).map(|s| Halfband::new(design(s))).collect(),
        }
    }

    pub(crate) fn reset(&mut self) {
        self.up.iter_mut().chain(self.down.iter_mut()).for_each(|h| h.reset());
    }

    /// Fill `out[..factor]` with the oversampled version of `x`.
    #[inline]
    pub(crate) fn up(&mut self, x: f64, out: &mut [f64; 8]) {
        out[0] = x;
        let mut len = 1;
        for st in self.up.iter_mut() {
            let mut tmp = [0.0; 8];
            for j in 0..len {
                let (a, b) = st.up(out[j]);
                tmp[2 * j] = a;
                tmp[2 * j + 1] = b;
            }
            len *= 2;
            out[..len].copy_from_slice(&tmp[..len]);
        }
    }

    /// Decimate `buf[..factor]` back to one sample.
    #[inline]
    pub(crate) fn down(&mut self, buf: &[f64; 8]) -> f64 {
        let mut b = *buf;
        let mut len = self.factor;
        for st in self.down.iter_mut().rev() {
            for j in 0..len / 2 {
                b[j] = st.down(b[2 * j], b[2 * j + 1]);
            }
            len /= 2;
        }
        b[0]
    }
}

// ─────────────────────────── Distortion ───────────────────────────

const DIST_OS: usize = 4;
/// Reference level for the automatic level compensation (≈ −9 dBFS peak).
const DIST_REF: f64 = 0.35;
const DIST_MODES: usize = 5;

/// Waveshaper with five characters, run at 4× through half-band filters.
///
/// Signal flow: DC block → (dry: same up/down filters, so dry and wet stay
/// phase-aligned) / (wet: 4× up → shaper × level compensation → 4× down →
/// `tone` low-pass → DC block) → linear `mix` → `out` gain.
///
/// Level compensation: for each mode/drive the RMS gain of the shaper on a
/// −9 dBFS sine is measured and inverted, so `drive` changes the character
/// (harmonics, compression) rather than the loudness.
#[derive(Clone)]
pub struct Distortion {
    /// 0 Soft (tanh), 1 Hard (clip), 2 Fold (sine folder), 3 Tube
    /// (asymmetric, even harmonics), 4 Fuzz (hard asymmetric).
    pub mode: f64,
    pub drive: f64,
    pub mix: f64,
    pub out: f64,
    /// Post low-pass cutoff in Hz.
    pub tone: f64,
    sr: f64,
    os_wet: [Oversampler; 2],
    os_dry: [Oversampler; 2],
    pre_dc: [DcBlock; 2],
    post_dc: [DcBlock; 2],
    tone_lp: [Biquad; 2],
    tone_s: Smooth,
    tone_applied: f64,
    drive_s: Smooth,
    mix_s: Smooth,
    out_s: Smooth,
    comp_s: Smooth,
    /// Mode crossfade: `from` → `to`, `fade` 0..1.
    mode_from: usize,
    mode_to: usize,
    fade: f64,
    fade_step: f64,
    comp_from: f64,
    comp_to: f64,
    cached_drive: f64,
    gate: SilenceGate,
    idle: bool,
}

#[inline]
fn shape(mode: usize, x: f64, d: f64) -> f64 {
    let u = x * d;
    match mode {
        1 => u.clamp(-1.0, 1.0),
        2 => (u * PI * 0.5).sin(),
        3 => {
            // Biased tanh: asymmetric → even harmonics (DC removed later).
            const B: f64 = 0.3;
            (u + B).tanh() - B.tanh()
        }
        4 => {
            // Fuzz: hard exponential knee, softer and lower on the negative side.
            let u = u * 2.0;
            if u >= 0.0 {
                1.0 - (-u).exp()
            } else {
                const K: f64 = 1.6;
                -(1.0 - (u * K).exp()) / K
            }
        }
        _ => u.tanh(),
    }
}

/// RMS gain correction for a mode/drive (measured on a DC-free sine).
fn level_comp(mode: usize, d: f64) -> f64 {
    const M: usize = 96;
    let mut ys = [0.0; M];
    let mut mean = 0.0;
    for (i, y) in ys.iter_mut().enumerate() {
        *y = shape(mode, DIST_REF * (TAU * i as f64 / M as f64).sin(), d);
        mean += *y;
    }
    mean /= M as f64;
    let rms = (ys.iter().map(|y| (y - mean) * (y - mean)).sum::<f64>() / M as f64).sqrt();
    let rms_in = DIST_REF / std::f64::consts::SQRT_2;
    if rms > 1e-9 {
        (rms_in / rms).clamp(0.05, 8.0)
    } else {
        1.0
    }
}

impl Distortion {
    pub fn new(params: &HashMap<String, f64>, sr: f64) -> Self {
        let sr = clamp_sr(sr);
        let mode = v(params, &["mode"], 0.0);
        let m = (mode.round().max(0.0) as usize).min(DIST_MODES - 1);
        let drive = v(params, &["drive"], 2.0);
        let mut d = Self {
            mode,
            drive,
            mix: v(params, &["mix"], 0.6),
            out: v(params, &["out"], 1.0),
            tone: v(params, &["tone"], 12000.0),
            sr,
            os_wet: [Oversampler::new(DIST_OS), Oversampler::new(DIST_OS)],
            os_dry: [Oversampler::new(DIST_OS), Oversampler::new(DIST_OS)],
            pre_dc: [DcBlock::new(5.0, sr); 2],
            post_dc: [DcBlock::new(5.0, sr); 2],
            tone_lp: [Biquad::default(); 2],
            tone_s: Smooth::new(20.0, sr),
            tone_applied: f64::NAN,
            drive_s: Smooth::new(20.0, sr),
            mix_s: Smooth::new(20.0, sr),
            out_s: Smooth::new(20.0, sr),
            comp_s: Smooth::new(20.0, sr),
            mode_from: m,
            mode_to: m,
            fade: 1.0,
            fade_step: 0.0,
            comp_from: 1.0,
            comp_to: 1.0,
            cached_drive: f64::NAN,
            gate: SilenceGate::new(sr),
            idle: true,
        };
        d.set_sample_rate(sr);
        d
    }

    pub fn set_sample_rate(&mut self, sr: f64) {
        let sr = clamp_sr(sr);
        self.sr = sr;
        self.pre_dc = [DcBlock::new(5.0, sr); 2];
        self.post_dc = [DcBlock::new(5.0, sr); 2];
        for s in [&mut self.tone_s, &mut self.drive_s, &mut self.mix_s, &mut self.out_s, &mut self.comp_s] {
            s.set_sample_rate(sr);
        }
        self.fade_step = 1.0 / (0.015 * sr);
        self.tone_applied = f64::NAN;
        self.gate = SilenceGate::new(sr);
        self.reset();
    }

    /// Kept for callers that set `tone` directly; `tick` follows it anyway.
    pub fn retune(&mut self) {
        self.tone_applied = f64::NAN;
    }

    pub fn reset(&mut self) {
        for i in 0..2 {
            self.os_wet[i].reset();
            self.os_dry[i].reset();
            self.pre_dc[i].reset();
            self.post_dc[i].reset();
            self.tone_lp[i].reset();
        }
        self.gate.reset();
        self.idle = true;
    }

    fn mode_index(&self) -> usize {
        if self.mode.is_finite() {
            (self.mode.round().max(0.0) as usize).min(DIST_MODES - 1)
        } else {
            0
        }
    }

    /// Track mode/drive changes: start a mode crossfade, refresh compensation.
    #[inline]
    fn follow_params(&mut self, drive: f64) {
        let m = self.mode_index();
        if m != self.mode_to {
            // Restart the fade from wherever we are.
            self.mode_from = self.mode_to;
            self.comp_from = self.comp_to;
            self.mode_to = m;
            self.fade = 0.0;
            self.cached_drive = f64::NAN;
        }
        if drive != self.cached_drive {
            self.cached_drive = drive;
            self.comp_to = level_comp(self.mode_to, drive);
            if self.fade >= 1.0 {
                self.comp_from = self.comp_to;
            }
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
        let drive_t = param(self.drive, 0.1, 64.0, 2.0);
        self.follow_params(drive_t);
        let d = self.drive_s.tick(drive_t);
        let comp = self.comp_s.tick(self.comp_to);
        let mix = self.mix_s.tick(param(self.mix, 0.0, 1.0, 0.6));
        let out = self.out_s.tick(param(self.out, 0.0, 8.0, 1.0));
        let tone = self.tone_s.tick(param(self.tone, 20.0, 24000.0, 12000.0).ln());
        if tone != self.tone_applied {
            self.tone_applied = tone;
            let f = tone.exp();
            for b in self.tone_lp.iter_mut() {
                b.set_lowpass(f, std::f64::consts::FRAC_1_SQRT_2, self.sr);
            }
        }
        let fading = self.fade < 1.0;
        let fade = self.fade;
        if fading {
            self.fade = (self.fade + self.fade_step).min(1.0);
        }

        let mut res = [0.0f64; 2];
        for (ch, x) in [xl, xr].into_iter().enumerate() {
            let x = self.pre_dc[ch].tick(x);
            let mut buf = [0.0; 8];
            self.os_dry[ch].up(x, &mut buf);
            let dry = self.os_dry[ch].down(&buf);
            self.os_wet[ch].up(x, &mut buf);
            for s in buf.iter_mut().take(DIST_OS) {
                let y = shape(self.mode_to, *s, d) * comp;
                *s = if fading { shape(self.mode_from, *s, d) * self.comp_from * (1.0 - fade) + y * fade } else { y };
            }
            let wet = self.os_wet[ch].down(&buf);
            let wet = self.post_dc[ch].tick(self.tone_lp[ch].tick(wet));
            res[ch] = (dry * (1.0 - mix) + wet * mix) * out;
        }
        if self.gate.quiet(silent_in, res[0].abs().max(res[1].abs())) {
            self.reset();
            return (0.0, 0.0);
        }
        (res[0] as f32, res[1] as f32)
    }
}

// ─────────────────────────── Filter ───────────────────────────

/// Which filter type an index selects (as `FilterKind::from_index` rounds it).
fn kind_slot(i: f64) -> i64 {
    if i.is_finite() {
        i.round().clamp(0.0, 5.0) as i64
    } else {
        0
    }
}

/// Stereo wrapper around the mono [`Filter`]: cutoff, resonance and drive are
/// smoothed; type/slope changes crossfade from the old filter pair.
pub struct FilterFx {
    cur: [Filter; 2],
    old: Option<[Filter; 2]>,
    fade: f64,
    fade_step: f64,
    kind: f64,
    poles: u8,
    cutoff: f64,
    q: f64,
    drive: f64,
    cut_s: Smooth,
    q_s: Smooth,
    drive_s: Smooth,
    applied: (f64, f64, f64),
    sr: f64,
    gate: SilenceGate,
    idle: bool,
}

impl FilterFx {
    pub fn new(params: &HashMap<String, f64>, sr: f64) -> Self {
        let sr = clamp_sr(sr);
        let kind = v(params, &["type", "ftype"], 0.0);
        let cutoff = v(params, &["cutoff"], 2000.0);
        let q = v(params, &["resonance", "res", "q"], 1.0);
        let drive = v(params, &["drive"], 1.0);
        let poles = v(params, &["poles"], 1.0).round().clamp(1.0, 2.0) as u8;
        let mut f = Self {
            cur: Self::make_pair(kind, cutoff, q, drive, poles, sr),
            old: None,
            fade: 1.0,
            fade_step: 1.0 / (0.02 * sr),
            kind,
            poles,
            cutoff,
            q,
            drive,
            cut_s: Smooth::new(15.0, sr),
            q_s: Smooth::new(15.0, sr),
            drive_s: Smooth::new(15.0, sr),
            applied: (f64::NAN, f64::NAN, f64::NAN),
            sr,
            gate: SilenceGate::new(sr),
            idle: false,
        };
        f.applied = (f.cut_target().ln(), f.q, f.drive);
        f
    }

    fn make_pair(kind: f64, cutoff: f64, q: f64, drive: f64, poles: u8, sr: f64) -> [Filter; 2] {
        let mk = || {
            let mut f = Filter::new(FilterKind::from_index(kind), cutoff.clamp(20.0, 0.45 * sr), q);
            f.set_sample_rate(sr);
            f.set_drive(drive);
            f.set_poles(poles);
            f
        };
        [mk(), mk()]
    }

    fn cut_target(&self) -> f64 {
        param(self.cutoff, 20.0, 20000.0, 2000.0).min(0.45 * self.sr)
    }

    pub fn set_sample_rate(&mut self, sr: f64) {
        let sr = clamp_sr(sr);
        self.sr = sr;
        for f in self.cur.iter_mut() {
            f.set_sample_rate(sr);
        }
        self.old = None;
        self.fade = 1.0;
        self.fade_step = 1.0 / (0.02 * sr);
        for s in [&mut self.cut_s, &mut self.q_s, &mut self.drive_s] {
            s.set_sample_rate(sr);
        }
        self.applied = (f64::NAN, f64::NAN, f64::NAN);
        self.gate = SilenceGate::new(sr);
    }

    fn switch_filters(&mut self) {
        let cut = if self.cut_s.value() > 0.0 { self.cut_s.value().exp() } else { self.cut_target() };
        let q = if self.q_s.value() > 0.0 { self.q_s.value() } else { self.q };
        let new = Self::make_pair(self.kind, cut, q, self.drive_s.value().max(0.0), self.poles, self.sr);
        self.old = Some(std::mem::replace(&mut self.cur, new));
        self.fade = 0.0;
    }

    pub fn set_param(&mut self, key: &str, val: f64) {
        if !val.is_finite() {
            return;
        }
        match key {
            "type" | "ftype" => {
                let changed = kind_slot(val) != kind_slot(self.kind);
                self.kind = val;
                if changed {
                    self.switch_filters();
                }
            }
            "cutoff" => self.cutoff = val,
            "resonance" | "res" | "q" => self.q = val,
            "drive" => self.drive = val,
            "poles" => {
                let p = val.round().clamp(1.0, 2.0) as u8;
                if p != self.poles {
                    self.poles = p;
                    self.switch_filters();
                }
            }
            _ => {}
        }
    }

    /// Ring-out time of the resonance (≈ T60 of the pole pair).
    pub fn tail_seconds(&self) -> f64 {
        let q = param(self.q, 0.1, 100.0, 1.0);
        (2.2 * q / self.cut_target() + 0.05).min(5.0)
    }

    pub fn reset(&mut self) {
        for f in self.cur.iter_mut() {
            f.reset();
        }
        self.old = None;
        self.fade = 1.0;
        self.gate.reset();
    }

    #[inline]
    pub fn tick(&mut self, l: f32, r: f32) -> (f32, f32) {
        let l = if l.is_finite() { l } else { 0.0 };
        let r = if r.is_finite() { r } else { 0.0 };
        let silent_in = l == 0.0 && r == 0.0;
        if self.idle && silent_in {
            return (0.0, 0.0);
        }
        self.idle = false;
        let (yl, yr) = self.process(l, r);
        if self.gate.quiet(silent_in, yl.abs().max(yr.abs())) {
            self.reset();
            self.idle = true;
            return (0.0, 0.0);
        }
        (yl as f32, yr as f32)
    }

    #[inline]
    fn process(&mut self, l: f32, r: f32) -> (f64, f64) {
        let c = self.cut_s.tick(self.cut_target().ln());
        let q = self.q_s.tick(param(self.q, 0.1, 40.0, 1.0));
        let d = self.drive_s.tick(param(self.drive, 0.0, 64.0, 1.0));
        if (c, q, d) != self.applied {
            let (pc, pq, pd) = self.applied;
            let hz = c.exp();
            let filters = self.cur.iter_mut().chain(self.old.iter_mut().flatten());
            for f in filters {
                if c != pc {
                    f.set_cutoff(hz);
                }
                if q != pq {
                    f.set_q(q);
                }
                if d != pd {
                    f.set_drive(d);
                }
            }
            self.applied = (c, q, d);
        }
        let yl = self.cur[0].tick(l) as f64;
        let yr = self.cur[1].tick(r) as f64;
        if let Some(old) = self.old.as_mut() {
            let ol = old[0].tick(l) as f64;
            let or = old[1].tick(r) as f64;
            let f = self.fade;
            self.fade += self.fade_step;
            if self.fade >= 1.0 {
                self.old = None;
                self.fade = 1.0;
            }
            return (ol + (yl - ol) * f, or + (yr - or) * f);
        }
        (yl, yr)
    }
}

// ─────────────────────────── EQ ───────────────────────────

/// Three-band EQ: low shelf (200 Hz), peak (1 kHz, Q 0.7), high shelf
/// (6 kHz), RBJ biquads in f64 with smoothed gains. A band at exactly 0 dB is
/// bypassed (bit-transparent).
#[derive(Clone)]
pub struct Eq3 {
    bands: [[Biquad; 2]; 3],
    gain: [f64; 3],
    freq: [f64; 3],
    gain_s: [Smooth; 3],
    applied: [f64; 3],
    sr: f64,
    gate: SilenceGate,
}

const EQ_Q: f64 = 0.7;

impl Eq3 {
    pub fn new(params: &HashMap<String, f64>, sr: f64) -> Self {
        let sr = clamp_sr(sr);
        Self {
            bands: [[Biquad::default(); 2]; 3],
            gain: [
                v(params, &["low_gain"], 0.0),
                v(params, &["mid_gain"], 0.0),
                v(params, &["hi_gain", "high_gain"], 0.0),
            ],
            freq: [v(params, &["low_freq"], 200.0), v(params, &["mid_freq"], 1000.0), v(params, &["hi_freq"], 6000.0)],
            gain_s: [Smooth::new(20.0, sr); 3],
            applied: [0.0; 3],
            sr,
            gate: SilenceGate::new(sr),
        }
    }

    pub fn set_sample_rate(&mut self, sr: f64) {
        self.sr = clamp_sr(sr);
        for s in self.gain_s.iter_mut() {
            s.set_sample_rate(self.sr);
        }
        self.applied = [f64::NAN; 3];
        self.gate = SilenceGate::new(self.sr);
        self.reset();
    }

    pub fn set_param(&mut self, key: &str, val: f64) {
        if !val.is_finite() {
            return;
        }
        match key {
            "low_gain" => self.gain[0] = val,
            "mid_gain" => self.gain[1] = val,
            "hi_gain" | "high_gain" => self.gain[2] = val,
            "low_freq" => {
                self.freq[0] = val;
                self.applied[0] = f64::NAN;
            }
            "mid_freq" => {
                self.freq[1] = val;
                self.applied[1] = f64::NAN;
            }
            "hi_freq" => {
                self.freq[2] = val;
                self.applied[2] = f64::NAN;
            }
            _ => {}
        }
    }

    pub fn reset(&mut self) {
        for b in self.bands.iter_mut().flatten() {
            b.reset();
        }
        self.gate.reset();
    }

    #[inline]
    pub fn tick(&mut self, l: f32, r: f32) -> (f32, f32) {
        let mut x = [sanitize(l), sanitize(r)];
        let silent_in = x[0] == 0.0 && x[1] == 0.0;
        for b in 0..3 {
            let g = self.gain_s[b].tick(self.gain[b].clamp(-30.0, 30.0));
            if g != self.applied[b] || self.applied[b].is_nan() {
                let f = self.freq[b].clamp(10.0, 0.45 * self.sr);
                if g == 0.0 {
                    // Entering bypass: an identity filter's state is ~0 anyway.
                    for q in self.bands[b].iter_mut() {
                        q.reset();
                    }
                }
                for q in self.bands[b].iter_mut() {
                    match b {
                        0 => q.set_low_shelf(f, g, self.sr),
                        1 => q.set_peak(f, g, EQ_Q, self.sr),
                        _ => q.set_high_shelf(f, g, self.sr),
                    }
                }
                self.applied[b] = g;
            }
            if g != 0.0 {
                x[0] = self.bands[b][0].tick(x[0]);
                x[1] = self.bands[b][1].tick(x[1]);
            }
        }
        if self.gate.quiet(silent_in, x[0].abs().max(x[1].abs())) {
            for q in self.bands.iter_mut().flatten() {
                q.reset();
            }
            return (0.0, 0.0);
        }
        (x[0] as f32, x[1] as f32)
    }
}

// ─────────────────────────── Compressor wrapper ───────────────────────────

#[derive(Clone)]
pub struct Comp {
    c: Compressor,
}

impl Comp {
    pub fn new(params: &HashMap<String, f64>, sr: f64) -> Self {
        let mut c = Compressor::new(
            v(params, &["threshold"], -18.0),
            v(params, &["ratio"], 4.0),
            v(params, &["attack"], 10.0),
            v(params, &["release"], 150.0),
        );
        c.set_sample_rate(sr);
        c.knee_db = v(params, &["knee"], 6.0);
        c.makeup_db = v(params, &["makeup"], 0.0);
        c.mix = v(params, &["mix"], 1.0);
        Self { c }
    }

    pub fn set_sample_rate(&mut self, sr: f64) {
        self.c.set_sample_rate(sr);
    }

    pub fn set_param(&mut self, key: &str, val: f64) {
        if !val.is_finite() {
            return;
        }
        match key {
            "threshold" => self.c.threshold_db = val,
            "ratio" => self.c.ratio = val,
            "attack" => self.c.attack_ms = val,
            "release" => self.c.release_ms = val,
            "makeup" => self.c.makeup_db = val,
            "knee" => self.c.knee_db = val,
            "mix" => self.c.mix = val,
            _ => {}
        }
    }

    /// Current gain reduction in dB (≤ 0).
    pub fn reduction_db(&self) -> f64 {
        self.c.reduction_db
    }

    pub fn reset(&mut self) {
        self.c.reset();
    }

    #[inline]
    pub fn tick(&mut self, l: f32, r: f32) -> (f32, f32) {
        self.c.tick(l, r)
    }
}

// ─────────────────────────── Bitcrush ───────────────────────────

/// Intentionally lo-fi: quantise to `bits` (fractional values glide between
/// depths) and sample-and-hold every `downsample` samples (no anti-aliasing,
/// that is the point). `mix` is linear and smoothed.
#[derive(Clone)]
pub struct Bitcrush {
    pub bits: f64,
    pub downsample: f64,
    pub mix: f64,
    phase: f64,
    hold_l: f64,
    hold_r: f64,
    mix_s: Smooth,
}

impl Bitcrush {
    pub fn new(params: &HashMap<String, f64>) -> Self {
        Self {
            bits: v(params, &["bits"], 8.0),
            downsample: v(params, &["downsample", "rate"], 1.0),
            mix: v(params, &["mix"], 1.0),
            phase: f64::MAX,
            hold_l: 0.0,
            hold_r: 0.0,
            mix_s: Smooth::new(20.0, DEFAULT_SR),
        }
    }

    pub fn set_sample_rate(&mut self, sr: f64) {
        self.mix_s.set_sample_rate(sr);
    }

    pub fn reset(&mut self) {
        // Start "due", so the first sample is captured immediately.
        self.phase = f64::MAX;
        self.hold_l = 0.0;
        self.hold_r = 0.0;
    }

    #[inline]
    pub fn tick(&mut self, l: f32, r: f32) -> (f32, f32) {
        let xl = sanitize(l);
        let xr = sanitize(r);
        let ds = param(self.downsample, 1.0, 64.0, 1.0);
        self.phase += 1.0;
        if self.phase >= ds {
            self.phase = if self.phase > 1e9 { 0.0 } else { self.phase - ds };
            let levels = 2f64.powf(param(self.bits, 1.0, 24.0, 8.0) - 1.0);
            self.hold_l = (xl * levels).round() / levels;
            self.hold_r = (xr * levels).round() / levels;
        }
        let mix = self.mix_s.tick(param(self.mix, 0.0, 1.0, 1.0));
        ((xl + (self.hold_l - xl) * mix) as f32, (xr + (self.hold_r - xr) * mix) as f32)
    }
}

// ─────────────────────────── The effect enum ───────────────────────────

pub enum Effect {
    Distortion(Distortion),
    Filter(FilterFx),
    Eq(Eq3),
    Compressor(Comp),
    Delay(StereoDelay),
    Reverb(Reverb),
    Chorus(Chorus),
    Phaser(Phaser),
    Bitcrush(Bitcrush),
}

impl Effect {
    pub fn new(kind: EffectKind, params: &HashMap<String, f64>, sr: f64) -> Effect {
        let mut e = match kind {
            EffectKind::Distortion => Effect::Distortion(Distortion::new(params, sr)),
            EffectKind::Filter => Effect::Filter(FilterFx::new(params, sr)),
            EffectKind::Eq => Effect::Eq(Eq3::new(params, sr)),
            EffectKind::Compressor => Effect::Compressor(Comp::new(params, sr)),
            EffectKind::Delay => {
                let mut d =
                    StereoDelay::new(v(params, &["time"], 250.0), v(params, &["feedback"], 0.4), v(params, &["mix"], 0.3));
                d.ping_pong = v(params, &["ping_pong"], 0.45);
                d.damp = v(params, &["damp", "damping"], 0.35);
                d.offset_ms = v(params, &["offset"], 0.0);
                d.mod_depth = v(params, &["mod"], 0.15);
                Effect::Delay(d)
            }
            EffectKind::Reverb => {
                let mut r =
                    Reverb::new(v(params, &["size"], 0.5), v(params, &["damping", "damp"], 0.5), v(params, &["mix"], 0.2));
                r.predelay_ms = v(params, &["predelay"], 12.0);
                r.width = v(params, &["width"], 1.0);
                r.mod_amount = v(params, &["mod"], 0.6);
                Effect::Reverb(r)
            }
            EffectKind::Chorus => {
                let mut c = Chorus::new(v(params, &["rate"], 0.6), v(params, &["depth"], 0.5), v(params, &["mix"], 0.4));
                c.spread = v(params, &["spread"], 0.6);
                Effect::Chorus(c)
            }
            EffectKind::Phaser => {
                let mut p = Phaser::new(v(params, &["rate"], 0.3), v(params, &["depth"], 0.7), v(params, &["mix"], 0.5));
                p.feedback = v(params, &["feedback"], 0.4);
                p.stages = v(params, &["stages"], 4.0).round().clamp(1.0, 6.0) as usize;
                Effect::Phaser(p)
            }
            EffectKind::Bitcrush => Effect::Bitcrush(Bitcrush::new(params)),
        };
        e.set_sample_rate(sr);
        e
    }

    pub fn kind_name(&self) -> &'static str {
        match self {
            Effect::Distortion(_) => "distortion",
            Effect::Filter(_) => "filter",
            Effect::Eq(_) => "eq",
            Effect::Compressor(_) => "compressor",
            Effect::Delay(_) => "delay",
            Effect::Reverb(_) => "reverb",
            Effect::Chorus(_) => "chorus",
            Effect::Phaser(_) => "phaser",
            Effect::Bitcrush(_) => "bitcrush",
        }
    }

    /// Re-derive all coefficients and buffers for a new rate (clears state).
    pub fn set_sample_rate(&mut self, sr: f64) {
        match self {
            Effect::Distortion(d) => d.set_sample_rate(sr),
            Effect::Filter(f) => f.set_sample_rate(sr),
            Effect::Eq(e) => e.set_sample_rate(sr),
            Effect::Compressor(c) => c.set_sample_rate(sr),
            Effect::Delay(d) => d.set_sample_rate(sr),
            Effect::Reverb(r) => r.set_sample_rate(sr),
            Effect::Chorus(c) => c.set_sample_rate(sr),
            Effect::Phaser(p) => p.set_sample_rate(sr),
            Effect::Bitcrush(b) => b.set_sample_rate(sr),
        }
    }

    pub fn set_param(&mut self, key: &str, val: f64) {
        if !val.is_finite() {
            return;
        }
        match self {
            Effect::Distortion(d) => match key {
                "mode" => d.mode = val,
                "drive" => d.drive = val,
                "mix" => d.mix = val,
                "out" => d.out = val,
                "tone" => d.tone = val,
                _ => {}
            },
            Effect::Filter(f) => f.set_param(key, val),
            Effect::Eq(e) => e.set_param(key, val),
            Effect::Compressor(c) => c.set_param(key, val),
            Effect::Delay(d) => match key {
                "time" | "delay_time" => d.time_ms = val,
                "feedback" => d.feedback = val,
                "mix" => d.mix = val,
                "ping_pong" => d.ping_pong = val,
                "damp" | "damping" => d.damp = val,
                "offset" => d.offset_ms = val,
                "mod" => d.mod_depth = val,
                _ => {}
            },
            Effect::Reverb(r) => match key {
                "size" => r.size = val,
                "damping" | "damp" => r.damping = val,
                "mix" => r.mix = val,
                "predelay" => r.predelay_ms = val,
                "width" => r.width = val,
                "mod" => r.mod_amount = val,
                _ => {}
            },
            Effect::Chorus(c) => match key {
                "rate" => c.rate = val,
                "depth" => c.depth = val,
                "mix" => c.mix = val,
                "spread" => c.spread = val,
                _ => {}
            },
            Effect::Phaser(p) => match key {
                "rate" => p.rate = val,
                "depth" => p.depth = val,
                "mix" => p.mix = val,
                "feedback" => p.feedback = val,
                "stages" => p.stages = val.round().clamp(1.0, 6.0) as usize,
                _ => {}
            },
            Effect::Bitcrush(b) => match key {
                "bits" => b.bits = val,
                "downsample" | "rate" => b.downsample = val,
                "mix" => b.mix = val,
                _ => {}
            },
        }
    }

    /// How long the effect keeps producing output after its input stops.
    pub fn tail_seconds(&self) -> f64 {
        match self {
            Effect::Delay(d) => d.tail_seconds(),
            Effect::Reverb(r) => (r.rt60() * 1.2 + param(r.predelay_ms, 0.0, 500.0, 0.0) * 0.001).min(30.0),
            Effect::Filter(f) => f.tail_seconds(),
            Effect::Phaser(_) => 0.1,
            _ => 0.05,
        }
    }

    /// Samples of delay the effect adds (only look-ahead processors report
    /// any; the oversampler's few samples of phase are not counted).
    pub fn latency_frames(&self) -> usize {
        0
    }

    pub fn reset(&mut self) {
        match self {
            Effect::Distortion(d) => d.reset(),
            Effect::Filter(f) => f.reset(),
            Effect::Eq(e) => e.reset(),
            Effect::Compressor(c) => c.reset(),
            Effect::Delay(d) => d.reset(),
            Effect::Reverb(r) => r.reset(),
            Effect::Chorus(c) => c.reset(),
            Effect::Phaser(p) => p.reset(),
            Effect::Bitcrush(b) => b.reset(),
        }
    }

    #[inline]
    pub fn tick(&mut self, l: f32, r: f32) -> (f32, f32) {
        match self {
            Effect::Distortion(d) => d.tick(l, r),
            Effect::Filter(f) => f.tick(l, r),
            Effect::Eq(e) => e.tick(l, r),
            Effect::Compressor(c) => c.tick(l, r),
            Effect::Delay(d) => d.tick(l, r),
            Effect::Reverb(rv) => rv.tick(l, r),
            Effect::Chorus(c) => c.tick(l, r),
            Effect::Phaser(p) => p.tick(l, r),
            Effect::Bitcrush(b) => b.tick(l, r),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(kv: &[(&str, f64)]) -> HashMap<String, f64> {
        kv.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    fn sine(i: usize, hz: f64, sr: f64, amp: f64) -> f32 {
        ((TAU * hz * i as f64 / sr).sin() * amp) as f32
    }

    fn rms(x: &[f64]) -> f64 {
        (x.iter().map(|v| v * v).sum::<f64>() / x.len() as f64).sqrt()
    }

    /// Amplitude of the `hz` component (single-bin DFT).
    fn tone_amp(x: &[f64], hz: f64, sr: f64) -> f64 {
        let (mut re, mut im) = (0.0, 0.0);
        for (i, v) in x.iter().enumerate() {
            let p = TAU * hz * i as f64 / sr;
            re += v * p.cos();
            im += v * p.sin();
        }
        2.0 * (re * re + im * im).sqrt() / x.len() as f64
    }

    #[test]
    fn oversampler_passband_and_image_rejection() {
        for factor in [2, 4, 8] {
            let mut os = Oversampler::new(factor);
            let sr = 48000.0;
            // In-band tone survives up/down with unity gain.
            let mut out = Vec::new();
            for i in 0..9600 {
                let mut b = [0.0; 8];
                os.up(sine(i, 1000.0, sr, 0.5) as f64, &mut b);
                out.push(os.down(&b));
            }
            let a = tone_amp(&out[4800..], 1000.0, sr);
            assert!((a / 0.5 - 1.0).abs() < 1e-3, "factor {factor}: {a}");
            // Upsampling must not leave images: energy above the base Nyquist.
            let mut up = Vec::new();
            for i in 0..4800 {
                let mut b = [0.0; 8];
                os.up(sine(i, 5000.0, sr, 0.5) as f64, &mut b);
                up.extend_from_slice(&b[..factor]);
            }
            let osr = sr * factor as f64;
            let image = tone_amp(&up[up.len() / 2..], sr - 5000.0, osr);
            assert!(image < 0.5 * 1e-4, "factor {factor}: image {image}");
        }
        // Decimation rejects content just above the base Nyquist (stage 1 band).
        let mut os = Oversampler::new(2);
        let sr = 48000.0;
        let mut out = Vec::new();
        for i in 0..4800 {
            let b0 = sine(2 * i, 27000.0, 2.0 * sr, 0.5) as f64;
            let b1 = sine(2 * i + 1, 27000.0, 2.0 * sr, 0.5) as f64;
            let b = [b0, b1, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
            out.push(os.down(&b));
        }
        let alias = tone_amp(&out[2400..], sr - 27000.0, sr);
        assert!(alias < 0.5 * 1e-4, "alias {alias}");
    }

    #[test]
    fn distortion_is_transparent_at_mix_zero_and_low_drive() {
        let sr = 44100.0;
        for (drive, mix) in [(1.0, 0.0), (0.25, 1.0)] {
            let mut d = Distortion::new(&params(&[("drive", drive), ("mix", mix), ("tone", 20000.0)]), sr);
            let mut out = Vec::new();
            let mut inp = Vec::new();
            for i in 0..8820 {
                let x = sine(i, 440.0, sr, 0.25);
                let (l, r) = d.tick(x, x);
                assert_eq!(l, r);
                out.push(l as f64);
                inp.push(x as f64);
            }
            let g = tone_amp(&out[4410..], 440.0, sr) / 0.25;
            assert!((g - 1.0).abs() < 0.02, "drive {drive} mix {mix}: gain {g}");
            // Harmonic content stays tiny.
            let h3 = tone_amp(&out[4410..], 1320.0, sr);
            assert!(h3 < 0.25 * 0.01, "h3 {h3}");
        }
    }

    #[test]
    fn distortion_compensates_level_and_stays_finite() {
        let sr = 48000.0;
        for mode in 0..5 {
            let mut levels = Vec::new();
            for drive in [1.0, 4.0, 16.0] {
                let mut d = Distortion::new(&params(&[("mode", mode as f64), ("drive", drive), ("mix", 1.0)]), sr);
                let mut out = Vec::new();
                for i in 0..9600 {
                    let (l, _) = d.tick(sine(i, 220.0, sr, 0.25), 0.0);
                    assert!(l.is_finite());
                    out.push(l as f64);
                }
                levels.push(rms(&out[4800..]));
            }
            let spread = levels.iter().cloned().fold(0.0, f64::max) / levels.iter().cloned().fold(9.0, f64::min);
            assert!(spread < 1.5, "mode {mode}: levels {levels:?}");
            // Mode change mid-stream stays bounded.
        }
        let mut d = Distortion::new(&params(&[("drive", 8.0), ("mix", 1.0)]), sr);
        let mut prev = 0.0f32;
        let mut step = 0.0f32;
        for i in 0..9600 {
            if i == 4800 {
                d.mode = 2.0;
                d.drive = 2.0;
            }
            let (l, _) = d.tick(sine(i, 110.0, sr, 0.25), 0.0);
            if i > 100 {
                step = step.max((l - prev).abs());
            }
            prev = l;
        }
        assert!(step < 0.2, "mode switch step {step}");
        for _ in 0..96000 {
            d.tick(0.0, 0.0);
        }
        assert_eq!(d.tick(0.0, 0.0), (0.0, 0.0));
    }

    #[test]
    fn eq_is_bit_transparent_flat_and_boosts_when_asked() {
        let sr = 48000.0;
        let mut e = Eq3::new(&HashMap::new(), sr);
        for i in 0..1000 {
            let x = sine(i, 300.0, sr, 0.5);
            assert_eq!(e.tick(x, -x), (x, -x));
        }
        e.set_param("mid_gain", 12.0);
        let mut out = Vec::new();
        for i in 0..9600 {
            out.push(e.tick(sine(i, 1000.0, sr, 0.1), 0.0).0 as f64);
        }
        let g = 20.0 * (tone_amp(&out[4800..], 1000.0, sr) / 0.1).log10();
        assert!((g - 12.0).abs() < 0.3, "peak gain {g}");
        e.set_param("mid_gain", 0.0);
        e.set_param("low_gain", -12.0);
        let mut out = Vec::new();
        for i in 0..19200 {
            out.push(e.tick(sine(i, 40.0, sr, 0.1), 0.0).0 as f64);
        }
        let g = 20.0 * (tone_amp(&out[9600..], 40.0, sr) / 0.1).log10();
        assert!((g + 12.0).abs() < 0.6, "shelf gain {g}");
    }

    #[test]
    fn every_effect_is_finite_and_silent_after_its_tail() {
        let kinds = ["distortion", "filter", "eq", "compressor", "delay", "reverb", "chorus", "phaser", "bitcrush"];
        for sr in [8000.0, 44100.0, 96000.0] {
            for k in kinds {
                let kind = EffectKind::from_name(k).unwrap();
                let mut p = HashMap::new();
                p.insert("feedback".to_string(), 0.9);
                p.insert("mix".to_string(), 0.5);
                p.insert("size".to_string(), 0.2);
                p.insert("low_gain".to_string(), 6.0);
                let mut e = Effect::new(kind, &p, sr);
                assert_eq!(e.kind_name(), kind.name());
                assert_eq!(e.latency_frames(), 0);
                for _ in 0..64 {
                    assert_eq!(e.tick(0.0, 0.0), (0.0, 0.0), "{k}: silence in");
                }
                let n = (sr * 0.1) as usize;
                for i in 0..n {
                    let x = if i == 0 { 1.0 } else { sine(i, 330.0, sr, 0.7) };
                    let (l, r) = e.tick(x, x * 0.5);
                    assert!(l.is_finite() && r.is_finite(), "{k}");
                }
                e.set_param("mix", 0.8);
                assert!(e.tail_seconds() > 0.0);
                let tail = ((e.tail_seconds() * 3.0 + 0.5) * sr) as usize;
                for _ in 0..tail {
                    let (l, r) = e.tick(0.0, 0.0);
                    assert!(l.is_finite() && r.is_finite(), "{k}");
                }
                for _ in 0..16 {
                    assert_eq!(e.tick(0.0, 0.0), (0.0, 0.0), "{k} at {sr}: not silent after tail");
                }
                // NaN input never poisons the state.
                e.tick(f32::NAN, f32::INFINITY);
                let (l, r) = e.tick(0.1, 0.1);
                assert!(l.is_finite() && r.is_finite(), "{k}");
            }
        }
    }

    #[test]
    fn delay_tail_and_bitcrush() {
        let p = params(&[("time", 500.0), ("feedback", 0.5)]);
        let e = Effect::new(EffectKind::Delay, &p, 44100.0);
        let t = e.tail_seconds();
        assert!((t - (0.5 + 0.5 * (1e-4f64.ln() / 0.5f64.ln()))).abs() < 1e-9);
        let mut b = Bitcrush::new(&params(&[("bits", 2.0), ("downsample", 4.0), ("mix", 1.0)]));
        b.reset();
        let outs: Vec<f32> = (0..8).map(|i| b.tick(0.3 + i as f32 * 0.01, 0.0).0).collect();
        assert!(outs[..4].iter().all(|&o| o == outs[0]));
        assert_eq!(outs[0], 0.5);
    }
}
