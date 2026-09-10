//! Small shared DSP helpers: deterministic RNG, saturators, a DC blocker, a
//! parameter smoother and gain/pan math.

/// xorshift64* — tiny, fast, deterministic. Every noise source in the engine
/// owns one seeded from the pattern so renders are reproducible.
#[derive(Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in [-1, 1).
    #[inline]
    pub fn bipolar(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 * (2.0 / 16_777_216.0) - 1.0
    }

    /// Uniform in [0, 1).
    #[inline]
    pub fn unipolar(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 * (1.0 / 16_777_216.0)
    }
}

/// Soft clip: `tanh` but cheaper and with a gentler knee.
#[inline]
pub fn soft_clip(x: f32) -> f32 {
    if x <= -3.0 {
        -1.0
    } else if x >= 3.0 {
        1.0
    } else {
        x * (27.0 + x * x) / (27.0 + 9.0 * x * x)
    }
}

/// Hard-ish clipper with a 3 dB rounded knee — used for drum bus glue.
#[inline]
pub fn clip_knee(x: f32) -> f32 {
    if x <= -1.5 {
        -1.0
    } else if x >= 1.5 {
        1.0
    } else if x <= -0.5 {
        -0.5 + (x + 0.5) * 0.5
    } else if x >= 0.5 {
        0.5 + (x - 0.5) * 0.5
    } else {
        x
    }
}

/// Asymmetric tube-style saturation (adds even harmonics).
#[inline]
pub fn tube(x: f32) -> f32 {
    if x >= 0.0 {
        x / (1.0 + x)
    } else {
        x / (1.0 - x)
    }
}

/// Removes DC / very low frequency content. One pole at ~12 Hz.
#[derive(Clone, Default)]
pub struct DcBlocker {
    x1: f32,
    y1: f32,
}

impl DcBlocker {
    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let y = x - self.x1 + 0.9985 * self.y1;
        self.x1 = x;
        self.y1 = y;
        y
    }

    pub fn reset(&mut self) {
        self.x1 = 0.0;
        self.y1 = 0.0;
    }
}

/// A one-pole parameter smoother (zipper-free knob moves).
#[derive(Clone)]
pub struct Smoother {
    value: f64,
    coef: f64,
}

impl Smoother {
    pub fn new(initial: f64, seconds: f64) -> Self {
        let coef = if seconds <= 0.0 { 1.0 } else { 1.0 - (-1.0 / (seconds * super::SR)).exp() };
        Self { value: initial, coef }
    }

    #[inline]
    pub fn set_target(&mut self, target: f64) {
        self.value += (target - self.value) * self.coef;
    }

    #[inline]
    pub fn value(&self) -> f64 {
        self.value
    }

    pub fn snap(&mut self, v: f64) {
        self.value = v;
    }
}

/// Equal-power stereo pan gains for a pan in [-1, 1].
#[inline]
pub fn pan_gains(pan: f64) -> (f32, f32) {
    let p = pan.clamp(-1.0, 1.0) as f32;
    let a = (p + 1.0) * std::f32::consts::FRAC_PI_4;
    (a.cos(), a.sin())
}

#[inline]
pub fn db_to_gain(db: f64) -> f32 {
    10f32.powf((db / 20.0) as f32)
}

#[inline]
pub fn gain_to_db(g: f32) -> f32 {
    20.0 * g.max(1e-9).log10()
}

/// MIDI note number → Hz (A4 = 69 = 440 Hz).
#[inline]
pub fn midi_to_hz(note: f64) -> f64 {
    440.0 * 2f64.powf((note - 69.0) / 12.0)
}

/// Frequency ratio for a detune in cents.
#[inline]
pub fn cents_ratio(cents: f64) -> f64 {
    2f64.powf(cents / 1200.0)
}

/// A transposed-direct-form-II biquad: the workhorse for EQ bands and the
/// oversampler's anti-imaging filters.
#[derive(Clone, Default)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

/// Coeffs normalised by a0.
fn norm(b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) -> Biquad {
    Biquad { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0, z1: 0.0, z2: 0.0 }
}

impl Biquad {
    /// Butterworth low-pass at `fc` for sample rate `sr`.
    pub fn lowpass(fc: f32, q: f32, sr: f32) -> Self {
        let w0 = 2.0 * std::f32::consts::PI * (fc / sr).clamp(1e-6, 0.499);
        let (sn, cs) = (w0.sin(), w0.cos());
        let alpha = sn / (2.0 * q);
        norm((1.0 - cs) * 0.5, 1.0 - cs, (1.0 - cs) * 0.5, 1.0 + alpha, -2.0 * cs, 1.0 - alpha)
    }

    /// RBJ low shelf: `gain_db` applied below `fc`.
    pub fn low_shelf(fc: f32, gain_db: f32, sr: f32) -> Self {
        let a = 10f32.powf(gain_db / 40.0);
        let w0 = 2.0 * std::f32::consts::PI * (fc / sr).clamp(1e-6, 0.499);
        let (sn, cs) = (w0.sin(), w0.cos());
        let alpha = sn / 2.0 * (2.0f32).sqrt();
        let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;
        norm(
            a * ((a + 1.0) - (a - 1.0) * cs + two_sqrt_a_alpha),
            2.0 * a * ((a - 1.0) - (a + 1.0) * cs),
            a * ((a + 1.0) - (a - 1.0) * cs - two_sqrt_a_alpha),
            (a + 1.0) + (a - 1.0) * cs + two_sqrt_a_alpha,
            -2.0 * ((a - 1.0) + (a + 1.0) * cs),
            (a + 1.0) + (a - 1.0) * cs - two_sqrt_a_alpha,
        )
    }

