//! Noise sources — white, pink and brown — plus a resonant "metallic" bank
//! used by hats and cymbals.

use super::util::Rng;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoiseKind {
    White,
    Pink,
    Brown,
    /// Band-passed white noise (bright, breathy — snare/clap body).
    Bright,
}

impl NoiseKind {
    pub fn from_index(i: f64) -> NoiseKind {
        match i.round() as i64 {
            1 => NoiseKind::Pink,
            2 => NoiseKind::Brown,
            3 => NoiseKind::Bright,
            _ => NoiseKind::White,
        }
    }
}

#[derive(Clone)]
pub struct Noise {
    rng: Rng,
    kind: NoiseKind,
    // Pink (Paul Kellet's economy filter).
    p0: f32,
    p1: f32,
    p2: f32,
    // Brown.
    brown: f32,
    // Bright: one-pole high-pass state.
    hp: f32,
}

impl Noise {
    pub fn new(seed: u64) -> Self {
        Self { rng: Rng::new(seed), kind: NoiseKind::White, p0: 0.0, p1: 0.0, p2: 0.0, brown: 0.0, hp: 0.0 }
    }

    pub fn set_kind(&mut self, kind: NoiseKind) {
        self.kind = kind;
    }

    pub fn reset(&mut self) {
        self.p0 = 0.0;
        self.p1 = 0.0;
        self.p2 = 0.0;
        self.brown = 0.0;
        self.hp = 0.0;
    }

    #[inline]
    pub fn tick(&mut self) -> f32 {
        let w = self.rng.bipolar();
        match self.kind {
            NoiseKind::White => w,
            NoiseKind::Pink => {
                self.p0 = 0.99765 * self.p0 + w * 0.099_046;
                self.p1 = 0.963_00 * self.p1 + w * 0.296_516;
                self.p2 = 0.570_00 * self.p2 + w * 1.052_691;
                (self.p0 + self.p1 + self.p2 + w * 0.1848) * 0.28
            }
            NoiseKind::Brown => {
                self.brown = (self.brown + 0.02 * w).clamp(-1.0, 1.0);
                self.brown * 3.0
            }
            NoiseKind::Bright => {
                // Differentiate white noise (one-pole high pass at ~3 kHz).
                let out = w - self.hp;
                self.hp += (w - self.hp) * 0.35;
                out
            }
        }
    }
}

/// Six square waves at the classic 808 metallic ratios through an optional
/// high-pass — the backbone of hi-hats, cymbals and shakers.
#[derive(Clone)]
pub struct MetalBank {
    phases: [f64; 6],
    ratios: [f64; 6],
}

impl Default for MetalBank {
    fn default() -> Self {
        Self::new()
    }
}

impl MetalBank {
    pub fn new() -> Self {
        Self { phases: [0.0; 6], ratios: [1.0, 1.4471, 1.6170, 1.9265, 2.5028, 2.6637] }
    }

    pub fn reset(&mut self) {
        self.phases = [0.0; 6];
    }

    /// Raw square bank at `freq` (typically 40–120 Hz for a hat).
    #[inline]
    pub fn tick(&mut self, freq: f64, sr: f64) -> f32 {
        let mut out = 0.0f32;
        for i in 0..6 {
            let dt = (freq * self.ratios[i] / sr).clamp(0.0, 0.45);
            out += if self.phases[i] < 0.5 { 1.0 } else { -1.0 };
            self.phases[i] += dt;
            if self.phases[i] >= 1.0 {
                self.phases[i] -= 1.0;
            }
        }
        out * (1.0 / 6.0)
    }
}
