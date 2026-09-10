//! Insert effects.
//!
//! Every effect is stereo-in / stereo-out so a voice's chain is a straight
//! line, and every parameter is a plain `f64` set by catalog key — which is
//! what lets both the automation lanes and the UI drive them uniformly.

use std::collections::HashMap;

use super::delay::{Chorus, Phaser, StereoDelay};
use super::filter::{Filter, FilterKind};
use super::limiter::Compressor;
use super::reverb::Reverb;
use super::util::{db_to_gain, soft_clip, tube, Biquad, DcBlocker, Oversampler2x};

/// The effect kinds the engine understands (mirrors `crate::fx::EFFECT_KINDS`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectKind {
    Distortion,
    Filter,
    Eq,
    Compressor,
    Delay,
    Reverb,
    Chorus,
    Phaser,
    Bitcrush,
}

impl EffectKind {
    pub fn from_name(kind: &str) -> Option<EffectKind> {
        Some(match kind {
            "distortion" | "drive" | "saturator" => EffectKind::Distortion,
            "filter" => EffectKind::Filter,
            "eq" => EffectKind::Eq,
            "compressor" | "comp" => EffectKind::Compressor,
            "delay" => EffectKind::Delay,
            "reverb" => EffectKind::Reverb,
            "chorus" | "ensemble" => EffectKind::Chorus,
            "phaser" => EffectKind::Phaser,
            "bitcrush" | "crush" => EffectKind::Bitcrush,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            EffectKind::Distortion => "distortion",
            EffectKind::Filter => "filter",
            EffectKind::Eq => "eq",
            EffectKind::Compressor => "compressor",
            EffectKind::Delay => "delay",
            EffectKind::Reverb => "reverb",
            EffectKind::Chorus => "chorus",
            EffectKind::Phaser => "phaser",
            EffectKind::Bitcrush => "bitcrush",
        }
    }
}

fn v(params: &HashMap<String, f64>, keys: &[&str], default: f64) -> f64 {
    for k in keys {
        if let Some(x) = params.get(*k) {
            return *x;
        }
    }
    default
}

// ─────────────────────────── Distortion ───────────────────────────

#[derive(Clone)]
pub struct Distortion {
    pub mode: f64,
    pub drive: f64,
    pub mix: f64,
    pub out: f64,
    pub tone: f64,
    os: Oversampler2x,
    tone_l: Biquad,
    tone_r: Biquad,
    dc_l: DcBlocker,
    dc_r: DcBlocker,
    sr: f32,
}

impl Distortion {
    pub fn new(params: &HashMap<String, f64>) -> Self {
        let mut d = Self {
            mode: v(params, &["mode"], 0.0),
            drive: v(params, &["drive"], 2.0).clamp(0.25, 64.0),
            mix: v(params, &["mix"], 0.5).clamp(0.0, 1.0),
            out: v(params, &["out"], 1.0).clamp(0.05, 4.0),
            tone: v(params, &["tone"], 12000.0).clamp(200.0, 20000.0),
            os: Oversampler2x::new(),
            tone_l: Biquad::lowpass(12000.0, 0.707, super::SR as f32),
            tone_r: Biquad::lowpass(12000.0, 0.707, super::SR as f32),
            dc_l: DcBlocker::default(),
            dc_r: DcBlocker::default(),
            sr: super::SR as f32,
        };
        d.retune();
        d
    }

    pub fn retune(&mut self) {
        self.tone_l = Biquad::lowpass(self.tone as f32, 0.707, self.sr);
        self.tone_r = Biquad::lowpass(self.tone as f32, 0.707, self.sr);
    }

    #[inline]
    fn shape(&self, x: f32) -> f32 {
        let d = self.drive as f32;
        match self.mode.round() as i64 {
            1 => (x * d).clamp(-1.0, 1.0),                       // hard clip
            2 => (x * d * 1.4).sin().clamp(-1.0, 1.0),           // wavefolder
            3 => tube(x * d) * 1.3,                              // asymmetric tube
            4 => {
                // Fuzz: bias then hard clip, which gates on the bias point.
                let b = (x * d + 0.35).clamp(-1.0, 1.0);
                b - 0.35
            }
            _ => soft_clip(x * d) / (1.0 + d * 0.15),            // smooth tanh
        }
    }

