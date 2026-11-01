//! Small shared DSP helpers: deterministic RNG, saturators, a DC blocker,
//! parameter smoothing, gain/pan math, f64 biquads and an oversampler.
//!
//! Everything that carries state between samples keeps that state in `f64`:
//! single-precision recursive filters at low normalised frequencies (a 20 Hz
//! DC blocker at 96 kHz, an anti-alias filter at 8× oversampling) accumulate
//! rounding error that is audible as hiss. Signals stay `f32` at the edges.

/// Values smaller than this are flushed to zero in recursive state so decaying
/// tails never turn into denormals (slow) or a perpetual noise floor.
pub const DENORMAL: f64 = 1e-20;

/// Put the current thread's FPU in flush-to-zero / denormals-are-zero mode
/// (x86-64 SSE). Decaying filter and reverb tails otherwise drift into
/// denormal numbers, which are 10–100× slower to compute — the difference
/// between a render that flies and one that crawls through silence. Results
/// are unaffected (denormals are below −700 dBFS). Call at the start of every
/// render thread.
pub fn denormals_off() {
    #[cfg(target_arch = "x86_64")]
    #[allow(deprecated)]
    unsafe {
        use std::arch::x86_64::{_mm_getcsr, _mm_setcsr};
        // FTZ (bit 15) | DAZ (bit 6).
        _mm_setcsr(_mm_getcsr() | 0x8040);
    }
}

/// Flush a tiny value to exact zero.
#[inline]
pub fn flush(x: f64) -> f64 {
    if x.abs() < DENORMAL {
        0.0
    } else {
        x
    }
}

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

    /// Uniform in [-1, 1) at double precision.
    #[inline]
    pub fn bipolar64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (2.0 / 9_007_199_254_740_992.0) - 1.0
    }
}

/// Soft clip: a rational `tanh` approximation, exact at the ±3 knee.
#[inline]
pub fn soft_clip(x: f32) -> f32 {
    soft_clip64(x as f64) as f32
}

