//! Low-frequency oscillators used for filter/pitch/amp modulation.

use super::util::Rng;
use std::f64::consts::TAU;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LfoShape {
    Sine,
    Triangle,
    Saw,
    Ramp,
    Square,
    /// Sample & hold — a new random value each cycle.
    SampleHold,
    /// Smoothed random (a.k.a. "drift").
    Random,
}

impl LfoShape {
    pub fn from_index(i: f64) -> LfoShape {
        match i.round() as i64 {
            0 => LfoShape::Sine,
            1 => LfoShape::Triangle,
            2 => LfoShape::Saw,
            3 => LfoShape::Ramp,
            4 => LfoShape::Square,
            5 => LfoShape::SampleHold,
            6 => LfoShape::Random,
            _ => LfoShape::Sine,
        }
    }
}

#[derive(Clone)]
pub struct Lfo {
    phase: f64,
    pub shape: LfoShape,
    hold: f64,
    smooth: f64,
    rng: Rng,
}

impl Lfo {
    pub fn new(shape: LfoShape, seed: u64) -> Self {
        let mut rng = Rng::new(seed);
        let hold = rng.bipolar() as f64;
        Self { phase: 0.0, shape, hold, smooth: hold, rng }
    }

    pub fn set_phase(&mut self, p: f64) {
        self.phase = p.rem_euclid(1.0);
    }

    pub fn reset(&mut self) {
        self.phase = 0.0;
    }

    /// Bipolar output in [-1, 1].
    #[inline]
    pub fn tick(&mut self, rate: f64, sr: f64) -> f32 {
        let dt = (rate / sr).clamp(0.0, 0.5);
        let p = self.phase;
        self.phase += dt;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
            if matches!(self.shape, LfoShape::SampleHold | LfoShape::Random) {
                self.hold = self.rng.bipolar() as f64;
            }
        }
        match self.shape {
            LfoShape::Sine => (TAU * p).sin() as f32,
            LfoShape::Triangle => (1.0 - 4.0 * (p - 0.5).abs()) as f32,
            LfoShape::Saw => (2.0 * p - 1.0) as f32,
            LfoShape::Ramp => (1.0 - 2.0 * p) as f32,
            LfoShape::Square => if p < 0.5 { 1.0 } else { -1.0 },
            LfoShape::SampleHold => self.hold as f32,
            LfoShape::Random => {
                // Fixed ~15 ms glide so the random source is a "drift", not a
                // step, and stays independent of the LFO rate.
                self.smooth += (self.hold - self.smooth) * 0.0015;
                self.smooth as f32
            }
        }
    }
}
