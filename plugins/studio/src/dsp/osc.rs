//! Band-limited oscillators.
//!
//! The wavetable shapes — saw, square, triangle, pulse, organ, Hammond,
//! soft-saw and the PolyBLEP fallbacks — are rendered by the [`fundsp`] DSP
//! library, which builds *band-limited* mip-mapped wavetables. That is a real
//! step up in anti-aliasing quality over the previous polyBLEP-only code (and
//! makes PWM at any width, and organ drawbars, clean at every pitch).
//!
//! Sine and noise stay in-house: the voice engine needs phase modulation (FM)
//! on the sine carrier and seeded, deterministic noise, neither of which
//! fundsp's wavetable oscillators expose.
//!
//! Shape indices are part of the saved-config contract, so existing variants
//! keep their numbers; the new fundsp shapes are appended.

use std::f64::consts::PI;

use fundsp::prelude::{
    hammond, organ, poly_pulse, poly_saw, poly_square, pulse, saw, soft_saw, square, triangle, An,
    Frame, PolyPulse, PolySaw, PolySquare, PulseWave, Setting, WaveSynth, U1, U2,
};

use super::util::{cents_ratio, Rng};

/// Basic oscillator shape selected by the UI's `o1w`/`o2w`/`wave` params.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wave {
    Sine,
    Triangle,
    Saw,
    Square,
    /// Pulse with a variable width (the `pw` parameter).
    Pulse,
    /// Digital noise (per-sample white).
    Noise,
    /// A soft, formant-ish "organ" wave.
    Organ,
    /// Band-limited Hammond drawbar wave (fundsp).
    Hammond,
    /// Band-limited soft saw whose partials fall off like a triangle (fundsp).
    SoftSaw,
    /// Fast PolyBLEP saw (fundsp) — cheaper, slightly less pristine.
    PolySaw,
    /// Fast PolyBLEP square (fundsp).
    PolySquare,
    /// Fast PolyBLEP pulse (fundsp).
    PolyPulse,
}

impl Wave {
    pub fn from_index(i: f64) -> Wave {
        match i.round() as i64 {
            0 => Wave::Sine,
            1 => Wave::Triangle,
            2 => Wave::Saw,
            3 => Wave::Square,
            4 => Wave::Pulse,
            5 => Wave::Noise,
            6 => Wave::Organ,
            7 => Wave::Hammond,
            8 => Wave::SoftSaw,
            9 => Wave::PolySaw,
            10 => Wave::PolySquare,
            11 => Wave::PolyPulse,
            _ => Wave::Saw,
        }
    }
}

/// The fundsp node backing a shape, if any. `None` means the shape is rendered
/// by the in-house sine/noise code in [`Osc::tick`].
#[derive(Clone)]
enum Wt {
    None,
    Table(An<WaveSynth<U1>>),
    Pulse(An<PulseWave>),
    PolySaw(An<PolySaw<f32>>),
    PolySquare(An<PolySquare<f32>>),
    PolyPulse(An<PolyPulse<f32>>),
}

impl Wt {
    fn build(wave: Wave) -> Wt {
        match wave {
            Wave::Saw => Wt::Table(saw()),
            Wave::Square => Wt::Table(square()),
            Wave::Triangle => Wt::Table(triangle()),
            Wave::Organ => Wt::Table(organ()),
            Wave::Hammond => Wt::Table(hammond()),
            Wave::SoftSaw => Wt::Table(soft_saw()),
            Wave::Pulse => Wt::Pulse(pulse()),
            Wave::PolySaw => Wt::PolySaw(poly_saw()),
            Wave::PolySquare => Wt::PolySquare(poly_square()),
            Wave::PolyPulse => Wt::PolyPulse(poly_pulse()),
            Wave::Sine | Wave::Noise => Wt::None,
        }
    }

    fn is_none(&self) -> bool {
        matches!(self, Wt::None)
    }

    fn set_sample_rate(&mut self, sr: f64) {
        match self {
            Wt::Table(n) => n.set_sample_rate(sr),
            Wt::Pulse(n) => n.set_sample_rate(sr),
            Wt::PolySaw(n) => n.set_sample_rate(sr),
            Wt::PolySquare(n) => n.set_sample_rate(sr),
            Wt::PolyPulse(n) => n.set_sample_rate(sr),
            Wt::None => {}
        }
    }

    /// Seed the pseudorandom start phase so stacked/unison voices don't
    /// phase-lock into a comb.
    fn set_hash(&mut self, h: u64) {
        match self {
            Wt::Table(n) => n.set_hash(h),
            Wt::Pulse(n) => n.set_hash(h),
            Wt::PolySaw(n) => n.set_hash(h),
            Wt::PolySquare(n) => n.set_hash(h),
            Wt::PolyPulse(n) => n.set_hash(h),
            Wt::None => {}
        }
    }