#[inline]
pub fn soft_clip64(x: f64) -> f64 {
    if x <= -3.0 {
        -1.0
    } else if x >= 3.0 {
        1.0
    } else {
        x * (27.0 + x * x) / (27.0 + 9.0 * x * x)
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

/// Removes DC / sub-audio content: a first-order high-pass at `hz`.
#[derive(Clone)]
pub struct DcBlocker {
    x1: f64,
    y1: f64,
    r: f64,
    hz: f64,
}

impl Default for DcBlocker {
    fn default() -> Self {
        Self::new(10.0, super::SR)
    }
}

impl DcBlocker {
    pub fn new(hz: f64, sr: f64) -> Self {
        let mut d = Self { x1: 0.0, y1: 0.0, r: 0.0, hz };
        d.set_sample_rate(sr);
        d
    }

    pub fn set_sample_rate(&mut self, sr: f64) {
        self.r = (-2.0 * std::f64::consts::PI * self.hz / sr.max(1.0)).exp();
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        self.process64(x as f64) as f32
    }

    #[inline]
    pub fn process64(&mut self, x: f64) -> f64 {
        let y = x - self.x1 + self.r * self.y1;
        self.x1 = x;
        self.y1 = flush(y);
        self.y1
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
    target: f64,
    coef: f64,
}

impl Smoother {
    pub fn new(initial: f64, seconds: f64) -> Self {
        Self::with_sr(initial, seconds, super::SR)
    }

    pub fn with_sr(initial: f64, seconds: f64, sr: f64) -> Self {
        let coef = if seconds <= 0.0 { 1.0 } else { 1.0 - (-1.0 / (seconds * sr)).exp() };
        Self { value: initial, target: initial, coef }
    }

    /// Set the target and advance one sample toward it.
    #[inline]
    pub fn set_target(&mut self, target: f64) {
        self.target = target;
        self.next();
    }

    /// Set the target without advancing.
    #[inline]
    pub fn aim(&mut self, target: f64) {
        self.target = target;
    }

    /// Advance one sample and return the new value.
    #[inline]
    pub fn next(&mut self) -> f64 {
        let d = self.target - self.value;
        if d.abs() < 1e-12 {
            self.value = self.target;
        } else {
            self.value += d * self.coef;
        }
        self.value
    }

    #[inline]
    pub fn value(&self) -> f64 {
        self.value
    }

    pub fn snap(&mut self, v: f64) {
        self.value = v;
        self.target = v;
    }

    pub fn is_settled(&self) -> bool {
        (self.target - self.value).abs() < 1e-9
    }
}

/// Equal-power (−3 dB centre) stereo pan gains for a pan in [-1, 1].
#[inline]
pub fn pan_gains(pan: f64) -> (f32, f32) {
    let p = pan.clamp(-1.0, 1.0);
    let a = (p + 1.0) * std::f64::consts::FRAC_PI_4;
    // Normalised so centre is unity on both sides (a mono source panned to the
    // centre keeps its level); hard left/right then gain +3 dB on one side.
    let k = std::f64::consts::SQRT_2;
    ((a.cos() * k) as f32, (a.sin() * k) as f32)
}

/// Stereo balance for an already-stereo signal: centre = unity on both sides,
/// turning toward one side attenuates the other (like a console's balance).
#[inline]
pub fn balance_gains(pan: f64) -> (f32, f32) {
    let p = pan.clamp(-1.0, 1.0);
    if p >= 0.0 {
        ((1.0 - p) as f32, 1.0)
    } else {
        (1.0, (1.0 + p) as f32)
    }
}

#[inline]
pub fn db_to_gain(db: f64) -> f32 {
    10f64.powf(db / 20.0) as f32
}

#[inline]
pub fn gain_to_db(g: f32) -> f32 {
    20.0 * g.max(1e-9).log10()
}

/// Frequency ratio for a detune in cents.
#[inline]
pub fn cents_ratio(cents: f64) -> f64 {
    (cents * (std::f64::consts::LN_2 / 1200.0)).exp()
}

/// A transposed-direct-form-II biquad with `f64` coefficients and state.
#[derive(Clone, Default)]
pub struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    z1: f64,
    z2: f64,
}

/// Coeffs normalised by a0.
fn norm(b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64) -> Biquad {
    Biquad { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0, z1: 0.0, z2: 0.0 }
}

fn w0(fc: f64, sr: f64) -> f64 {
    2.0 * std::f64::consts::PI * (fc / sr).clamp(1e-7, 0.499)
}

impl Biquad {
    /// RBJ low-pass at `fc` (Q 0.7071 = Butterworth).
    pub fn lowpass(fc: f32, q: f32, sr: f32) -> Self {
        Self::lowpass64(fc as f64, q as f64, sr as f64)
    }

    pub fn lowpass64(fc: f64, q: f64, sr: f64) -> Self {
        let w = w0(fc, sr);
        let (sn, cs) = (w.sin(), w.cos());
        let alpha = sn / (2.0 * q.max(0.05));
        norm((1.0 - cs) * 0.5, 1.0 - cs, (1.0 - cs) * 0.5, 1.0 + alpha, -2.0 * cs, 1.0 - alpha)
    }

    /// RBJ high-pass.
    pub fn highpass(fc: f32, q: f32, sr: f32) -> Self {
        Self::highpass64(fc as f64, q as f64, sr as f64)
    }

    pub fn highpass64(fc: f64, q: f64, sr: f64) -> Self {
        let w = w0(fc, sr);
        let (sn, cs) = (w.sin(), w.cos());
        let alpha = sn / (2.0 * q.max(0.05));
        norm((1.0 + cs) * 0.5, -(1.0 + cs), (1.0 + cs) * 0.5, 1.0 + alpha, -2.0 * cs, 1.0 - alpha)
    }

    /// RBJ constant-skirt band-pass (peak gain = Q).
    pub fn bandpass64(fc: f64, q: f64, sr: f64) -> Self {
        let w = w0(fc, sr);
        let (sn, cs) = (w.sin(), w.cos());
        let alpha = sn / (2.0 * q.max(0.05));
        norm(alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * cs, 1.0 - alpha)
    }

    /// RBJ low shelf: `gain_db` applied below `fc`.
    pub fn low_shelf(fc: f32, gain_db: f32, sr: f32) -> Self {
        Self::low_shelf64(fc as f64, gain_db as f64, sr as f64)
    }

    pub fn low_shelf64(fc: f64, gain_db: f64, sr: f64) -> Self {
        let a = 10f64.powf(gain_db / 40.0);
        let w = w0(fc, sr);
        let (sn, cs) = (w.sin(), w.cos());
        let alpha = sn / 2.0 * std::f64::consts::SQRT_2;
        let t = 2.0 * a.sqrt() * alpha;
        norm(
            a * ((a + 1.0) - (a - 1.0) * cs + t),
            2.0 * a * ((a - 1.0) - (a + 1.0) * cs),
            a * ((a + 1.0) - (a - 1.0) * cs - t),
            (a + 1.0) + (a - 1.0) * cs + t,
            -2.0 * ((a - 1.0) + (a + 1.0) * cs),
            (a + 1.0) + (a - 1.0) * cs - t,
        )
    }

    /// RBJ high shelf: `gain_db` applied above `fc`.
    pub fn high_shelf(fc: f32, gain_db: f32, sr: f32) -> Self {
        Self::high_shelf64(fc as f64, gain_db as f64, sr as f64)
    }

    pub fn high_shelf64(fc: f64, gain_db: f64, sr: f64) -> Self {
        let a = 10f64.powf(gain_db / 40.0);
        let w = w0(fc, sr);
        let (sn, cs) = (w.sin(), w.cos());
        let alpha = sn / 2.0 * std::f64::consts::SQRT_2;
        let t = 2.0 * a.sqrt() * alpha;
        norm(
            a * ((a + 1.0) + (a - 1.0) * cs + t),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * cs),
            a * ((a + 1.0) + (a - 1.0) * cs - t),
            (a + 1.0) - (a - 1.0) * cs + t,
            2.0 * ((a - 1.0) - (a + 1.0) * cs),
            (a + 1.0) - (a - 1.0) * cs - t,
        )
    }

    /// RBJ peaking EQ.
    pub fn peak(fc: f32, gain_db: f32, q: f32, sr: f32) -> Self {
        Self::peak64(fc as f64, gain_db as f64, q as f64, sr as f64)
    }

    pub fn peak64(fc: f64, gain_db: f64, q: f64, sr: f64) -> Self {
        let a = 10f64.powf(gain_db / 40.0);
        let w = w0(fc, sr);
        let (sn, cs) = (w.sin(), w.cos());
        let alpha = sn / (2.0 * q.max(0.05));
        norm(1.0 + alpha * a, -2.0 * cs, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * cs, 1.0 - alpha / a)
    }

    /// Replace the coefficients but keep the state (for smooth retuning).
    pub fn retune(&mut self, other: &Biquad) {
        self.b0 = other.b0;
        self.b1 = other.b1;
        self.b2 = other.b2;
        self.a1 = other.a1;
        self.a2 = other.a2;
    }

    #[inline]
    pub fn tick(&mut self, x: f32) -> f32 {
        self.tick64(x as f64) as f32
    }

    #[inline]
    pub fn tick64(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.z1;
        self.z1 = flush(self.b1 * x - self.a1 * y + self.z2);
        self.z2 = flush(self.b2 * x - self.a2 * y);
        y
    }

    pub fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }
}