    /// RBJ high shelf: `gain_db` applied above `fc`.
    pub fn high_shelf(fc: f32, gain_db: f32, sr: f32) -> Self {
        let a = 10f32.powf(gain_db / 40.0);
        let w0 = 2.0 * std::f32::consts::PI * (fc / sr).clamp(1e-6, 0.499);
        let (sn, cs) = (w0.sin(), w0.cos());
        let alpha = sn / 2.0 * (2.0f32).sqrt();
        let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;
        norm(
            a * ((a + 1.0) + (a - 1.0) * cs + two_sqrt_a_alpha),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * cs),
            a * ((a + 1.0) + (a - 1.0) * cs - two_sqrt_a_alpha),
            (a + 1.0) - (a - 1.0) * cs + two_sqrt_a_alpha,
            2.0 * ((a - 1.0) - (a + 1.0) * cs),
            (a + 1.0) - (a - 1.0) * cs - two_sqrt_a_alpha,
        )
    }

    /// RBJ peaking EQ.
    pub fn peak(fc: f32, gain_db: f32, q: f32, sr: f32) -> Self {
        let a = 10f32.powf(gain_db / 40.0);
        let w0 = 2.0 * std::f32::consts::PI * (fc / sr).clamp(1e-6, 0.499);
        let (sn, cs) = (w0.sin(), w0.cos());
        let alpha = sn / (2.0 * q.max(0.05));
        norm(
            1.0 + alpha * a,
            -2.0 * cs,
            1.0 - alpha * a,
            1.0 + alpha / a,
            -2.0 * cs,
            1.0 - alpha / a,
        )
    }

    /// RBJ high-pass.
    pub fn highpass(fc: f32, q: f32, sr: f32) -> Self {
        let w0 = 2.0 * std::f32::consts::PI * (fc / sr).clamp(1e-6, 0.499);
        let (sn, cs) = (w0.sin(), w0.cos());
        let alpha = sn / (2.0 * q);
        norm((1.0 + cs) * 0.5, -(1.0 + cs), (1.0 + cs) * 0.5, 1.0 + alpha, -2.0 * cs, 1.0 - alpha)
    }

    #[inline]
    pub fn tick(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }

    pub fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }
}

/// 2× oversampling for nonlinear stages (distortion, drum drive, limiter).
///
/// A zero-stuffed signal fed straight into `tanh` folds every spectral image
/// right back into the audible band, so both the interpolation filter (up) and
/// the decimation filter (down) matter. We use two cascaded Butterworth
/// sections (4th order) at 0.45 × base Nyquist, running at 2×.
#[derive(Clone)]
pub struct Oversampler2x {
    up: [Biquad; 2],
    down: [Biquad; 2],
    prev: f32,
}

impl Default for Oversampler2x {
    fn default() -> Self {
        Self::new()
    }
}

impl Oversampler2x {
    pub fn new() -> Self {
        let sr2 = (super::SR * 2.0) as f32;
        let fc = (super::SR as f32) * 0.5 * 0.45;
        Self {
            up: [Biquad::lowpass(fc, 0.541_2, sr2), Biquad::lowpass(fc, 1.306_6, sr2)],
            down: [Biquad::lowpass(fc, 0.541_2, sr2), Biquad::lowpass(fc, 1.306_6, sr2)],
            prev: 0.0,
        }
    }

    /// Upsample one base-rate sample to two 2×-rate samples.
    #[inline]
    pub fn up(&mut self, x: f32) -> (f32, f32) {
        // Linear interpolation between the previous and current input is a poor
        // image filter on its own, but it halves the work of the FIR that
        // follows; the Butterworth cascade removes the rest.
        let mid = 0.5 * (self.prev + x);
        self.prev = x;
        let a0 = self.up[0].tick(x);
        let a = self.up[1].tick(a0);
        let b0 = self.up[0].tick(mid);
        let b = self.up[1].tick(b0);
        (a, b)
    }

    /// Decimate two 2×-rate samples back to one base-rate sample.
    #[inline]
    pub fn down(&mut self, a: f32, b: f32) -> f32 {
        let a0 = self.down[0].tick(a);
        let _ = self.down[1].tick(a0);
        let b0 = self.down[0].tick(b);
        self.down[1].tick(b0)
    }

    pub fn reset(&mut self) {
        for s in self.up.iter_mut().chain(self.down.iter_mut()) {
            s.reset();
        }
        self.prev = 0.0;
    }
}

/// Sample-accurate linear ramp used to apply per-block control values without
/// clicks.
#[inline]
pub fn ramp(start: f32, end: f32, i: usize, n: usize) -> f32 {
    if n <= 1 {
        end
    } else {
        start + (end - start) * (i as f32 / (n - 1) as f32)
    }
}