    fn set_phase(&mut self, p: f32) {
        match self {
            Wt::Table(n) => {
                n.set(Setting::phase(p));
                n.reset();
            }
            Wt::Pulse(n) => {
                n.set(Setting::phase(p));
                n.reset();
            }
            Wt::PolySaw(n) => {
                n.set(Setting::phase(p));
                n.reset();
            }
            Wt::PolySquare(n) => {
                n.set(Setting::phase(p));
                n.reset();
            }
            Wt::PolyPulse(n) => {
                n.set(Setting::phase(p));
                n.reset();
            }
            Wt::None => {}
        }
    }

    fn reset(&mut self) {
        match self {
            Wt::Table(n) => n.reset(),
            Wt::Pulse(n) => n.reset(),
            Wt::PolySaw(n) => n.reset(),
            Wt::PolySquare(n) => n.reset(),
            Wt::PolyPulse(n) => n.reset(),
            Wt::None => {}
        }
    }
}

/// A single free-running oscillator with per-sample frequency input.
#[derive(Clone)]
pub struct Osc {
    phase: f64,
    tri: f64,
    /// Public for the Grid's parameter editor; a change is picked up lazily by
    /// [`Osc::tick`] via [`Osc::sync`].
    pub wave: Wave,
    pub pulse_width: f64,
    /// Phase offset in [0,1) — lets unison voices start decorrelated.
    offset: f64,
    rng: Rng,
    seed: u64,
    wt: Wt,
    wt_wave: Wave,
    sr: f64,
}

impl Osc {
    pub fn new(wave: Wave) -> Self {
        let seed = 0x1234_5678u64;
        let mut o = Self {
            phase: 0.0,
            tri: 0.0,
            wave,
            pulse_width: 0.5,
            offset: 0.0,
            rng: Rng::new(seed),
            seed,
            wt: Wt::build(wave),
            wt_wave: wave,
            sr: super::SR,
        };
        o.wt.set_sample_rate(o.sr);
        o.wt.set_hash(seed);
        o
    }

    pub fn with_seed(mut self, seed: u64) -> Self {
        self.rng = Rng::new(seed);
        self.seed = seed;
        self.wt.set_hash(seed);
        self
    }

    pub fn set_phase(&mut self, p: f64) {
        self.phase = p.rem_euclid(1.0);
        self.offset = self.phase;
        self.wt.set_phase(self.phase as f32);
    }

    /// Randomise the start phase (unison / supersaw decorrelation).
    pub fn randomize_phase(&mut self, seed: u64) {
        self.rng = Rng::new(seed);
        self.seed = seed;
        let p = self.rng.unipolar() as f64;
        self.set_phase(p);
        self.wt.set_hash(seed);
    }

    pub fn reset(&mut self) {
        self.phase = self.offset;
        self.tri = 0.0;
        self.wt.reset();
    }

    /// Rebuild the fundsp node after `wave` was changed directly (the Grid and
    /// parameter UI write the field).
    fn sync(&mut self) {
        if self.wave != self.wt_wave {
            self.wt = Wt::build(self.wave);
            self.wt.set_sample_rate(self.sr);
            self.wt.set_hash(self.seed);
            self.wt_wave = self.wave;
        }
    }

    /// Generate one sample for a frequency in Hz at sample rate `sr`.
    #[inline]
    pub fn tick(&mut self, freq: f64, sr: f64) -> f32 {
        if self.wave != self.wt_wave {
            self.sync();
        }
        if !self.wt.is_none() {
            if (sr - self.sr).abs() > 1e-9 {
                self.sr = sr;
                self.wt.set_sample_rate(sr);
            }
            let f = freq as f32;
            let pw = self.pulse_width.clamp(0.02, 0.98) as f32;
            return match &mut self.wt {
                Wt::Table(n) => {
                    let x: Frame<f32, U1> = [f].into();
                    n.tick(&x)[0]
                }
                Wt::Pulse(n) => {
                    let x: Frame<f32, U2> = [f, pw].into();
                    n.tick(&x)[0]
                }
                Wt::PolySaw(n) => {
                    let x: Frame<f32, U1> = [f].into();
                    n.tick(&x)[0]
                }
                Wt::PolySquare(n) => {
                    let x: Frame<f32, U1> = [f].into();
                    n.tick(&x)[0]
                }
                Wt::PolyPulse(n) => {
                    let x: Frame<f32, U2> = [f, pw].into();
                    n.tick(&x)[0]
                }
                Wt::None => 0.0,
            };
        }

        let dt = (freq / sr).clamp(0.0, 0.45);
        let p = self.phase;
        let sample = match self.wave {
            Wave::Sine => (2.0 * PI * p).sin(),
            Wave::Noise => self.rng.bipolar() as f64,
            _ => 0.0,
        };

        self.phase += dt;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        sample as f32
    }

