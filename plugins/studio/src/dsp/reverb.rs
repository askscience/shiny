//! A feedback-delay-network (FDN) reverb.
//!
//! Eight prime-length delay lines in a lossless Householder feedback matrix,
//! input diffusion through four all-passes, per-line damping and slow delay
//! modulation. Compared with a Schroeder/Freeverb comb bank this has a much
//! denser, smoother tail and no metallic ringing, and the modulated taps mean
//! it never sounds static on sustained pads.

use super::delay::{Allpass, DelayLine};
use super::filter::OnePole;
use super::lfo::{Lfo, LfoShape};
use super::util::DcBlocker;

const N: usize = 8;
/// Prime-ish line lengths in samples at 44.1 kHz (Freeverb comb tunings,
/// extended to eight lines) — mutually prime so the echoes don't line up.
const BASE: [f64; N] = [1116.0, 1188.0, 1277.0, 1356.0, 1422.0, 1491.0, 1557.0, 1617.0];

#[derive(Clone)]
pub struct Reverb {
    lines: [DelayLine; N],
    base: [f64; N],
    damp: [OnePole; N],
    ap: [Allpass; 4],
    pre_l: DelayLine,
    pre_r: DelayLine,
    mods: [Lfo; N],
    pub size: f64,
    pub damping: f64,
    pub mix: f64,
    pub predelay_ms: f64,
    pub width: f64,
    pub mod_amount: f64,
    sr: f64,
    dc_l: DcBlocker,
    dc_r: DcBlocker,
}

impl Reverb {
    pub fn new(size: f64, damping: f64, mix: f64) -> Self {
        let sr = 44100.0;
        Self {
            lines: std::array::from_fn(|_| DelayLine::new(4 * 44100)),
            base: BASE,
            damp: std::array::from_fn(|_| OnePole::lowpass(6000.0, sr)),
            ap: [Allpass::new(225, 0.5), Allpass::new(341, 0.5), Allpass::new(441, 0.5), Allpass::new(556, 0.5)],
            pre_l: DelayLine::new(44100),
            pre_r: DelayLine::new(44100),
            mods: std::array::from_fn(|i| {
                // Slightly different rates so the lines decorrelate over time.
                Lfo::new(if i % 3 == 0 { LfoShape::Sine } else { LfoShape::Triangle }, 0x4E57u64.wrapping_add(i as u64))
            }),
            size,
            damping,
            mix,
            predelay_ms: 12.0,
            width: 1.0,
            mod_amount: 0.6,
            sr,
            dc_l: DcBlocker::default(),
            dc_r: DcBlocker::default(),
        }
    }

    pub fn set_sample_rate(&mut self, sr: f64) {
        // Line lengths are quoted at 44.1 kHz; rescale for any other rate.
        let k = sr / 44100.0;
        for i in 0..N {
            self.base[i] = BASE[i] * k;
        }
        self.sr = sr;
        self.update_damping();
    }

    pub fn update_damping(&mut self) {
        // 0 → bright (14 kHz), 1 → dark (700 Hz).
        let fc = 14000.0 * (0.05f64).powf(self.damping.clamp(0.0, 1.0));
        for d in self.damp.iter_mut() {
            *d = OnePole::lowpass(fc, self.sr);
        }
    }

    pub fn reset(&mut self) {
        for l in self.lines.iter_mut() {
            l.reset();
        }
        for a in self.ap.iter_mut() {
            a.reset();
        }
        self.pre_l.reset();
        self.pre_r.reset();
        for d in self.damp.iter_mut() {
            d.reset();
        }
        self.dc_l.reset();
        self.dc_r.reset();
    }

    #[inline]
    pub fn tick(&mut self, in_l: f32, in_r: f32) -> (f32, f32) {
        let pre = (self.predelay_ms.clamp(0.0, 500.0) * 0.001 * self.sr) as f32;
        let a = if pre >= 2.0 { self.pre_l.read(pre) } else { in_l };
        let b = if pre >= 2.0 { self.pre_r.read(pre) } else { in_r };
        self.pre_l.write(in_l);
        self.pre_r.write(in_r);

        // Diffuse the input before it hits the network.
        let mono_in = (a + b) * 0.5;
        let mut d = mono_in;
        for ap in self.ap.iter_mut() {
            d = ap.tick(d);
        }
        // Feed alternate lines with alternating polarity for a wide image.
        let size = self.size.clamp(0.05, 1.6);
        let mut taps = [0.0f32; N];
        for i in 0..N {
            let base = self.base[i] * size;
            let m = self.mods[i].tick(0.08 + i as f64 * 0.017, self.sr) * (self.mod_amount as f32 * 6.0);
            taps[i] = self.lines[i].read((base + m as f64).max(2.0) as f32);
        }

        // Householder feedback: y = x - (2/N)·Σx (lossless, well-conditioned).
        let sum: f32 = taps.iter().sum();
        let norm = 2.0 / N as f32;
        let mut wet_l = 0.0f32;
        let mut wet_r = 0.0f32;
        for i in 0..N {
            let mut v = taps[i] - sum * norm;
            v = self.damp[i].tick(v);
            let out = v * 0.82 + d * (if i % 2 == 0 { 0.5 } else { -0.5 });
            self.lines[i].write(out);
            if i % 2 == 0 {
                wet_l += v;
                wet_r += v * 0.7;
            } else {
                wet_r += v;
                wet_l += v * 0.7;
            }
        }
        wet_l *= 1.0 / (N as f32 * 0.5);
        wet_r *= 1.0 / (N as f32 * 0.5);
        // Stereo width (mid/side scaling).
        let w = self.width.clamp(0.0, 2.0) as f32;
        let mid = (wet_l + wet_r) * 0.5;
        let side = (wet_l - wet_r) * 0.5 * w;
        wet_l = self.dc_l.process(mid + side);
        wet_r = self.dc_r.process(mid - side);

        let mix = self.mix.clamp(0.0, 1.0) as f32;
        (in_l * (1.0 - mix) + wet_l * mix, in_r * (1.0 - mix) + wet_r * mix)
    }
}
