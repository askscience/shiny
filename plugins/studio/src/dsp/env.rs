//! Envelopes.
//!
//! [`Adsr`] is an analog-style envelope: the attack charges toward an
//! overshooting target (like an RC circuit driving a comparator, so it is
//! fast at first and lands exactly on 1.0 at the attack time) while decay and
//! release fall exponentially — a straight line in dB, which is how a natural
//! sound dies away. A `shape` control blends toward linear segments.
//!
//! Every segment starts from the *current* level, so re-triggering a ringing
//! voice or releasing mid-attack never jumps. Release ends at −90 dBFS, where
//! the envelope snaps to an exact zero and reports itself idle.

use super::SR;

/// Level below which a releasing envelope is considered silent (−90 dB).
const SILENT: f64 = 3.2e-5;
/// Decay/release times are quoted as "time to fall 60 dB" (RT60-style).
const T60: f64 = 6.907_755_278_982_137; // ln(1000)

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
    ac: f64,
    dc: f64,
    rc: f64,
    /// Linear-segment slopes (per sample).
    a_step: f64,
    d_step: f64,
    r_step: f64,
    dirty: bool,
}

impl Adsr {
    pub fn new(attack: f64, decay: f64, sustain: f64, release: f64) -> Self {
        let mut e = Self {
            stage: Stage::Idle,
            level: 0.0,
            attack: 0.0,
            decay: 0.0,
            sustain: 0.0,
            release: 0.0,
            shape: 0.0,
            sample_rate: SR,
            ac: 0.0,
            dc: 0.0,
            rc: 0.0,
            a_step: 0.0,
            d_step: 0.0,
            r_step: 0.0,
            dirty: true,
        };
        e.set_times(attack, decay, sustain, release);
        e.recompute();
        e
    }

    pub fn set_times(&mut self, attack: f64, decay: f64, sustain: f64, release: f64) {
        let a = attack.max(0.0005);
        let d = decay.max(0.001);
        let s = sustain.clamp(0.0, 1.0);
        let r = release.max(0.002);
        if a != self.attack || d != self.decay || s != self.sustain || r != self.release {
            self.attack = a;
            self.decay = d;
            self.sustain = s;
            self.release = r;
            self.dirty = true;
        }
    }

    pub fn times(&self) -> (f64, f64, f64, f64) {
        (self.attack, self.decay, self.sustain, self.release)
    }

    pub fn set_shape(&mut self, shape: f64) {
        self.shape = shape.clamp(0.0, 1.0);
    }

    pub fn set_sample_rate(&mut self, sr: f64) {
        if sr != self.sample_rate {
            self.sample_rate = sr;
            self.dirty = true;
        }
    }

    fn recompute(&mut self) {
        let sr = self.sample_rate;
        // Charging toward 1.5: reaching 1.0 takes ln(3) time constants.
        let tau_a = self.attack / 3f64.ln();
        self.ac = 1.0 - (-1.0 / (tau_a * sr)).exp();
        let tau_d = self.decay / T60;
        let tau_r = self.release / T60;
        self.dc = 1.0 - (-1.0 / (tau_d * sr)).exp();
        self.rc = 1.0 - (-1.0 / (tau_r * sr)).exp();
        self.a_step = 1.0 / (self.attack * sr).max(1.0);
        self.d_step = 1.0 / (self.decay * sr).max(1.0);
        self.r_step = 1.0 / (self.release * sr).max(1.0);
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

    pub fn is_releasing(&self) -> bool {
        self.stage == Stage::Release
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
                let e = (1.5 - self.level) * self.ac;
                self.level += e * exp + self.a_step * lin;
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = Stage::Decay;
                }
            }
            Stage::Decay => {
                let s = self.sustain;
                let e = (s - self.level) * self.dc;
                let l = -self.d_step * (1.0 - s);
                self.level += e * exp + l * lin;
                if self.level <= s + 1e-6 {
                    self.level = s;
                    self.stage = Stage::Sustain;
                }
            }
            Stage::Sustain => {
                // Track sustain changes smoothly (automation).
                self.level += (self.sustain - self.level) * self.dc.max(0.001);
            }
            Stage::Release => {
                let e = -self.level * self.rc;
                let l = -self.r_step;
                self.level += e * exp + l * lin;
                if self.level <= SILENT {
                    self.level = 0.0;
                    self.stage = Stage::Idle;
                }
            }
        }
        if self.stage == Stage::Sustain && self.sustain <= SILENT && self.level <= SILENT {
            // A zero-sustain envelope has finished once it decays to silence.
            self.level = 0.0;
            self.stage = Stage::Idle;
        }
        self.level
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attack_lands_on_time() {
        let sr = 48_000.0;
        let mut e = Adsr::new(0.1, 0.2, 0.5, 0.3);
        e.set_sample_rate(sr);
        e.gate_on();
        let mut n = 0;
        while e.tick() < 1.0 {
            n += 1;
            assert!(n < 48_000);
        }
        let t = n as f64 / sr;
        assert!((t - 0.1).abs() < 0.005, "attack took {t}");
    }

    #[test]
    fn release_reaches_exact_zero() {
        let sr = 44_100.0;
        let mut e = Adsr::new(0.001, 0.05, 0.8, 0.5);
        e.set_sample_rate(sr);
        e.gate_on();
        for _ in 0..4410 {
            e.tick();
        }
        e.gate_off();
        let mut n = 0;
        while e.is_active() {
            e.tick();
            n += 1;
            assert!(n < 10 * 44_100, "release never ended");
        }
        assert_eq!(e.level(), 0.0);
        // −60 dB at 0.5 s, −90 dB at ~0.75 s.
        let t = n as f64 / sr;
        assert!(t > 0.6 && t < 0.9, "release length {t}");
    }

    #[test]
    fn retrigger_does_not_jump() {
        let mut e = Adsr::new(0.01, 0.1, 0.5, 0.2);
        e.gate_on();
        for _ in 0..2000 {
            e.tick();
        }
        e.gate_off();
        for _ in 0..1000 {
            e.tick();
        }
        let before = e.level();
        e.gate_on();
        let after = e.tick();
        assert!((after - before).abs() < 0.05);
    }
}
