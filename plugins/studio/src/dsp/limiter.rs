//! Dynamics: a soft-knee stereo compressor and a look-ahead brickwall limiter,
//! plus the master bus glue used by the engine.

use super::util::{db_to_gain, gain_to_db, soft_clip, Oversampler};

/// Feed-forward stereo-linked compressor with a soft knee and parallel mix.
#[derive(Clone)]
pub struct Compressor {
    env: f32,
    att_coef: f32,
    rel_coef: f32,
    pub threshold_db: f64,
    pub ratio: f64,
    pub attack_ms: f64,
    pub release_ms: f64,
    pub knee_db: f64,
    pub makeup_db: f64,
    pub mix: f64,
    sr: f64,
    /// Smoothed gain reduction in dB (for metering / UI).
    pub reduction_db: f64,
}

impl Compressor {
    pub fn new(threshold_db: f64, ratio: f64, attack_ms: f64, release_ms: f64) -> Self {
        let mut c = Self {
            env: 0.0,
            att_coef: 0.0,
            rel_coef: 0.0,
            threshold_db,
            ratio,
            attack_ms,
            release_ms,
            knee_db: 6.0,
            makeup_db: 0.0,
            mix: 1.0,
            sr: 44100.0,
            reduction_db: 0.0,
        };
        c.recompute();
        c
    }

    pub fn set_sample_rate(&mut self, sr: f64) {
        self.sr = sr;
        self.recompute();
    }

    fn recompute(&mut self) {
        let a = (self.attack_ms.max(0.05) * 0.001 * self.sr as f64) as f32;
        let r = (self.release_ms.max(1.0) * 0.001 * self.sr as f64) as f32;
        self.att_coef = 1.0 - (-1.0 / a.max(1.0)).exp();
        self.rel_coef = 1.0 - (-1.0 / r.max(1.0)).exp();
    }

    pub fn reset(&mut self) {
        self.env = 0.0;
        self.reduction_db = 0.0;
    }

    #[inline]
    pub fn tick(&mut self, l: f32, r: f32) -> (f32, f32) {
        // Peak detection, stereo-linked, on the loudest channel.
        let peak = l.abs().max(r.abs());
        let coef = if peak > self.env { self.att_coef } else { self.rel_coef };
        self.env += (peak - self.env) * coef;
        let level_db = gain_to_db(self.env.max(1e-6)) as f64;

        let over = level_db - self.threshold_db;
        let slope = 1.0 - 1.0 / self.ratio.max(1.0);
        // Quadratic soft knee around the threshold.
        let gr_db = if over <= -self.knee_db * 0.5 {
            0.0
        } else if over >= self.knee_db * 0.5 {
            -over * slope
        } else {
            let x = over + self.knee_db * 0.5;
            -(slope * x * x / (2.0 * self.knee_db.max(0.001)))
        };
        self.reduction_db = gr_db;
        let gain = db_to_gain(gr_db + self.makeup_db) as f32;
        let mix = self.mix.clamp(0.0, 1.0) as f32;
        (l * (1.0 - mix + gain * mix), r * (1.0 - mix + gain * mix))
    }
}

/// Look-ahead brickwall limiter.
///
/// The detector sees the signal `lookahead` frames early, so the gain is
/// already down when the transient arrives — no overshoot, no clicky attack.
/// The release is program-dependent (two time constants) so it breathes with
/// the material instead of pumping.
#[derive(Clone)]
pub struct Limiter {
    delay_l: Vec<f32>,
    delay_r: Vec<f32>,
    pos: usize,
    gain: f32,
    /// Fast (transient) and slow (program) release coefficients.
    fast_rel: f32,
    slow_rel: f32,
    pub ceiling_db: f64,
    ceiling: f32,
    release_ms: f64,
    sr: f64,
}

impl Limiter {
    pub fn new(ceiling_db: f64, release_ms: f64, sr: f64) -> Self {
        let ceiling = db_to_gain(ceiling_db);
        let mut l = Self {
            delay_l: Vec::new(),
            delay_r: Vec::new(),
            pos: 0,
            gain: 1.0,
            fast_rel: 0.0,
            slow_rel: 0.0,
            ceiling_db,
            ceiling,
            release_ms,
            sr,
        };
        l.rebuild();
        l
    }

