//! Filters: a zero-delay-feedback (TPT) state-variable filter with 12 or
//! 24 dB/oct slopes and input drive, and a ZDF four-pole "Moog-style" ladder.
//!
//! Both are unconditionally stable at any cutoff/resonance, keep their state
//! in `f64`, and process **stereo with independent per-channel state** while
//! sharing one set of coefficients (the old engine ran L and R through one
//! state, which turned every stereo voice into a Nyquist-rate rattle).
//! Cutoff changes are cheap (`tan` once per sample, shared by both channels),
//! so per-sample envelope/LFO modulation is exact rather than stepped.

use super::util::{flush, soft_clip};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterKind {
    LowPass,
    HighPass,
    BandPass,
    Notch,
    Peak,
    /// Four-pole resonant ladder low-pass.
    Ladder,
}

impl FilterKind {
    pub fn from_index(i: f64) -> FilterKind {
        match i.round() as i64 {
            1 => FilterKind::HighPass,
            2 => FilterKind::BandPass,
            3 => FilterKind::Notch,
            4 => FilterKind::Peak,
            5 => FilterKind::Ladder,
            _ => FilterKind::LowPass,
        }
    }

    pub fn index(self) -> f64 {
        match self {
            FilterKind::LowPass => 0.0,
            FilterKind::HighPass => 1.0,
            FilterKind::BandPass => 2.0,
            FilterKind::Notch => 3.0,
            FilterKind::Peak => 4.0,
            FilterKind::Ladder => 5.0,
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Svf {
    ic1: f64,
    ic2: f64,
}

impl Svf {
    #[inline]
    fn tick(&mut self, x: f64, c: &Coefs, kind: FilterKind) -> f64 {
        let v3 = x - self.ic2;
        let v1 = c.a1 * self.ic1 + c.a2 * v3;
        let v2 = self.ic2 + c.a2 * self.ic1 + c.a3 * v3;
        self.ic1 = flush(2.0 * v1 - self.ic1);
        self.ic2 = flush(2.0 * v2 - self.ic2);
        let k = c.k;
        match kind {
            FilterKind::LowPass | FilterKind::Ladder => v2,
            FilterKind::HighPass => x - k * v1 - v2,
            // Normalised band-pass: unity gain at the centre for any Q.
            FilterKind::BandPass => k * v1,
            FilterKind::Notch => x - k * v1,
            // Low minus high: a resonant peak at the cutoff, flat elsewhere.
            FilterKind::Peak => 2.0 * v2 - x + k * v1,
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Ladder {
    s: [f64; 4],
}

impl Ladder {
    #[inline]
    fn tick(&mut self, x: f64, c: &Coefs) -> f64 {
        let g = c.g;
        let big_g = g / (1.0 + g);
        let inv = 1.0 / (1.0 + g);
        // Zero-delay feedback: solve the loop for the 4th-stage output.
        let s = &self.s;
        let sigma = big_g * big_g * big_g * s[0] * inv
            + big_g * big_g * s[1] * inv
            + big_g * s[2] * inv
            + s[3] * inv;
        let g4 = big_g * big_g * big_g * big_g;
        let k = c.ladder_k;
        let y4_est = (g4 * x + sigma) / (1.0 + k * g4);
        // Saturate the loop input (the "Moog" character), then run the four
        // one-pole stages with the solved input.
        let mut u = soft_clip_f64(x - k * y4_est);
        for st in self.s.iter_mut() {
            let v = (u - *st) * big_g;
            let y = v + *st;
            *st = flush(y + v);
            u = y;
        }
        // Compensate the passband loss that resonance causes in a ladder.
        u * (1.0 + 0.5 * k)
    }
}

#[inline]
fn soft_clip_f64(x: f64) -> f64 {
    soft_clip(x as f32) as f64
}

#[derive(Clone, Copy, Default)]
struct Coefs {
    g: f64,
    k: f64,
    a1: f64,
    a2: f64,
    a3: f64,
    ladder_k: f64,
}

#[derive(Clone)]
pub struct Filter {
    pub kind: FilterKind,
    cutoff: f64,
    q: f64,
    /// Input drive as a linear pre-gain (1.0 = clean).
    drive: f64,
    /// 1 = 12 dB/oct, 2 = 24 dB/oct (SVF kinds).
    pub poles: u8,
    c: Coefs,
    a: [Svf; 2],
    b: [Svf; 2],
    ladder: [Ladder; 2],
    /// Post-drive make-up so `drive` changes character, not loudness.
    makeup: f64,
    sample_rate: f64,
    dirty: bool,
    /// Optional output trim.
    pub out_trim: f64,
}

impl Filter {
    pub fn new(kind: FilterKind, cutoff: f64, q: f64) -> Self {
        let mut f = Self {
            kind,
            cutoff: cutoff.max(10.0),
            q,
            drive: 1.0,
            poles: 1,
            c: Coefs::default(),
            a: [Svf::default(); 2],
            b: [Svf::default(); 2],
            ladder: [Ladder::default(); 2],
            makeup: 1.0,
            sample_rate: super::SR,
            dirty: true,
            out_trim: 1.0,
        };
        f.recompute();
        f
    }

    pub fn set_sample_rate(&mut self, sr: f64) {
        if sr != self.sample_rate {
            self.sample_rate = sr;
            self.dirty = true;
        }
    }

    pub fn set_cutoff(&mut self, hz: f64) {
        let hz = hz.clamp(10.0, 40_000.0);
        if hz != self.cutoff {
            self.cutoff = hz;
            self.dirty = true;
        }
    }

    pub fn cutoff(&self) -> f64 {
        self.cutoff
    }

    pub fn set_q(&mut self, q: f64) {
        if q != self.q {
            self.q = q;
            self.dirty = true;
        }
    }

    pub fn set_drive(&mut self, drive: f64) {
        let d = drive.clamp(0.05, 64.0);
        if d != self.drive {
            self.drive = d;
            // Keep perceived level roughly constant as the saturator works.
            self.makeup = if d > 1.0 { 1.0 / (1.0 + (d - 1.0) * 0.35).sqrt() } else { 1.0 };
        }
    }

    pub fn set_poles(&mut self, poles: u8) {
        self.poles = poles.clamp(1, 2);
    }

    fn recompute(&mut self) {
        let sr = self.sample_rate;
        let fc = self.cutoff.clamp(10.0, sr * 0.49);
        let g = (std::f64::consts::PI * fc / sr).tan();
        // Q in [0.5, 24] → damping k in [2, ~0.04]. With two cascaded
        // sections the per-section Q is softened so a 24 dB slope does not
        // double the resonant peak.
        let q = self.q.clamp(0.5, 24.0);
        let q_eff = if self.poles >= 2 { 0.5 + (q - 0.5) * 0.6 } else { q };
        let k = 1.0 / q_eff;
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;
        // Ladder feedback 0 … 3.95 (self-oscillation sits at 4).
        let ladder_k = (4.0 * (1.0 - 0.5 / q)).clamp(0.0, 3.95);
        self.c = Coefs { g, k, a1, a2, a3, ladder_k };
        self.dirty = false;
    }

    pub fn reset(&mut self) {
        self.a = [Svf::default(); 2];
        self.b = [Svf::default(); 2];
        self.ladder = [Ladder::default(); 2];
    }

    #[inline]
    fn run(&mut self, ch: usize, x: f64) -> f64 {
        let mut s = x;
        if self.drive > 1.0001 {
            s = soft_clip_f64(s * self.drive) * self.makeup;
        } else if self.drive < 0.9999 {
            s *= self.drive;
        }
        let y = if self.kind == FilterKind::Ladder {
            self.ladder[ch].tick(s, &self.c)
        } else {
            let y = self.a[ch].tick(s, &self.c, self.kind);
            if self.poles >= 2 {
                self.b[ch].tick(y, &self.c, self.kind)
            } else {
                y
            }
        };
        y * self.out_trim
    }

    /// Mono processing (channel 0 state).
    #[inline]
    pub fn tick(&mut self, x: f32) -> f32 {
        if self.dirty {
            self.recompute();
        }
        self.run(0, x as f64) as f32
    }

    /// Stereo processing with independent state per channel.
    #[inline]
    pub fn tick_stereo(&mut self, l: f32, r: f32) -> (f32, f32) {
        if self.dirty {
            self.recompute();
        }
        (self.run(0, l as f64) as f32, self.run(1, r as f64) as f32)
    }
}

/// A gentle fixed one-pole tone shaper.
#[derive(Clone)]
pub struct OnePole {
    z: f64,
    coef: f64,
    cutoff: f64,
    hp: bool,
}

impl OnePole {
    pub fn lowpass(cutoff: f64, sr: f64) -> Self {
        let mut p = Self { z: 0.0, coef: 0.0, cutoff, hp: false };
        p.set_sample_rate(sr);
        p
    }

    pub fn highpass(cutoff: f64, sr: f64) -> Self {
        let mut p = Self { z: 0.0, coef: 0.0, cutoff, hp: true };
        p.set_sample_rate(sr);
        p
    }

    pub fn set_sample_rate(&mut self, sr: f64) {
        self.coef = 1.0 - (-2.0 * std::f64::consts::PI * self.cutoff / sr).exp();
    }

    pub fn reset(&mut self) {
        self.z = 0.0;
    }

    #[inline]
    pub fn tick(&mut self, x: f32) -> f32 {
        let x = x as f64;
        self.z = flush(self.z + (x - self.z) * self.coef);
        (if self.hp { x - self.z } else { self.z }) as f32
    }
}

/// Convenience: a soft saturation with level compensation.
#[inline]
pub fn saturate(x: f32, amount: f32) -> f32 {
    if amount <= 0.0 {
        x
    } else {
        let d = 1.0 + amount * 8.0;
        soft_clip(x * d) / (1.0 + amount * 0.6)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::osc::{Osc, Wave};

    fn rms_through(kind: FilterKind, cut: f64, q: f64, poles: u8, f: f64) -> f64 {
        let sr = 48_000.0;
        let mut o = Osc::new(Wave::Sine);
        let mut flt = Filter::new(kind, cut, q);
        flt.set_sample_rate(sr);
        flt.set_poles(poles);
        let mut sum = 0.0;
        let n = 48_000;
        for i in 0..n {
            let x = o.tick(f, sr);
            let (y, _) = flt.tick_stereo(x, x);
            if i > n / 2 {
                sum += (y as f64).powi(2);
            }
        }
        (sum / (n / 2) as f64).sqrt() * std::f64::consts::SQRT_2
    }

    #[test]
    fn lowpass_passes_low_and_cuts_high() {
        let pass = rms_through(FilterKind::LowPass, 1000.0, 0.707, 2, 100.0);
        let stop = rms_through(FilterKind::LowPass, 1000.0, 0.707, 2, 8000.0);
        assert!((pass - 1.0).abs() < 0.05, "pass {pass}");
        assert!(stop < 0.005, "stop {stop}");
    }

    #[test]
    fn channels_have_independent_state() {
        // L = signal, R = silence: R must stay exactly silent.
        let mut f = Filter::new(FilterKind::LowPass, 800.0, 4.0);
        let mut o = Osc::new(Wave::Saw);
        for _ in 0..4410 {
            let (_, r) = f.tick_stereo(o.tick(220.0, 44_100.0), 0.0);
            assert_eq!(r, 0.0);
        }
    }

    #[test]
    fn ladder_is_stable_and_audible() {
        let mut o = Osc::new(Wave::Saw);
        let mut f = Filter::new(FilterKind::Ladder, 800.0, 20.0);
        let mut peak = 0.0f32;
        for _ in 0..44100 {
            let y = f.tick(o.tick(110.0, 44100.0));
            assert!(y.is_finite());
            peak = peak.max(y.abs());
        }
        assert!(peak > 0.01 && peak < 4.0, "ladder peak {peak}");
    }

    #[test]
    fn bandpass_is_unity_at_centre() {
        let g = rms_through(FilterKind::BandPass, 1000.0, 4.0, 1, 1000.0);
        assert!((g - 1.0).abs() < 0.05, "bp centre gain {g}");
    }
}
