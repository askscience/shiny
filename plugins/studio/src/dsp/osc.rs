//! Band-limited oscillators.
//!
//! Every periodic shape except the sine is played from a **mip-mapped
//! additive wavetable**: for each shape we precompute tables at half-octave
//! harmonic counts (1024, 724, 512, … 1 partials) and, per sample, pick the
//! richest table whose highest partial still sits below Nyquist for the
//! current pitch. Nothing can alias, at any pitch or sample rate, and because
//! each table is stored at ≥ 4 samples per period of its top partial, linear
//! interpolation adds no audible error (≤ −90 dB).
//!
//! Pulse/PWM is the difference of two phase-shifted saws, so it is exactly as
//! clean as the saw at every width. The sine is computed directly in `f64`
//! (it is also the FM carrier). Shape indices are part of the saved-config
//! contract and keep their numbers; the former "PolyBLEP" variants are now
//! aliases of the wavetable shapes (they existed only as cheaper fallbacks).

use std::f64::consts::TAU;
use std::sync::OnceLock;

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
    /// Digital white noise.
    Noise,
    /// Drawbar-style "organ" wave (8' + 4' + 2⅔' + 2').
    Organ,
    /// Classic "888" Hammond registration (16' + 5⅓' + 8').
    Hammond,
    /// Saw whose partials fall off like a triangle (1/k²): mellow and round.
    SoftSaw,
    /// Kept for saved configs: identical to `Saw`.
    PolySaw,
    /// Kept for saved configs: identical to `Square`.
    PolySquare,
    /// Kept for saved configs: identical to `Pulse`.
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

    pub fn index(self) -> f64 {
        match self {
            Wave::Sine => 0.0,
            Wave::Triangle => 1.0,
            Wave::Saw => 2.0,
            Wave::Square => 3.0,
            Wave::Pulse => 4.0,
            Wave::Noise => 5.0,
            Wave::Organ => 6.0,
            Wave::Hammond => 7.0,
            Wave::SoftSaw => 8.0,
            Wave::PolySaw => 9.0,
            Wave::PolySquare => 10.0,
            Wave::PolyPulse => 11.0,
        }
    }

    /// The table-backed shape this wave plays from, if any.
    fn table(self) -> Option<Shape> {
        match self {
            Wave::Saw | Wave::PolySaw => Some(Shape::Saw),
            Wave::Square | Wave::PolySquare => Some(Shape::Square),
            Wave::Triangle => Some(Shape::Triangle),
            Wave::Organ => Some(Shape::Organ),
            Wave::Hammond => Some(Shape::Hammond),
            Wave::SoftSaw => Some(Shape::SoftSaw),
            Wave::Pulse | Wave::PolyPulse => Some(Shape::Saw),
            Wave::Sine | Wave::Noise => None,
        }
    }

    fn is_pulse(self) -> bool {
        matches!(self, Wave::Pulse | Wave::PolyPulse)
    }

    /// Hammond's table period is the 16' footage — one octave below the note.
    fn period_div(self) -> f64 {
        if self == Wave::Hammond {
            0.5
        } else {
            1.0
        }
    }
}

/* ── Wavetables ─────────────────────────────────────────────── */

#[derive(Clone, Copy, PartialEq, Eq)]
enum Shape {
    Saw,
    Square,
    Triangle,
    Organ,
    Hammond,
    SoftSaw,
}

const SHAPES: usize = 6;

/// One band-limited table: `size` samples (a power of two) plus one guard
/// sample, holding partials 1..=`harmonics`.
struct Table {
    harmonics: usize,
    mask: usize,
    size: f64,
    data: Vec<f32>,
}

struct Bank {
    /// Tables ordered from the richest to the poorest.
    tables: Vec<Table>,
}