    pub fn reset(&mut self) {
        self.os.reset();
        self.tone_l.reset();
        self.tone_r.reset();
        self.dc_l.reset();
        self.dc_r.reset();
    }

    #[inline]
    pub fn tick(&mut self, l: f32, r: f32) -> (f32, f32) {
        let drive_comp = 1.0 / (1.0 + self.drive as f32 * 0.25);
        let (al, a2l) = self.os.up(l * drive_comp);
        let (ar, a2r) = self.os.up(r * drive_comp);
        let wl = self.os.down(self.shape(al), self.shape(a2l));
        let wr = self.os.down(self.shape(ar), self.shape(a2r));
        let wl = self.tone_l.tick(wl);
        let wr = self.tone_r.tick(wr);
        let mix = self.mix as f32;
        let out = self.out as f32;
        (
            self.dc_l.process(l * (1.0 - mix) + wl * mix * 1.6) * out,
            self.dc_r.process(r * (1.0 - mix) + wr * mix * 1.6) * out,
        )
    }
}

// ─────────────────────────── Filter / EQ ───────────────────────────

#[derive(Clone)]
pub struct FilterFx {
    l: Filter,
    r: Filter,
}

impl FilterFx {
    pub fn new(params: &HashMap<String, f64>) -> Self {
        let kind = FilterKind::from_index(v(params, &["type", "ftype"], 0.0));
        let cutoff = v(params, &["cutoff"], 2000.0);
        let q = v(params, &["resonance", "res", "q"], 1.0);
        let mut l = Filter::new(kind, cutoff, q);
        let mut r = Filter::new(kind, cutoff, q);
        l.set_drive(v(params, &["drive"], 1.0));
        r.set_drive(v(params, &["drive"], 1.0));
        l.set_poles(v(params, &["poles"], 1.0).round().clamp(1.0, 2.0) as u8);
        r.set_poles(v(params, &["poles"], 1.0).round().clamp(1.0, 2.0) as u8);
        Self { l, r }
    }

    pub fn set_param(&mut self, key: &str, val: f64) {
        match key {
            "type" | "ftype" => {
                let k = FilterKind::from_index(val);
                self.l.kind = k;
                self.r.kind = k;
            }
            "cutoff" => {
                self.l.set_cutoff(val);
                self.r.set_cutoff(val);
            }
            "resonance" | "res" | "q" => {
                self.l.set_q(val);
                self.r.set_q(val);
            }
            "drive" => {
                self.l.set_drive(val);
                self.r.set_drive(val);
            }
            "poles" => {
                let p = val.round().clamp(1.0, 2.0) as u8;
                self.l.set_poles(p);
                self.r.set_poles(p);
            }
            _ => {}
        }
    }

    pub fn reset(&mut self) {
        self.l.reset();
        self.r.reset();
    }

    #[inline]
    pub fn tick(&mut self, l: f32, r: f32) -> (f32, f32) {
        (self.l.tick(l), self.r.tick(r))
    }
}

#[derive(Clone)]
pub struct Eq3 {
    low_l: Biquad,
    low_r: Biquad,
    mid_l: Biquad,
    mid_r: Biquad,
    hi_l: Biquad,
    hi_r: Biquad,
    sr: f32,
}

impl Eq3 {
    pub fn new(params: &HashMap<String, f64>) -> Self {
        let sr = super::SR as f32;
        let low_f = v(params, &["low_freq"], 220.0) as f32;
        let mid_f = v(params, &["mid_freq"], 1000.0) as f32;
        let hi_f = v(params, &["hi_freq"], 5000.0) as f32;
        let low_g = v(params, &["low_gain"], 0.0) as f32;
        let mid_g = v(params, &["mid_gain"], 0.0) as f32;
        let hi_g = v(params, &["hi_gain"], 0.0) as f32;
        Self {
            low_l: Biquad::low_shelf(low_f, low_g, sr),
            low_r: Biquad::low_shelf(low_f, low_g, sr),
            mid_l: Biquad::peak(mid_f, mid_g, 0.9, sr),
            mid_r: Biquad::peak(mid_f, mid_g, 0.9, sr),
            hi_l: Biquad::high_shelf(hi_f, hi_g, sr),
            hi_r: Biquad::high_shelf(hi_f, hi_g, sr),
            sr,
        }
    }

