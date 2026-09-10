//! Band-limited oscillators.
//!
//! Every shape is derived from a polyBLEP-corrected saw so that the whole set
//! shares one phase accumulator and one correction routine:
//!
//! * `saw`   — the corrected ramp directly.
//! * `square`/`pulse` — the difference of two saws (a pulse of width `pw`);
//!   polyBLEP lands on both edges, so PWM stays clean at any width.
//! * `triangle` — the *integral* of the corrected square. This fixes the
//!   classic bug where an integrated square is scaled by the pitch-dependent
//!   step size (`4·dt`) instead of being accumulated, which makes the triangle
//!   quieter as it goes up and detunes the timbre.
//! * `sine` — a lookup-free `sin`, used for subs and FM carriers.

use std::f64::consts::PI;

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
    /// A soft, formant-ish "organ" wave (sine + 2nd + 3rd harmonics).
    Organ,
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
            _ => Wave::Saw,
        }
    }
}

/// Residual of a step discontinuity (polyBLEP).
#[inline]
fn poly_blep(t: f64, dt: f64) -> f64 {
    if dt <= 0.0 {
        return 0.0;
    }
    if t < dt {
        let t = t / dt;
        2.0 * t - t * t - 1.0
    } else if t > 1.0 - dt {
        let t = (t - 1.0) / dt;
        t * t + 2.0 * t + 1.0
    } else {
        0.0
    }
}

/// A single free-running oscillator with per-sample frequency input.
#[derive(Clone)]
pub struct Osc {
    phase: f64,
    tri: f64,
    pub wave: Wave,
    pub pulse_width: f64,
    /// Phase offset in [0,1) — lets unison voices start decorrelated.
    offset: f64,
    rng: Rng,
}

impl Osc {
    pub fn new(wave: Wave) -> Self {
        Self { phase: 0.0, tri: 0.0, wave, pulse_width: 0.5, offset: 0.0, rng: Rng::new(0x1234_5678) }
    }

    pub fn with_seed(mut self, seed: u64) -> Self {
        self.rng = Rng::new(seed);
        self
    }

    pub fn set_phase(&mut self, p: f64) {
        self.phase = p.rem_euclid(1.0);
        self.offset = self.phase;
    }

    /// Randomise the start phase (unison / supersaw decorrelation).
    pub fn randomize_phase(&mut self, seed: u64) {
        self.rng = Rng::new(seed);
        let p = self.rng.unipolar() as f64;
        self.set_phase(p);
    }

    pub fn reset(&mut self) {
        self.phase = self.offset;
        self.tri = 0.0;
    }

    /// Generate one sample for a frequency in Hz at sample rate `sr`.
    #[inline]
    pub fn tick(&mut self, freq: f64, sr: f64) -> f32 {
        let dt = (freq / sr).clamp(0.0, 0.45);
        let p = self.phase;
        let sample = match self.wave {
            Wave::Sine => (2.0 * PI * p).sin(),
            Wave::Organ => {
                // Sine with a touch of 2nd/3rd harmonic — cheap drawbar colour.
                0.62 * (2.0 * PI * p).sin() + 0.28 * (4.0 * PI * p).sin() + 0.16 * (6.0 * PI * p).sin()
            }
            Wave::Saw => 2.0 * p - 1.0 - poly_blep(p, dt),
            Wave::Square | Wave::Pulse => {
                let pw = if self.wave == Wave::Square { 0.5 } else { self.pulse_width.clamp(0.02, 0.98) };
                let p2 = (p + pw).rem_euclid(1.0);
                let a = 2.0 * p - 1.0 - poly_blep(p, dt);
                let b = 2.0 * p2 - 1.0 - poly_blep(p2, dt);
                // Difference of two ramps = a pulse spanning [-2pw, 2-2pw]
                // with mean 2-4pw. Remove that mean (duty changes must not
                // thump) and halve to unity amplitude.
                0.5 * ((a - b) - (2.0 - 4.0 * pw))
            }
            Wave::Triangle => {
                // Integrate the corrected square. `4·dt` is the per-sample
                // phase step scaled by the triangle's slope (4 per cycle), so
                // the accumulating integral has unit amplitude at every pitch.
                let sq = {
                    let p2 = (p + 0.5).rem_euclid(1.0);
                    let a = 2.0 * p - 1.0 - poly_blep(p, dt);
                    let b = 2.0 * p2 - 1.0 - poly_blep(p2, dt);
                    a - b
                };
                self.tri += sq * 4.0 * dt;
                // A whisper of leak keeps DC from wandering on very long notes.
                self.tri *= 0.999_95;
                self.tri
            }
            Wave::Noise => self.rng.bipolar() as f64,
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
