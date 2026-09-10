//! Filters — a topological-preserving-transform (TPT) state-variable filter
//! with a switchable second pole, plus input saturation.
//!
//! Two cascaded SVF sections give the classic 24 dB/oct "ladder-ish" slope
//! without the numerical fragility of a nonlinear feedback ladder: each
//! section is unconditionally stable at any cutoff/resonance, and the input
//! `tanh` supplies the drive character.

use super::util::soft_clip;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterKind {
    LowPass,
    HighPass,
    BandPass,
    Notch,
    Peak,
}

impl FilterKind {
    pub fn from_index(i: f64) -> FilterKind {
        match i.round() as i64 {
            1 => FilterKind::HighPass,
            2 => FilterKind::BandPass,
            3 => FilterKind::Notch,
            4 => FilterKind::Peak,
            _ => FilterKind::LowPass,
        }
    }

    pub fn index(self) -> f64 {
        match self {
            FilterKind::LowPass => 0.0,
            FilterKind::HighPass => 1.0,
            FilterKind::BandPass => 2.0,
            FilterKind::Notch => 3.0,
            FilterKind::Peak => 4.0,
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Svf {
    ic1: f64,
    ic2: f64,
}

impl Svf {
    #[inline]
    fn tick(&mut self, x: f64, g: f64, k: f64, kind: FilterKind) -> f64 {
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;
        let v3 = x - self.ic2;
        let v1 = a1 * self.ic1 + a2 * v3;
        let v2 = self.ic2 + a2 * self.ic1 + a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        match kind {
            FilterKind::LowPass => v2,
            FilterKind::HighPass => x - k * v1 - v2,
            FilterKind::BandPass => v1,
            FilterKind::Notch => x - k * v1,
            FilterKind::Peak => 2.0 * v2 - x + k * v1,
        }
    }

    fn reset(&mut self) {
        self.ic1 = 0.0;
        self.ic2 = 0.0;
    }
}

#[derive(Clone)]
pub struct Filter {
    pub kind: FilterKind,
    cutoff: f64,
    q: f64,
    /// Input drive as a linear pre-gain (1.0 = clean).
    drive: f64,
    /// 1 = 12 dB/oct, 2 = 24 dB/oct.
    pub poles: u8,
    g: f64,
    k: f64,
    a: Svf,
    b: Svf,
    /// Post-drive make-up so `drive` changes character, not loudness.
    makeup: f64,
    sample_rate: f64,
    dirty: bool,
    dc: super::util::DcBlocker,
    /// Optional output trim used by the voice to keep resonance peaks sane.
    pub out_trim: f64,
}

impl Filter {
    pub fn new(kind: FilterKind, cutoff: f64, q: f64) -> Self {
        let mut f = Self {
            kind,
            cutoff,
            q,
            drive: 1.0,
            poles: 1,
            g: 0.0,
            k: 0.0,
            a: Svf::default(),
            b: Svf::default(),
            makeup: 1.0,
            sample_rate: super::SR,
            dirty: true,
            dc: super::util::DcBlocker::default(),
            out_trim: 1.0,
        };
        f.recompute();
        f
    }

    pub fn set_sample_rate(&mut self, sr: f64) {
        self.sample_rate = sr;
        self.dirty = true;
    }

    pub fn set_cutoff(&mut self, hz: f64) {
        let hz = hz.clamp(10.0, 22000.0);
        if (hz - self.cutoff).abs() > 1e-6 {
            self.cutoff = hz;
            self.dirty = true;
        }
    }

    pub fn set_q(&mut self, q: f64) {
        if (q - self.q).abs() > 1e-9 {
            self.q = q;
            self.dirty = true;
        }
    }

    pub fn set_drive(&mut self, drive: f64) {
        let d = drive.clamp(0.05, 64.0);
        if (d - self.drive).abs() > 1e-9 {
            self.drive = d;
            // tanh-ish make-up: keep perceived level roughly constant.
            self.makeup = 1.0 / (1.0 + (d - 1.0) * 0.55).max(0.15);
        }
    }

    pub fn set_poles(&mut self, poles: u8) {
        self.poles = poles.clamp(1, 2);
    }

    fn recompute(&mut self) {
        let nyq = self.sample_rate * 0.5;
        let fc = self.cutoff.clamp(10.0, nyq * 0.94);
        let g = (std::f64::consts::PI * fc / self.sample_rate).tan();
        self.g = g.clamp(1e-6, 20.0);
        // Q in [0.5, 24] → damping k in [2, 0.0417]. A small floor stops the
        // self-oscillation blow-up that a true k=0 would allow.
        let q = self.q.clamp(0.4, 24.0);
        self.k = (1.0 / q).max(0.035);
        self.dirty = false;
    }

    pub fn reset(&mut self) {
        self.a.reset();
        self.b.reset();
        self.dc.reset();
    }

    #[inline]
    pub fn tick(&mut self, x: f32) -> f32 {
        if self.dirty {
            self.recompute();
        }
        let mut s = x as f64;
        if self.drive > 1.0001 {
            s = (s * self.drive).tanh() * self.makeup;
        }
        let g = self.g;
        let k = self.k;
        let mut y = self.a.tick(s, g, k, self.kind);
        if self.poles >= 2 {
            y = self.b.tick(y, g, k, self.kind);
        }
        // A touch of DC removal keeps asymmetric drive from eating headroom.
        (self.dc.process((y * self.out_trim) as f32) as f64) as f32
    }
}

/// A gentle fixed tone shaper used by drums (band emphasis without a full
/// filter instance in the hot loop).
#[derive(Clone)]
pub struct OnePole {
    z: f32,
    coef: f32,
    hp: bool,
}

impl OnePole {
    pub fn lowpass(cutoff: f64, sr: f64) -> Self {
        let coef = 1.0 - (-2.0 * std::f64::consts::PI * cutoff / sr).exp();
        Self { z: 0.0, coef: coef as f32, hp: false }
    }

    pub fn highpass(cutoff: f64, sr: f64) -> Self {
        let coef = 1.0 - (-2.0 * std::f64::consts::PI * cutoff / sr).exp();
        Self { z: 0.0, coef: coef as f32, hp: true }
    }

    pub fn reset(&mut self) {
        self.z = 0.0;
    }

    #[inline]
    pub fn tick(&mut self, x: f32) -> f32 {
        self.z += (x - self.z) * self.coef;
        if self.hp {
            x - self.z
        } else {
            self.z
        }
    }
}

/// A resonant 2-pole band-pass built from the TPT SVF, used for noise shaping
/// in snares/claps.
#[derive(Clone)]
pub struct BandPass {
    svf: Svf,
    g: f64,
    k: f64,
    cutoff: f64,
    q: f64,
    sr: f64,
    dirty: bool,
}

impl BandPass {
    pub fn new(cutoff: f64, q: f64) -> Self {
        let mut b = Self { svf: Svf::default(), g: 0.0, k: 0.0, cutoff, q, sr: super::SR, dirty: true };
        b.recompute();
        b
    }

    pub fn set_cutoff(&mut self, hz: f64) {
        self.cutoff = hz;
        self.dirty = true;
    }

    pub fn set_q(&mut self, q: f64) {
        self.q = q;
        self.dirty = true;
    }

    fn recompute(&mut self) {
        let fc = self.cutoff.clamp(20.0, self.sr * 0.45);
        self.g = (std::f64::consts::PI * fc / self.sr).tan().clamp(1e-6, 20.0);
        self.k = (1.0 / self.q.clamp(0.3, 24.0)).max(0.02);
        self.dirty = false;
    }

    pub fn reset(&mut self) {
        self.svf.reset();
    }

    #[inline]
    pub fn tick(&mut self, x: f32) -> f32 {
        if self.dirty {
            self.recompute();
        }
        self.svf.tick(x as f64, self.g, self.k, FilterKind::BandPass) as f32
    }
}

/// Convenience: a soft saturation used across drums.
#[inline]
pub fn saturate(x: f32, amount: f32) -> f32 {
    if amount <= 0.0 {
        x
    } else {
        let d = 1.0 + amount * 8.0;
        soft_clip(x * d) / (1.0 + amount * 0.6)
    }
}

#[cfg(test)]
mod probe_tests {
#[test]
fn probe_filter_cutoff_changes_output() {
    use crate::dsp::filter::{Filter, FilterKind};
    use crate::dsp::osc::{Osc, Wave};
    let render = |cut: f64| {
        let mut o = Osc::new(Wave::Saw);
        let mut f = Filter::new(FilterKind::LowPass, cut, 1.6);
        let mut sum = 0.0f64;
        for i in 0..4410 {
            let x = o.tick(220.0, 44100.0);
            let y = f.tick(x);
            if i > 1000 { sum += (y as f64) * (y as f64); }
        }
        sum
    };
    let a = render(500.0);
    let b = render(8000.0);
    eprintln!("[filterprobe] low={a} high={b}");
    assert!((a - b).abs() > 1e-6, "cutoff must change energy");
}

}