    pub fn set_param(&mut self, key: &str, val: f64) {
        // Rebuilding coefficient sets is cheap enough at control rate.
        let sr = self.sr;
        match key {
            "low_gain" => {
                let f = 220.0;
                self.low_l = Biquad::low_shelf(f, val as f32, sr);
                self.low_r = Biquad::low_shelf(f, val as f32, sr);
            }
            "hi_gain" => {
                let f = 5000.0;
                self.hi_l = Biquad::high_shelf(f, val as f32, sr);
                self.hi_r = Biquad::high_shelf(f, val as f32, sr);
            }
            "mid_gain" => {
                self.mid_l = Biquad::peak(1000.0, val as f32, 0.9, sr);
                self.mid_r = Biquad::peak(1000.0, val as f32, 0.9, sr);
            }
            _ => {}
        }
    }

    pub fn reset(&mut self) {
        for b in [
            &mut self.low_l,
            &mut self.low_r,
            &mut self.mid_l,
            &mut self.mid_r,
            &mut self.hi_l,
            &mut self.hi_r,
        ] {
            b.reset();
        }
    }

    #[inline]
    pub fn tick(&mut self, l: f32, r: f32) -> (f32, f32) {
        let l = self.hi_l.tick(self.mid_l.tick(self.low_l.tick(l)));
        let r = self.hi_r.tick(self.mid_r.tick(self.low_r.tick(r)));
        (l, r)
    }
}

// ─────────────────────────── Compressor wrapper ───────────────────────────

#[derive(Clone)]
pub struct Comp {
    c: Compressor,
}

impl Comp {
    pub fn new(params: &HashMap<String, f64>) -> Self {
        let mut c = Compressor::new(
            v(params, &["threshold"], -18.0),
            v(params, &["ratio"], 4.0),
            v(params, &["attack"], 10.0),
            v(params, &["release"], 150.0),
        );
        c.knee_db = v(params, &["knee"], 6.0);
        c.makeup_db = v(params, &["makeup"], 0.0);
        c.mix = v(params, &["mix"], 1.0);
        Self { c }
    }

    pub fn set_param(&mut self, key: &str, val: f64) {
        match key {
            "threshold" => self.c.threshold_db = val,
            "ratio" => self.c.ratio = val,
            "attack" => {
                self.c.attack_ms = val;
                self.c.set_sample_rate(super::SR);
            }
            "release" => {
                self.c.release_ms = val;
                self.c.set_sample_rate(super::SR);
            }
            "makeup" => self.c.makeup_db = val,
            "knee" => self.c.knee_db = val,
            "mix" => self.c.mix = val,
            _ => {}
        }
    }

    pub fn reset(&mut self) {
        self.c.reset();
    }

    #[inline]
    pub fn tick(&mut self, l: f32, r: f32) -> (f32, f32) {
        self.c.tick(l, r)
    }
}

// ─────────────────────────── Bitcrush ───────────────────────────

#[derive(Clone)]
pub struct Bitcrush {
    pub bits: f64,
    pub downsample: f64,
    pub mix: f64,
    phase: f64,
    hold_l: f32,
    hold_r: f32,
}

impl Bitcrush {
    pub fn new(params: &HashMap<String, f64>) -> Self {
        Self {
            bits: v(params, &["bits"], 8.0).clamp(1.0, 16.0),
            downsample: v(params, &["downsample", "rate"], 1.0).clamp(1.0, 64.0),
            mix: v(params, &["mix"], 1.0).clamp(0.0, 1.0),
            phase: 0.0,
            hold_l: 0.0,
            hold_r: 0.0,
        }
    }

