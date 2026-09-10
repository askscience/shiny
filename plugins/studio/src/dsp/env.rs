//! Envelopes.
//!
//! [`Adsr`] is an analog-style exponential envelope: the attack *overshoots*
//! its target (like a charging RC circuit driving a comparator) and decay /
//! release approach their targets asymptotically. A `shape` control blends
//! between the exponential response and a linear one, which is what most
//! synths expose as "curve".

use super::SR;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Stage {
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

#[derive(Clone)]
pub struct Adsr {
    stage: Stage,
    level: f64,
    attack: f64,
    decay: f64,
    sustain: f64,
    release: f64,
    /// 0 = exponential, 1 = linear.
    shape: f64,
    sample_rate: f64,
    /// Cached coefficients, recomputed when a time changes.
    ac: f64,
    dc: f64,
    rc: f64,
    dirty: bool,
}

impl Adsr {
    pub fn new(attack: f64, decay: f64, sustain: f64, release: f64) -> Self {
        let mut e = Self {
            stage: Stage::Idle,
            level: 0.0,
            attack,
            decay,
            sustain,
            release,
            shape: 0.0,
            sample_rate: SR,
            ac: 0.0,
            dc: 0.0,
            rc: 0.0,
            dirty: true,
        };
        e.recompute();
        e
    }

    pub fn set_times(&mut self, attack: f64, decay: f64, sustain: f64, release: f64) {
        self.attack = attack.max(0.0002);
        self.decay = decay.max(0.0002);
        self.sustain = sustain.clamp(0.0, 1.0);
        self.release = release.max(0.0002);
        self.dirty = true;
    }

    pub fn times(&self) -> (f64, f64, f64, f64) {
        (self.attack, self.decay, self.sustain, self.release)
    }

    pub fn set_shape(&mut self, shape: f64) {
        self.shape = shape.clamp(0.0, 1.0);
    }

    pub fn set_sample_rate(&mut self, sr: f64) {
        self.sample_rate = sr;
        self.dirty = true;
    }

    fn recompute(&mut self) {
        let sr = self.sample_rate;
        // Attack reaches 1.0 at ≈ `attack` seconds despite the 1.3 overshoot
        // target (the asymptote is reached faster than a plain RC curve).
        let tau_a = (self.attack * 0.683).max(1e-5);
        self.ac = 1.0 - (-1.0 / (tau_a * sr)).exp();
        // Decay/release settle to within ~1 % in their nominal time.
        let tau_d = (self.decay / 4.6).max(1e-5);
        let tau_r = (self.release / 4.6).max(1e-5);
        self.dc = 1.0 - (-1.0 / (tau_d * sr)).exp();
        self.rc = 1.0 - (-1.0 / (tau_r * sr)).exp();
        self.dirty = false;
    }

    pub fn gate_on(&mut self) {
        self.stage = Stage::Attack;
    }

    pub fn gate_off(&mut self) {
        if self.stage != Stage::Idle {
            self.stage = Stage::Release;
        }
    }

    pub fn reset(&mut self) {
        self.stage = Stage::Idle;
        self.level = 0.0;
    }

    pub fn is_active(&self) -> bool {
        self.stage != Stage::Idle
    }

    pub fn level(&self) -> f64 {
        self.level
    }

    #[inline]
    pub fn tick(&mut self) -> f64 {
        if self.dirty {
            self.recompute();
        }
        let lin = self.shape;
        let exp = 1.0 - lin;
        match self.stage {
            Stage::Idle => {}
            Stage::Attack => {
                let target = 1.3;
                let inc = (target - self.level) * self.ac;
                let step = 1.0 / (self.attack * self.sample_rate).max(1.0);
                self.level += inc * exp + step * lin;
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = Stage::Decay;
                }
            }
            Stage::Decay => {
                let inc = (self.sustain - self.level) * self.dc;
                let step = (self.sustain - self.level) / (self.decay * self.sample_rate).max(1.0);
                self.level += inc * exp + step * lin;
                if (self.level - self.sustain).abs() < 1e-4 {
                    self.level = self.sustain;
                    self.stage = Stage::Sustain;
                }
            }
            Stage::Sustain => {
                self.level = self.sustain;
            }
            Stage::Release => {
                let target = -0.02;
                let inc = (target - self.level) * self.rc;
                let step = -self.level / (self.release * self.sample_rate).max(1.0);
                self.level += inc * exp + step * lin;
                if self.level <= 1e-4 {
                    self.level = 0.0;
                    self.stage = Stage::Idle;
                }
            }
        }
        self.level
    }
}

/// A percussive one-shot envelope: fast attack, exponential decay, optional
/// hold. Drums use this instead of a full ADSR (no sustain stage).
#[derive(Clone)]
pub struct PercEnv {
    level: f64,
    attack: f64,
    decay: f64,
    hold: f64,
    time: f64,
    velocity: f64,
    sample_rate: f64,
    attack_coef: f64,
    decay_coef: f64,
}

impl PercEnv {
    pub fn new(attack: f64, decay: f64, hold: f64) -> Self {
        let mut e = Self {
            level: 0.0,
            attack,
            decay,
            hold,
            time: 0.0,
            velocity: 1.0,
            sample_rate: SR,
            attack_coef: 0.0,
            decay_coef: 0.0,
        };
        e.recompute();
        e
    }

    pub fn set(&mut self, attack: f64, decay: f64, hold: f64) {
        self.attack = attack.max(0.0);
        self.decay = decay.max(0.0005);
        self.hold = hold.max(0.0);
        self.recompute();
    }

    fn recompute(&mut self) {
        let sr = self.sample_rate;
        let ta = (self.attack * 0.683).max(1e-5);
        self.attack_coef = 1.0 - (-1.0 / (ta * sr)).exp();
        let td = (self.decay / 4.6).max(1e-5);
        self.decay_coef = 1.0 - (-1.0 / (td * sr)).exp();
    }

    pub fn trigger(&mut self, velocity: f64) {
        self.time = 0.0;
        self.velocity = velocity.clamp(0.0, 1.6);
        // An instant attack jumps straight to the peak; a slower one rises from
        // wherever the envelope currently is so re-triggers don't click.
        if self.attack <= 0.0 || self.level < 1e-6 {
            self.level = self.velocity;
        }
    }

    pub fn reset(&mut self) {
        self.level = 0.0;
        self.time = 0.0;
    }

    pub fn is_active(&self) -> bool {
        self.level > 1e-5
    }

    pub fn level(&self) -> f64 {
        self.level
    }

    /// Decay stage only (used by hats/perc where the attack is instant).
    #[inline]
    pub fn tick_decay(&mut self) -> f64 {
        self.level *= 1.0 - self.decay_coef;
        self.level
    }

    #[inline]
    pub fn tick(&mut self) -> f64 {
        let dt = 1.0 / self.sample_rate;
        self.time += dt;
        if self.time < self.hold {
            self.level = self.velocity;
        } else if self.level < self.velocity && self.attack > 0.0 {
            let inc = (self.velocity * 1.3 - self.level) * self.attack_coef;
            self.level += inc;
            if self.level >= self.velocity {
                self.level = self.velocity;
            }
        } else {
            self.level += (0.0 - self.level) * self.decay_coef;
            if self.level < 1e-6 {
                self.level = 0.0;
            }
        }
        self.level
    }
}