    /// Phase-modulated sine (FM). Only meaningful for `Wave::Sine`; other
    /// shapes fall back to their normal `tick` so FM can't produce nonsense.
    #[inline]
    pub fn tick_pm(&mut self, freq: f64, sr: f64, pm: f64) -> f32 {
        if self.wave != Wave::Sine {
            return self.tick(freq, sr);
        }
        let dt = (freq / sr).clamp(0.0, 0.45);
        let s = (2.0 * PI * (self.phase + pm)).sin();
        self.phase += dt;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        s as f32
    }
}

/// A detuned unison stack with stereo spread — the difference between a thin
/// one-oscillator lead and a "real" supersaw.
#[derive(Clone)]
pub struct Unison {
    oscs: Vec<Osc>,
    detunes: Vec<f64>,
    pans: Vec<(f32, f32)>,
    gains: Vec<f32>,
}

impl Unison {
    /// `count` voices, `detune_cents` total spread, `spread` stereo width 0..1.
    pub fn new(count: usize, wave: Wave, detune_cents: f64, spread: f64, seed: u64) -> Self {
        let n = count.clamp(1, 16);
        let mut oscs = Vec::with_capacity(n);
        let mut detunes = Vec::with_capacity(n);
        let mut pans = Vec::with_capacity(n);
        let mut gains = Vec::with_capacity(n);
        for i in 0..n {
            let mut o = Osc::new(wave).with_seed(seed.wrapping_add(i as u64 * 7919));
            o.randomize_phase(seed.wrapping_add(i as u64 * 104_729));
            // [-0.5, 0.5] → symmetric detune; the centre voice stays in tune.
            let t = if n == 1 { 0.0 } else { i as f64 / (n - 1) as f64 - 0.5 };
            detunes.push(t * detune_cents);
            let pan = (t * 2.0 * spread).clamp(-1.0, 1.0) as f32;
            pans.push(super::util::pan_gains(pan as f64));
            // Slight level compensation so more voices don't just get louder.
            gains.push((1.0 / (n as f32).sqrt()) * (1.0 - 0.25 * t.abs() as f32));
            oscs.push(o);
        }
        Self { oscs, detunes, pans, gains }
    }

    pub fn count(&self) -> usize {
        self.oscs.len()
    }

    pub fn set_wave(&mut self, wave: Wave) {
        for o in &mut self.oscs {
            o.wave = wave;
        }
    }

    pub fn set_pulse_width(&mut self, pw: f64) {
        for o in &mut self.oscs {
            o.pulse_width = pw;
        }
    }

    pub fn reset(&mut self) {
        for o in &mut self.oscs {
            o.reset();
        }
    }

    /// Render one sample into a stereo pair.
    #[inline]
    pub fn tick(&mut self, freq: f64, sr: f64) -> (f32, f32) {
        let mut l = 0.0f32;
        let mut r = 0.0f32;
        for i in 0..self.oscs.len() {
            let f = freq * cents_ratio(self.detunes[i]);
            let s = self.oscs[i].tick(f, sr) * self.gains[i];
            l += s * self.pans[i].0;
            r += s * self.pans[i].1;
        }
        (l, r)
    }

    /// Phase-modulated stereo render (FM through a unison stack).
    #[inline]
    pub fn tick_pm(&mut self, freq: f64, sr: f64, pm: f64) -> (f32, f32) {
        let mut l = 0.0f32;
        let mut r = 0.0f32;
        for i in 0..self.oscs.len() {
            let f = freq * cents_ratio(self.detunes[i]);
            let s = self.oscs[i].tick_pm(f, sr, pm) * self.gains[i];
            l += s * self.pans[i].0;
            r += s * self.pans[i].1;
        }
        (l, r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_wave_renders_finite_audio() {
        for i in 0..12 {
            let w = Wave::from_index(i as f64);
            let mut o = Osc::new(w);
            let mut peak = 0.0f32;
            for _ in 0..4410 {
                let s = o.tick(440.0, 44100.0);
                assert!(s.is_finite(), "{w:?} produced non-finite output");
                peak = peak.max(s.abs());
            }
            assert!(peak > 0.05, "{w:?} rendered silence (peak {peak})");
        }
    }

    #[test]
    fn unison_voices_are_decorrelated() {
        // Two unison voices must not phase-lock into an identical signal.
        let mut a = Unison::new(1, Wave::Saw, 0.0, 0.0, 1);
        let mut b = Unison::new(1, Wave::Saw, 0.0, 0.0, 2);
        let mut diff = 0.0f64;
        for _ in 0..4410 {
            let (la, _) = a.tick(220.0, 44100.0);
            let (lb, _) = b.tick(220.0, 44100.0);
            diff += ((la - lb) as f64).abs();
        }
        assert!(diff > 1.0, "unison stacks are phase-locked (diff {diff})");
    }
}