    pub fn reset(&mut self) {
        self.phase = 0.0;
        self.hold_l = 0.0;
        self.hold_r = 0.0;
    }

    #[inline]
    pub fn tick(&mut self, l: f32, r: f32) -> (f32, f32) {
        self.phase += 1.0;
        if self.phase >= self.downsample {
            self.phase -= self.downsample;
            let levels = (2f64.powf(self.bits) / 2.0) as f32;
            self.hold_l = (l * levels).round() / levels;
            self.hold_r = (r * levels).round() / levels;
        }
        let mix = self.mix as f32;
        (l * (1.0 - mix) + self.hold_l * mix, r * (1.0 - mix) + self.hold_r * mix)
    }
}

// ─────────────────────────── The effect enum ───────────────────────────

#[derive(Clone)]
pub enum Effect {
    Distortion(Distortion),
    Filter(FilterFx),
    Eq(Eq3),
    Compressor(Comp),
    Delay(StereoDelay),
    Reverb(Reverb),
    Chorus(Chorus),
    Phaser(Phaser),
    Bitcrush(Bitcrush),
}

impl Effect {
    pub fn new(kind: EffectKind, params: &HashMap<String, f64>) -> Effect {
        match kind {
            EffectKind::Distortion => Effect::Distortion(Distortion::new(params)),
            EffectKind::Filter => Effect::Filter(FilterFx::new(params)),
            EffectKind::Eq => Effect::Eq(Eq3::new(params)),
            EffectKind::Compressor => Effect::Compressor(Comp::new(params)),
            EffectKind::Delay => {
                let mut d = StereoDelay::new(
                    v(params, &["time"], 250.0),
                    v(params, &["feedback"], 0.4),
                    v(params, &["mix"], 0.3),
                );
                d.set_sample_rate(super::SR);
                d.ping_pong = v(params, &["ping_pong"], 0.5);
                d.damp = v(params, &["damp", "damping"], 0.35);
                d.offset_ms = v(params, &["offset"], 0.0);
                d.mod_depth = v(params, &["mod"], 0.15);
                d.update_damping();
                Effect::Delay(d)
            }
            EffectKind::Reverb => {
                let mut r = Reverb::new(
                    v(params, &["size"], 0.5),
                    v(params, &["damping", "damp"], 0.5),
                    v(params, &["mix"], 0.2),
                );
                r.set_sample_rate(super::SR);
                r.predelay_ms = v(params, &["predelay"], 12.0);
                r.width = v(params, &["width"], 1.0);
                r.mod_amount = v(params, &["mod"], 0.6);
                r.update_damping();
                Effect::Reverb(r)
            }
            EffectKind::Chorus => {
                let mut c = Chorus::new(
                    v(params, &["rate"], 0.6),
                    v(params, &["depth"], 0.5),
                    v(params, &["mix"], 0.4),
                );
                c.set_sample_rate(super::SR);
                c.spread = v(params, &["spread"], 0.6);
                Effect::Chorus(c)
            }
            EffectKind::Phaser => {
                let mut p = Phaser::new(
                    v(params, &["rate"], 0.3),
                    v(params, &["depth"], 0.7),
                    v(params, &["mix"], 0.5),
                );
                p.set_sample_rate(super::SR);
                p.feedback = v(params, &["feedback"], 0.4);
                p.stages = v(params, &["stages"], 4.0).round().clamp(1.0, 6.0) as usize;
                Effect::Phaser(p)
            }
            EffectKind::Bitcrush => Effect::Bitcrush(Bitcrush::new(params)),
        }
    }

    pub fn kind_name(&self) -> &'static str {
        match self {
            Effect::Distortion(_) => "distortion",
            Effect::Filter(_) => "filter",
            Effect::Eq(_) => "eq",
            Effect::Compressor(_) => "compressor",
            Effect::Delay(_) => "delay",
            Effect::Reverb(_) => "reverb",
            Effect::Chorus(_) => "chorus",
            Effect::Phaser(_) => "phaser",
            Effect::Bitcrush(_) => "bitcrush",
        }
    }

