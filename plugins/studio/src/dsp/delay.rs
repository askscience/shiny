//! Delay-line primitives and the time-based effects built on them.

use super::filter::OnePole;
use super::lfo::{Lfo, LfoShape};
use super::util::DcBlocker;

/// A circular buffer with fractional (linearly interpolated) reads.
///
/// Linear interpolation is the right call inside a feedback loop: it is
/// monotonic (no overshoot that could pump the feedback) and cheap, and the
/// high-frequency loss it introduces is exactly what you want in a delay.
#[derive(Clone)]
pub struct DelayLine {
    buf: Vec<f32>,
    write: usize,
    mask: usize,
}

impl DelayLine {
    pub fn new(max_frames: usize) -> Self {
        // Round up to a power of two so the wrap is a mask, not a modulo.
        let mut n = 1usize;
        while n < max_frames.max(4) {
            n <<= 1;
        }
        Self { buf: vec![0.0; n], write: 0, mask: n - 1 }
    }

    pub fn reset(&mut self) {
        for s in self.buf.iter_mut() {
            *s = 0.0;
        }
        self.write = 0;
    }

    #[inline]
    pub fn write(&mut self, x: f32) {
        self.buf[self.write] = x;
        self.write = (self.write + 1) & self.mask;
    }

    /// Read `delay` frames back from the write head (>= 1).
    #[inline]
    pub fn read(&self, delay: f32) -> f32 {
        let d = delay.clamp(1.0, (self.buf.len() - 2) as f32);
        let i = d.floor();
        let frac = d - i;
        let i0 = i as usize;
        let a = self.buf[(self.write + self.buf.len() - i0) & self.mask];
        let b = self.buf[(self.write + self.buf.len() - i0 - 1) & self.mask];
        a + (b - a) * frac
    }

    /// Integer read (no interpolation) — used for static delays.
    #[inline]
    pub fn read_int(&self, delay: usize) -> f32 {
        let d = delay.min(self.buf.len() - 1);
        self.buf[(self.write + self.buf.len() - d) & self.mask]
    }
}

/// Schroeder all-pass — used for input diffusion in the reverb.
#[derive(Clone)]
pub struct Allpass {
    line: DelayLine,
    delay: usize,
    g: f32,
}

impl Allpass {
    pub fn new(delay: usize, g: f32) -> Self {
        Self { line: DelayLine::new(delay + 4), delay, g }
    }

    pub fn reset(&mut self) {
        self.line.reset();
    }

    #[inline]
    pub fn tick(&mut self, x: f32) -> f32 {
        let d = self.line.read_int(self.delay);
        let v = x + d * self.g;
        self.line.write(v);
        d - v * self.g
    }
}

/// A stereo delay with feedback damping, ping-pong cross-feedback and a gently
/// modulated tap so the repeats never sit exactly on the grid.
#[derive(Clone)]
pub struct StereoDelay {
    l: DelayLine,
    r: DelayLine,
    damp_l: OnePole,
    damp_r: OnePole,
    dc_l: DcBlocker,
    dc_r: DcBlocker,
    lfo: Lfo,
    pub time_ms: f64,
    /// Ping-pong offset between the channels, in ms.
    pub offset_ms: f64,
    pub feedback: f64,
    pub mix: f64,
    /// 0 = parallel stereo, 1 = full ping-pong cross-feedback.
    pub ping_pong: f64,
    pub damp: f64,
    pub mod_depth: f64,
    sr: f64,
}

impl StereoDelay {
    pub fn new(time_ms: f64, feedback: f64, mix: f64) -> Self {
        Self {
            l: DelayLine::new(4 * 44100),
            r: DelayLine::new(4 * 44100),
            damp_l: OnePole::lowpass(6000.0, 44100.0),
            damp_r: OnePole::lowpass(6000.0, 44100.0),
            dc_l: DcBlocker::default(),
            dc_r: DcBlocker::default(),
            lfo: Lfo::new(LfoShape::Sine, 0xD314),
            time_ms,
            offset_ms: 0.0,
            feedback,
            mix,
            ping_pong: 0.0,
            damp: 0.35,
            mod_depth: 0.15,
            sr: 44100.0,
        }
    }