/// Partial amplitude (sine phase) of shape `s` at harmonic `k` (1-based).
fn partial(s: Shape, k: usize) -> f64 {
    let kf = k as f64;
    match s {
        // Rising saw: −(2/π) Σ (−1)^k sin(kx)/k … we use the falling-edge
        // convention 2/π Σ sin(kx)/k and negate so the ramp rises.
        Shape::Saw => -(2.0 / std::f64::consts::PI) / kf,
        Shape::Square => {
            if k % 2 == 1 {
                (4.0 / std::f64::consts::PI) / kf
            } else {
                0.0
            }
        }
        Shape::Triangle => {
            if k % 2 == 1 {
                let sign = if (k / 2) % 2 == 0 { 1.0 } else { -1.0 };
                sign * (8.0 / (std::f64::consts::PI * std::f64::consts::PI)) / (kf * kf)
            } else {
                0.0
            }
        }
        Shape::SoftSaw => 0.75 / (kf * kf),
        // 8' (1), 4' (2), 2⅔' (3), 2' (4), plus a whisper of 1⅗' (5).
        Shape::Organ => match k {
            1 => 1.0,
            2 => 0.6,
            3 => 0.45,
            4 => 0.35,
            5 => 0.12,
            6 => 0.18,
            8 => 0.12,
            _ => 0.0,
        },
        // Relative to the 16' fundamental: 16' (1), 8' (2), 5⅓' (3), with a
        // little key-click-free top (4', 2⅔') for presence.
        Shape::Hammond => match k {
            1 => 0.8,
            2 => 1.0,
            3 => 0.8,
            4 => 0.18,
            6 => 0.12,
            _ => 0.0,
        },
    }
}

fn shape_of(i: usize) -> Shape {
    [Shape::Saw, Shape::Square, Shape::Triangle, Shape::Organ, Shape::Hammond, Shape::SoftSaw][i]
}

fn shape_index(s: Shape) -> usize {
    match s {
        Shape::Saw => 0,
        Shape::Square => 1,
        Shape::Triangle => 2,
        Shape::Organ => 3,
        Shape::Hammond => 4,
        Shape::SoftSaw => 5,
    }
}

fn build_bank(s: Shape) -> Bank {
    // Half-octave harmonic counts from 1024 down to 1.
    let mut counts: Vec<usize> = Vec::new();
    let mut k = 0;
    loop {
        let h = (1024.0 / 2f64.powf(k as f64 * 0.5)).floor() as usize;
        if h < 1 {
            break;
        }
        if counts.last() != Some(&h) {
            counts.push(h);
        }
        if h == 1 {
            break;
        }
        k += 1;
    }
    // A single normalisation for the whole bank (from the richest table) so
    // switching tables never changes the level.
    let mut tables: Vec<Table> = Vec::with_capacity(counts.len());
    for &h in &counts {
        let size = (h * 4).next_power_of_two().clamp(256, 4096);
        let mut sin_tab = Vec::with_capacity(size);
        for n in 0..size {
            sin_tab.push((TAU * n as f64 / size as f64).sin());
        }
        let mut data = vec![0.0f64; size];
        for kk in 1..=h {
            let a = partial(s, kk);
            if a == 0.0 {
                continue;
            }
            let mut idx = 0usize;
            for d in data.iter_mut() {
                *d += a * sin_tab[idx];
                idx = (idx + kk) & (size - 1);
            }
        }
        let mut v: Vec<f32> = data.iter().map(|x| *x as f32).collect();
        v.push(v[0]);
        tables.push(Table { harmonics: h, mask: size - 1, size: size as f64, data: v });
    }
    let peak = tables[0].data.iter().fold(0.0f32, |m, x| m.max(x.abs())).max(1e-9);
    // Saw/square keep their natural Gibbs overshoot relative to a unit ramp;
    // the additive shapes are normalised to a unit peak.
    let norm = match s {
        Shape::Saw | Shape::Square | Shape::Triangle => 1.0,
        _ => 1.0 / peak,
    };
    if norm != 1.0 {
        for t in tables.iter_mut() {
            for x in t.data.iter_mut() {
                *x *= norm;
            }
        }
    }
    Bank { tables }
}

fn banks() -> &'static [Bank] {
    static BANKS: OnceLock<Vec<Bank>> = OnceLock::new();
    BANKS.get_or_init(|| (0..SHAPES).map(|i| build_bank(shape_of(i))).collect())
}

impl Bank {
    /// The richest table whose top partial stays below Nyquist for `dt`
    /// (cycles per sample).
    /// Index of the richest table whose top partial stays below Nyquist.
    #[inline]
    fn level(&self, dt: f64) -> usize {
        let max_h = if dt <= 0.0 { usize::MAX } else { (0.5 / dt) as usize };
        for (i, t) in self.tables.iter().enumerate() {
            if t.harmonics <= max_h {
                return i;
            }
        }
        self.tables.len() - 1
    }
}

impl Table {
    #[inline]
    fn read(&self, phase: f64) -> f64 {
        let pos = phase * self.size;
        let i = pos as usize;
        let frac = pos - i as f64;
        let i = i & self.mask;
        let a = self.data[i] as f64;
        let b = self.data[i + 1] as f64;
        a + (b - a) * frac
    }
}