/// Oversampling for a nonlinear stage, at `factor`× the base rate.
///
/// Up: zero-stuffing (×factor gain) followed by an 8th-order Butterworth
/// low-pass at 0.45 × the base Nyquist; down: the same filter, then
/// decimation. All in `f64`, so even 8× at 192 kHz stays numerically quiet.
/// `factor == 1` is a bit-transparent pass-through.
#[derive(Clone)]
pub struct Oversampler {
    factor: usize,
    up: [Biquad; 4],
    down: [Biquad; 4],
}

impl Default for Oversampler {
    fn default() -> Self {
        Self::new(super::SR, 2)
    }
}

/// Q values of the four second-order sections of an 8th-order Butterworth.
const BUTTER8_Q: [f64; 4] = [0.509_795_6, 0.601_344_9, 0.899_976_1, 2.562_915_4];

impl Oversampler {
    pub fn new(base_sr: f64, factor: usize) -> Self {
        let factor = factor.clamp(1, 8);
        let hr = base_sr * factor as f64;
        // 0.9 × base Nyquist, but never above ~21.6 kHz: at high base rates
        // there is nothing audible up there worth the steeper transition.
        let fc = (base_sr * 0.45).min(21_600.0);
        let mk = || std::array::from_fn(|i| Biquad::lowpass64(fc, BUTTER8_Q[i], hr));
        Self { factor, up: mk(), down: mk() }
    }