    fn rebuild(&mut self) {
        let lookahead = (0.0015 * self.sr) as usize; // 1.5 ms of gain look-ahead
        self.delay_l = vec![0.0; lookahead + 2];
        self.delay_r = vec![0.0; lookahead + 2];
        self.pos = 0;
        let fast_tau = (self.release_ms * 0.1).max(1.0) * 0.001 * self.sr;
        let slow_tau = self.release_ms.max(5.0) * 0.001 * self.sr;
        self.fast_rel = (1.0 - (-1.0 / fast_tau).exp()) as f32;
        self.slow_rel = (1.0 - (-1.0 / slow_tau).exp()) as f32;
    }

    pub fn set_sample_rate(&mut self, sr: f64) {
        if (sr - self.sr).abs() > 1e-9 {
            self.sr = sr;
            self.rebuild();
        }
    }

    /// Change the output ceiling (dBFS).
    pub fn set_ceiling(&mut self, ceiling_db: f64) {
        self.ceiling_db = ceiling_db;
        self.ceiling = db_to_gain(ceiling_db);
    }

    pub fn reset(&mut self) {
        for s in self.delay_l.iter_mut().chain(self.delay_r.iter_mut()) {
            *s = 0.0;
        }
        self.gain = 1.0;
        self.pos = 0;
    }

    #[inline]
    pub fn tick(&mut self, l: f32, r: f32) -> (f32, f32) {
        let n = self.delay_l.len();
        let peak = l.abs().max(r.abs());
        // Gain needed to keep this sample under the ceiling.
        let target = if peak > self.ceiling { self.ceiling / peak } else { 1.0 };
        if target < self.gain {
            // Instant attack (we have look-ahead to hide it).
            self.gain = target;
        } else {
            // Two-stage release: quick for small overshoots, slow for big ones.
            let rel = if target - self.gain > 0.15 { self.fast_rel } else { self.slow_rel };
            self.gain += (target - self.gain) * rel;
        }

        let dl = self.delay_l[self.pos];
        let dr = self.delay_r[self.pos];
        self.delay_l[self.pos] = l;
        self.delay_r[self.pos] = r;
        self.pos = (self.pos + 1) % n;

        let c = self.ceiling;
        ((dl * self.gain).clamp(-c, c), (dr * self.gain).clamp(-c, c))
    }
}

/// The master bus: DC safety, gentle bus saturation, and a final clip.
///
/// The saturation is the only nonlinear master stage, so it is the one worth
/// oversampling (`master_oversample` in the config). The limiter downstream is
/// a pure gain law and does not alias.
#[derive(Clone)]
pub struct MasterBus {
    dc_l: super::util::DcBlocker,
    dc_r: super::util::DcBlocker,
    pub drive: f64,
    pub width: f64,
    pub gain: f64,
    os: Oversampler,
    os_l: [f32; 8],
    os_r: [f32; 8],
}

impl Default for MasterBus {
    fn default() -> Self {
        Self::new(super::SR, 1)
    }
}

impl MasterBus {
    pub fn new(sr: f64, oversample: usize) -> Self {
        Self {
            dc_l: Default::default(),
            dc_r: Default::default(),
            drive: 0.0,
            width: 1.0,
            gain: 1.0,
            os: Oversampler::new(sr, oversample),
            os_l: [0.0; 8],
            os_r: [0.0; 8],
        }
    }

    pub fn reset(&mut self) {
        self.dc_l.reset();
        self.dc_r.reset();
        self.os.reset();
    }

    #[inline]
    pub fn tick(&mut self, l: f32, r: f32) -> (f32, f32) {
        let g = self.gain as f32;
        let mut a = self.dc_l.process(l * g);
        let mut b = self.dc_r.process(r * g);
        if self.drive > 0.0 {
            let drive = self.drive as f32;
            let d = 1.0 + drive * 4.0;
            let make = 1.0 + drive * 0.5;
            let sat = |x: f32| soft_clip(x * d) / make;
            if self.os.factor() > 1 {
                self.os.up(a, &mut self.os_l);
                self.os.up(b, &mut self.os_r);
                let n = self.os.factor();
                for i in 0..n {
                    self.os_l[i] = sat(self.os_l[i]);
                    self.os_r[i] = sat(self.os_r[i]);
                }
                a = self.os.down(&self.os_l);
                b = self.os.down(&self.os_r);
            } else {
                a = sat(a);
                b = sat(b);
            }
        }
        let w = self.width.clamp(0.0, 2.0) as f32;
        let mid = (a + b) * 0.5;
        let side = (a - b) * 0.5 * w;
        (mid + side, mid - side)
    }
}