/* ── Oscillator ─────────────────────────────────────────────── */

/// A single free-running oscillator with per-sample frequency input.
#[derive(Clone)]
pub struct Osc {
    phase: f64,
    /// Public for the Grid's parameter editor.
    pub wave: Wave,
    pub pulse_width: f64,
    /// Phase the oscillator restarts from on `reset`.
    offset: f64,
    rng: Rng,
    seed: u64,
    /// Cached mip level for `cached_dt` (picking is a search; pitch rarely
    /// changes per sample).
    cached_dt: f64,
    cached_level: usize,
}

impl Osc {
    pub fn new(wave: Wave) -> Self {
        // Touch the tables now so the first note never pays for building them.
        let _ = banks();
        let seed = 0x1234_5678u64;
        Self { phase: 0.0, wave, pulse_width: 0.5, offset: 0.0, rng: Rng::new(seed), seed, cached_dt: -1.0, cached_level: 0 }
    }

    pub fn with_seed(mut self, seed: u64) -> Self {
        self.rng = Rng::new(seed);
        self.seed = seed;
        self
    }

    pub fn set_phase(&mut self, p: f64) {
        self.phase = p.rem_euclid(1.0);
        self.offset = self.phase;
    }

    /// Randomise the start phase (unison / supersaw decorrelation).
    pub fn randomize_phase(&mut self, seed: u64) {
        self.rng = Rng::new(seed);
        self.seed = seed;
        let p = self.rng.unipolar() as f64;
        self.set_phase(p);
    }

    pub fn reset(&mut self) {
        self.phase = self.offset;
    }

    pub fn phase(&self) -> f64 {
        self.phase
    }

    /// Generate one sample for a frequency in Hz at sample rate `sr`.
    #[inline]
    pub fn tick(&mut self, freq: f64, sr: f64) -> f32 {
        self.tick_pm(freq, sr, 0.0)
    }

    /// Phase-modulated sample: `pm` is a phase offset in cycles (FM).
    #[inline]
    pub fn tick_pm(&mut self, freq: f64, sr: f64, pm: f64) -> f32 {
        let dt = (freq.abs() / sr.max(1.0) * self.wave.period_div()).min(0.5);
        let p = if pm == 0.0 { self.phase } else { (self.phase + pm).rem_euclid(1.0) };
        let out = match self.wave {
            Wave::Sine => (TAU * p).sin(),
            Wave::Noise => self.rng.bipolar64(),
            w => {
                let shape = w.table().unwrap_or(Shape::Saw);
                let bank = &banks()[shape_index(shape)];
                if dt != self.cached_dt {
                    self.cached_dt = dt;
                    self.cached_level = bank.level(dt);
                }
                let t = &bank.tables[self.cached_level.min(bank.tables.len() - 1)];
                if w.is_pulse() {
                    let width = self.pulse_width.clamp(0.02, 0.98);
                    let mut q = p - width;
                    if q < 0.0 {
                        q += 1.0;
                    }
                    // Difference of two saws: a band-limited pulse that is
                    // zero-mean at any width (levels 2w and 2w − 2).
                    t.read(p) - t.read(q)
                } else {
                    t.read(p)
                }
            }
        };
        self.phase += dt;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        out as f32
    }
}

/// A detuned unison stack with stereo spread — the difference between a thin
/// one-oscillator lead and a "real" supersaw.
#[derive(Clone)]
pub struct Unison {
    oscs: Vec<Osc>,
    ratios: Vec<f64>,
    pans: Vec<(f32, f32)>,
    gains: Vec<f32>,
    seed: u64,
    retriggers: u64,
}

impl Unison {
    /// `count` voices, `detune_cents` total spread, `spread` stereo width 0..1.
    pub fn new(count: usize, wave: Wave, detune_cents: f64, spread: f64, seed: u64) -> Self {
        let n = count.clamp(1, 16);
        let mut u = Self {
            oscs: Vec::with_capacity(n),
            ratios: Vec::with_capacity(n),
            pans: Vec::with_capacity(n),
            gains: Vec::with_capacity(n),
            seed,
            retriggers: 0,
        };
        for i in 0..n {
            let mut o = Osc::new(wave).with_seed(seed.wrapping_add(i as u64 * 7919));
            o.randomize_phase(seed.wrapping_add(i as u64 * 104_729));
            u.oscs.push(o);
        }
        u.configure(detune_cents, spread);
        u
    }