    #[inline]
    pub fn factor(&self) -> usize {
        self.factor
    }

    /// Upsample one base-rate sample into `out[..factor]`.
    #[inline]
    pub fn up(&mut self, x: f32, out: &mut [f32]) {
        let n = self.factor;
        if n == 1 {
            out[0] = x;
            return;
        }
        for (i, o) in out.iter_mut().enumerate().take(n) {
            let mut v = if i == 0 { x as f64 * n as f64 } else { 0.0 };
            for s in self.up.iter_mut() {
                v = s.tick64(v);
            }
            *o = v as f32;
        }
    }

    /// Decimate `input[..factor]` back to one base-rate sample.
    #[inline]
    pub fn down(&mut self, input: &[f32]) -> f32 {
        let n = self.factor;
        if n == 1 {
            return input[0];
        }
        let mut first = 0.0;
        for (i, x) in input.iter().enumerate().take(n) {
            let mut v = *x as f64;
            for s in self.down.iter_mut() {
                v = s.tick64(v);
            }
            if i == 0 {
                first = v;
            }
        }
        first as f32
    }

    pub fn reset(&mut self) {
        for s in self.up.iter_mut().chain(self.down.iter_mut()) {
            s.reset();
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pan_centre_is_unity() {
        let (l, r) = pan_gains(0.0);
        assert!((l - 1.0).abs() < 1e-6 && (r - 1.0).abs() < 1e-6);
        let (l, r) = pan_gains(-1.0);
        assert!(r.abs() < 1e-6 && l > 1.4);
    }

    #[test]
    fn dc_blocker_removes_offset() {
        let mut d = DcBlocker::new(10.0, 48_000.0);
        let mut y = 0.0;
        for _ in 0..48_000 {
            y = d.process(0.5);
        }
        assert!(y.abs() < 1e-3, "residual dc {y}");
    }

    #[test]
    fn oversampler_passes_audio_and_is_quiet() {
        let sr = 44_100.0;
        let mut os = Oversampler::new(sr, 4);
        let mut buf = [0.0f32; 4];
        let mut peak = 0.0f32;
        for i in 0..44_100 {
            let x = (2.0 * std::f64::consts::PI * 1000.0 * i as f64 / sr).sin() as f32 * 0.5;
            os.up(x, &mut buf);
            let y = os.down(&buf);
            if i > 4410 {
                peak = peak.max(y.abs());
            }
        }
        assert!((peak - 0.5).abs() < 0.03, "passband gain wrong: {peak}");
        // Silence after the signal must decay to exact zero.
        let mut last = 1.0f32;
        for _ in 0..44_100 {
            os.up(0.0, &mut buf);
            last = os.down(&buf);
        }
        assert!(last.abs() < 1e-12, "tail {last}");
    }

    #[test]
    fn smoother_reaches_target() {
        let mut s = Smoother::with_sr(0.0, 0.01, 48_000.0);
        s.aim(1.0);
        for _ in 0..48_000 {
            s.next();
        }
        assert!((s.value() - 1.0).abs() < 1e-9);
    }
}