    pub fn set_sample_rate(&mut self, sr: f64) {
        self.sr = sr;
    }

    pub fn update_damping(&mut self) {
        // 0 → open (12 kHz), 1 → dark (900 Hz).
        let fc = 12000.0 * (0.075f64).powf(self.damp.clamp(0.0, 1.0));
        self.damp_l = OnePole::lowpass(fc, self.sr);
        self.damp_r = OnePole::lowpass(fc, self.sr);
    }

    pub fn reset(&mut self) {
        self.l.reset();
        self.r.reset();
        self.damp_l.reset();
        self.damp_r.reset();
        self.dc_l.reset();
        self.dc_r.reset();
    }

    #[inline]
    pub fn tick(&mut self, in_l: f32, in_r: f32) -> (f32, f32) {
        let base = (self.time_ms.clamp(1.0, 4000.0) * 0.001 * self.sr) as f32;
        let off = (self.offset_ms * 0.001 * self.sr) as f32;
        // Slow modulation widens the repeats; depth is in samples (≤ 12).
        let m = self.lfo.tick(0.13, self.sr) * (self.mod_depth as f32 * 12.0);
        let dl = (base - off * 0.5 + m).max(2.0);
        let dr = (base + off * 0.5 - m).max(2.0);

        let fb_l = self.damp_l.tick(self.l.read(dl));
        let fb_r = self.damp_r.tick(self.r.read(dr));
        let cross = self.ping_pong.clamp(0.0, 1.0) as f32;

        let wl = in_l + (fb_l * (1.0 - cross) + fb_r * cross) * self.feedback as f32;
        let wr = in_r + (fb_r * (1.0 - cross) + fb_l * cross) * self.feedback as f32;
        self.l.write(self.dc_l.process(wl));
        self.r.write(self.dc_r.process(wr));

        let mix = self.mix as f32;
        (in_l * (1.0 - mix) + fb_l * mix, in_r * (1.0 - mix) + fb_r * mix)
    }
}

/// A stereo chorus/ensemble: two modulated taps per side plus a short
/// cross-feedback-free voice, which is what gives a Juno-ish sheen.
#[derive(Clone)]
pub struct Chorus {
    l: DelayLine,
    r: DelayLine,
    lfo1: Lfo,
    lfo2: Lfo,
    pub rate: f64,
    pub depth: f64,
    pub mix: f64,
    pub spread: f64,
    sr: f64,
    dc_l: DcBlocker,
    dc_r: DcBlocker,
}

impl Chorus {
    pub fn new(rate: f64, depth: f64, mix: f64) -> Self {
        Self {
            l: DelayLine::new(4096),
            r: DelayLine::new(4096),
            lfo1: Lfo::new(LfoShape::Sine, 0xC40),
            lfo2: Lfo::new(LfoShape::Triangle, 0xC41),
            rate,
            depth,
            mix,
            spread: 0.6,
            sr: 44100.0,
            dc_l: DcBlocker::default(),
            dc_r: DcBlocker::default(),
        }
    }

    pub fn set_sample_rate(&mut self, sr: f64) {
        self.sr = sr;
    }

    pub fn reset(&mut self) {
        self.l.reset();
        self.r.reset();
        self.dc_l.reset();
        self.dc_r.reset();
    }