    pub fn set_param(&mut self, key: &str, val: f64) {
        match self {
            Effect::Distortion(d) => match key {
                "mode" => d.mode = val,
                "drive" => {
                    d.drive = val.clamp(0.25, 64.0);
                }
                "mix" => d.mix = val.clamp(0.0, 1.0),
                "out" => d.out = val.clamp(0.05, 4.0),
                "tone" => {
                    d.tone = val.clamp(200.0, 20000.0);
                    d.retune();
                }
                _ => {}
            },
            Effect::Filter(f) => f.set_param(key, val),
            Effect::Eq(e) => e.set_param(key, val),
            Effect::Compressor(c) => c.set_param(key, val),
            Effect::Delay(d) => match key {
                "time" | "delay_time" => d.time_ms = val,
                "feedback" => d.feedback = val.clamp(0.0, 0.98),
                "mix" => d.mix = val.clamp(0.0, 1.0),
                "ping_pong" => d.ping_pong = val.clamp(0.0, 1.0),
                "damp" | "damping" => {
                    d.damp = val;
                    d.update_damping();
                }
                "offset" => d.offset_ms = val,
                "mod" => d.mod_depth = val,
                _ => {}
            },
            Effect::Reverb(r) => match key {
                "size" => r.size = val,
                "damping" | "damp" => {
                    r.damping = val;
                    r.update_damping();
                }
                "mix" => r.mix = val.clamp(0.0, 1.0),
                "predelay" => r.predelay_ms = val,
                "width" => r.width = val,
                "mod" => r.mod_amount = val,
                _ => {}
            },
            Effect::Chorus(c) => match key {
                "rate" => c.rate = val,
                "depth" => c.depth = val,
                "mix" => c.mix = val,
                "spread" => c.spread = val,
                _ => {}
            },
            Effect::Phaser(p) => match key {
                "rate" => p.rate = val,
                "depth" => p.depth = val,
                "mix" => p.mix = val,
                "feedback" => p.feedback = val,
                "stages" => p.stages = val.round().clamp(1.0, 6.0) as usize,
                _ => {}
            },
            Effect::Bitcrush(b) => match key {
                "bits" => b.bits = val.clamp(1.0, 16.0),
                "downsample" | "rate" => b.downsample = val.clamp(1.0, 64.0),
                "mix" => b.mix = val.clamp(0.0, 1.0),
                _ => {}
            },
        }
    }

    pub fn reset(&mut self) {
        match self {
            Effect::Distortion(d) => d.reset(),
            Effect::Filter(f) => f.reset(),
            Effect::Eq(e) => e.reset(),
            Effect::Compressor(c) => c.reset(),
            Effect::Delay(d) => d.reset(),
            Effect::Reverb(r) => r.reset(),
            Effect::Chorus(c) => c.reset(),
            Effect::Phaser(p) => p.reset(),
            Effect::Bitcrush(b) => b.reset(),
        }
    }

    #[inline]
    pub fn tick(&mut self, l: f32, r: f32) -> (f32, f32) {
        match self {
            Effect::Distortion(d) => d.tick(l, r),
            Effect::Filter(f) => f.tick(l, r),
            Effect::Eq(e) => e.tick(l, r),
            Effect::Compressor(c) => c.tick(l, r),
            Effect::Delay(d) => d.tick(l, r),
            Effect::Reverb(rv) => rv.tick(l, r),
            Effect::Chorus(c) => c.tick(l, r),
            Effect::Phaser(p) => p.tick(l, r),
            Effect::Bitcrush(b) => b.tick(l, r),
        }
    }
}

/// Small helper: linear gain from a possibly-missing key.
pub fn gain_of(params: &HashMap<String, f64>, key: &str, default_db: f64) -> f32 {
    db_to_gain(v(params, &[key], default_db))
}