    /// Re-derive detune ratios and pan gains (no phase reset).
    pub fn configure(&mut self, detune_cents: f64, spread: f64) {
        let n = self.oscs.len();
        self.ratios.clear();
        self.pans.clear();
        self.gains.clear();
        for i in 0..n {
            // [-0.5, 0.5] → symmetric detune; the centre voice stays in tune.
            // A slightly non-linear spread (like a JP-8000 supersaw) keeps the
            // beating from sounding like a regular tremolo.
            let t = if n == 1 { 0.0 } else { i as f64 / (n - 1) as f64 - 0.5 };
            let curve = t * (0.6 + 0.8 * t.abs());
            self.ratios.push(cents_ratio(curve * detune_cents));
            let pan = (t * 2.0 * spread).clamp(-1.0, 1.0);
            // Equal-power pan, normalised so a centred voice is unity.
            let a = (pan + 1.0) * std::f64::consts::FRAC_PI_4;
            let k = std::f64::consts::SQRT_2;
            self.pans.push(((a.cos() * k) as f32, (a.sin() * k) as f32));
            self.gains.push((1.0 / (n as f64).sqrt()) as f32);
        }
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

    /// Restart the stack for a new note. A single oscillator restarts at
    /// phase 0 (punchy, repeatable attacks); a stack gets fresh random
    /// phases so consecutive notes don't all start with the same comb.
    pub fn reset(&mut self) {
        self.retriggers = self.retriggers.wrapping_add(1);
        if self.oscs.len() == 1 {
            self.oscs[0].set_phase(0.0);
            return;
        }
        let base = self.seed ^ self.retriggers.wrapping_mul(0x9E37_79B9);
        for (i, o) in self.oscs.iter_mut().enumerate() {
            o.randomize_phase(base.wrapping_add(i as u64 * 104_729));
        }
    }

    /// Render one sample into a stereo pair.
    #[inline]
    pub fn tick(&mut self, freq: f64, sr: f64) -> (f32, f32) {
        self.tick_pm(freq, sr, 0.0)
    }

    /// Phase-modulated stereo render (FM through a unison stack).
    #[inline]
    pub fn tick_pm(&mut self, freq: f64, sr: f64, pm: f64) -> (f32, f32) {
        let mut l = 0.0f32;
        let mut r = 0.0f32;
        for i in 0..self.oscs.len() {
            let s = self.oscs[i].tick_pm(freq * self.ratios[i], sr, pm) * self.gains[i];
            l += s * self.pans[i].0;
            r += s * self.pans[i].1;
        }
        (l, r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Energy of `x` at frequency `f` (single-bin DFT, normalised amplitude).
    fn tone(x: &[f32], f: f64, sr: f64) -> f64 {
        let (mut re, mut im) = (0.0, 0.0);
        for (n, s) in x.iter().enumerate() {
            let a = TAU * f * n as f64 / sr;
            re += *s as f64 * a.cos();
            im += *s as f64 * a.sin();
        }
        2.0 * (re * re + im * im).sqrt() / x.len() as f64
    }

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
            assert!(peak > 0.05 && peak < 1.3, "{w:?} peak {peak}");
        }
    }

    #[test]
    fn saw_does_not_alias() {
        // A 3520 Hz saw at 44.1 kHz: harmonics above Nyquist would fold back
        // to e.g. 44100 − 7·3520 = 19460 Hz… check a fold-back bin is silent
        // while the real 5th harmonic is present.
        let sr = 44_100.0;
        let f = 3520.0;
        let mut o = Osc::new(Wave::Saw);
        let x: Vec<f32> = (0..44_100).map(|_| o.tick(f, sr)).collect();
        let h5 = tone(&x, 5.0 * f, sr);
        let alias = tone(&x, sr - 7.0 * f, sr);
        assert!(h5 > 0.05, "missing harmonic: {h5}");
        assert!(alias < 1e-4, "aliasing at {}: {alias}", sr - 7.0 * f);
    }

    #[test]
    fn pulse_has_no_dc_at_any_width() {
        for w in [0.1, 0.25, 0.5, 0.8] {
            let mut o = Osc::new(Wave::Pulse);
            o.pulse_width = w;
            let n = 44_100;
            let mean: f64 = (0..n).map(|_| o.tick(100.0, 44_100.0) as f64).sum::<f64>() / n as f64;
            assert!(mean.abs() < 0.01, "pw {w}: dc {mean}");
        }
    }

    #[test]
    fn unison_voices_are_decorrelated() {
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