    #[inline]
    pub fn tick(&mut self, in_l: f32, in_r: f32) -> (f32, f32) {
        // Base delay ~12 ms so the chorus sits behind the note.
        let base = 0.012 * self.sr as f32;
        let depth = (self.depth.clamp(0.0, 1.0) as f32) * 0.010 * self.sr as f32;
        let a = self.lfo1.tick(self.rate.clamp(0.01, 12.0), self.sr);
        let b = self.lfo2.tick(self.rate.clamp(0.01, 12.0) * 1.37, self.sr);
        let spread = self.spread.clamp(0.0, 1.0) as f32;

        let dl = (base + a * depth * (1.0 + spread * 0.5)).max(1.0);
        let dr = (base - a * depth * (1.0 - spread * 0.5)).max(1.0);
        let dl2 = (base * 2.0 + b * depth).max(1.0);
        let dr2 = (base * 2.0 - b * depth).max(1.0);

        let wl = self.l.read(dl) * 0.5 + self.l.read(dl2) * 0.5;
        let wr = self.r.read(dr) * 0.5 + self.r.read(dr2) * 0.5;
        self.l.write(self.dc_l.process(in_l));
        self.r.write(self.dc_r.process(in_r));

        let mix = self.mix.clamp(0.0, 1.0) as f32;
        (in_l * (1.0 - mix) + (in_l + wl) * mix * 0.8, in_r * (1.0 - mix) + (in_r + wr) * mix * 0.8)
    }
}

/// A 6-stage all-pass phaser with feedback and stereo phase offset.
#[derive(Clone)]
pub struct Phaser {
    stages_l: [f32; 6],
    stages_r: [f32; 6],
    fb_l: f32,
    fb_r: f32,
    lfo: Lfo,
    pub rate: f64,
    pub depth: f64,
    pub mix: f64,
    pub feedback: f64,
    pub stages: usize,
    sr: f64,
    dc_l: DcBlocker,
    dc_r: DcBlocker,
}

impl Phaser {
    pub fn new(rate: f64, depth: f64, mix: f64) -> Self {
        Self {
            stages_l: [0.0; 6],
            stages_r: [0.0; 6],
            fb_l: 0.0,
            fb_r: 0.0,
            lfo: Lfo::new(LfoShape::Sine, 0x9A5),
            rate,
            depth,
            mix,
            feedback: 0.4,
            stages: 4,
            sr: 44100.0,
            dc_l: DcBlocker::default(),
            dc_r: DcBlocker::default(),
        }
    }

    pub fn set_sample_rate(&mut self, sr: f64) {
        self.sr = sr;
    }

    pub fn reset(&mut self) {
        self.stages_l = [0.0; 6];
        self.stages_r = [0.0; 6];
        self.fb_l = 0.0;
        self.fb_r = 0.0;
        self.dc_l.reset();
        self.dc_r.reset();
    }

    fn allpass_chain(stages: &mut [f32; 6], n: usize, x: f32, coef: f32) -> f32 {
        let mut v = x;
        for s in stages.iter_mut().take(n) {
            let y = -coef * v + *s;
            *s = v + coef * y;
            v = y;
        }
        v
    }

    #[inline]
    pub fn tick(&mut self, in_l: f32, in_r: f32) -> (f32, f32) {
        let l = self.lfo.tick(self.rate.clamp(0.01, 8.0), self.sr);
        // Map the LFO to a coefficient: 200 Hz … 3 kHz sweep.
        let depth = self.depth.clamp(0.0, 1.0) as f32;
        let centre = 0.5 + l * 0.5;
        let f = 200.0 * (3000.0f32 / 200.0).powf((centre * depth + (1.0 - depth) * 0.5).clamp(0.0, 1.0));
        // One-pole allpass coefficient from the target frequency.
        let t = (std::f32::consts::PI * f / self.sr as f32).tan();
        let coef = ((1.0 - t) / (1.0 + t)).clamp(-0.999, 0.999);

        let fb = self.feedback.clamp(0.0, 0.95) as f32;
        let n = self.stages.clamp(1, 6);
        let xl = in_l + self.fb_l * fb;
        let xr = in_r + self.fb_r * fb;
        let yl = Self::allpass_chain(&mut self.stages_l, n, xl, coef);
        let yr = Self::allpass_chain(&mut self.stages_r, n, xr, coef);
        self.fb_l = self.dc_l.process(yl);
        self.fb_r = self.dc_r.process(yr);

        let mix = self.mix.clamp(0.0, 1.0) as f32;
        (in_l * (1.0 - mix) + yl * mix, in_r * (1.0 - mix) + yr * mix)
    }
}
