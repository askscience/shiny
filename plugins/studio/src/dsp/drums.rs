//! Drum synthesis.
//!
//! Every drum is a small self-contained model rather than a one-shot sample:
//! layered transients, pitch envelopes, band-passed noise and analog-style
//! saturation. Each one exposes the legacy trem parameter names (`pitch`,
//! `decay`, `sweep`, `tone`, `body`, `noise`) so old configs keep working, plus
//! richer controls (`click`, `drive`, `snap`, `metal`, …) for the new UI.

use super::env::PercEnv;
use super::filter::{saturate, BandPass, OnePole};
use super::noise::{MetalBank, Noise};
use super::osc::{Osc, Wave};
use super::util::DcBlocker;
use super::SR;

/// A drum voice.
#[derive(Clone)]
pub enum Drum {
    Kick(Kick),
    Snare(Snare),
    Hat(Hat),
    Clap(Clap),
    Tom(Tom),
    Perc(Perc),
    Rim(Rim),
    Cowbell(Cowbell),
    Shaker(Shaker),
    Crash(Crash),
    Ride(Ride),
}

impl Drum {
    pub fn from_kind(kind: &str, seed: u64) -> Drum {
        match kind {
            "snare" => Drum::Snare(Snare::new(seed)),
            "hat" => Drum::Hat(Hat::new(seed)),
            "clap" => Drum::Clap(Clap::new(seed)),
            "tom" => Drum::Tom(Tom::new(seed)),
            "perc" => Drum::Perc(Perc::new(seed)),
            "rim" => Drum::Rim(Rim::new(seed)),
            "cowbell" => Drum::Cowbell(Cowbell::new(seed)),
            "shaker" => Drum::Shaker(Shaker::new(seed)),
            "crash" => Drum::Crash(Crash::new(seed)),
            "ride" => Drum::Ride(Ride::new(seed)),
            _ => Drum::Kick(Kick::new(seed)),
        }
    }

    pub fn trigger(&mut self, velocity: f64) {
        match self {
            Drum::Kick(d) => d.trigger(velocity),
            Drum::Snare(d) => d.trigger(velocity),
            Drum::Hat(d) => d.trigger(velocity),
            Drum::Clap(d) => d.trigger(velocity),
            Drum::Tom(d) => d.trigger(velocity),
            Drum::Perc(d) => d.trigger(velocity),
            Drum::Rim(d) => d.trigger(velocity),
            Drum::Cowbell(d) => d.trigger(velocity),
            Drum::Shaker(d) => d.trigger(velocity),
            Drum::Crash(d) => d.trigger(velocity),
            Drum::Ride(d) => d.trigger(velocity),
        }
    }

    pub fn reset(&mut self) {
        match self {
            Drum::Kick(d) => d.reset(),
            Drum::Snare(d) => d.reset(),
            Drum::Hat(d) => d.reset(),
            Drum::Clap(d) => d.reset(),
            Drum::Tom(d) => d.reset(),
            Drum::Perc(d) => d.reset(),
            Drum::Rim(d) => d.reset(),
            Drum::Cowbell(d) => d.reset(),
            Drum::Shaker(d) => d.reset(),
            Drum::Crash(d) => d.reset(),
            Drum::Ride(d) => d.reset(),
        }
    }

    /// True while the drum is still ringing (used to skip silent voices).
    pub fn is_active(&self) -> bool {
        match self {
            Drum::Kick(d) => d.active(),
            Drum::Snare(d) => d.active(),
            Drum::Hat(d) => d.active(),
            Drum::Clap(d) => d.active(),
            Drum::Tom(d) => d.active(),
            Drum::Perc(d) => d.active(),
            Drum::Rim(d) => d.active(),
            Drum::Cowbell(d) => d.active(),
            Drum::Shaker(d) => d.active(),
            Drum::Crash(d) => d.active(),
            Drum::Ride(d) => d.active(),
        }
    }

    #[inline]
    pub fn tick(&mut self, sr: f64) -> f32 {
        match self {
            Drum::Kick(d) => d.tick(sr),
            Drum::Snare(d) => d.tick(sr),
            Drum::Hat(d) => d.tick(sr),
            Drum::Clap(d) => d.tick(sr),
            Drum::Tom(d) => d.tick(sr),
            Drum::Perc(d) => d.tick(sr),
            Drum::Rim(d) => d.tick(sr),
            Drum::Cowbell(d) => d.tick(sr),
            Drum::Shaker(d) => d.tick(sr),
            Drum::Crash(d) => d.tick(sr),
            Drum::Ride(d) => d.tick(sr),
        }
    }

    /// Apply one parameter by its catalog key. Unknown keys are ignored.
    pub fn set_param(&mut self, key: &str, value: f64) {
        let v = value;
        match self {
            Drum::Kick(d) => match key {
                "pitch" => d.pitch = v.clamp(20.0, 200.0),
                "decay" => d.decay = v.clamp(0.5, 40.0),
                "sweep" => {
                    d.sweep_rate = v.clamp(0.5, 120.0);
                    d.start_ratio = 2f64.powf(d.sweep_rate / 120.0);
                }
                "click" => d.click = v.clamp(0.0, 1.0),
                "drive" => d.drive = v.clamp(0.0, 1.0),
                "hold" => d.hold = v.clamp(0.0, 0.2),
                _ => {}
            },
            Drum::Snare(d) => match key {
                "tone" => d.tone = v.clamp(80.0, 500.0),
                "body" => d.body = v.clamp(1.0, 80.0),
                "noise" => d.noise_rate = v.clamp(1.0, 80.0),
                "snap" => d.snap = v.clamp(0.0, 1.0),
                "buzz" => d.buzz = v.clamp(0.0, 1.0),
                "drive" => d.drive = v.clamp(0.0, 1.0),
                _ => {}
            },
            Drum::Hat(d) => match key {
                "decay" => d.decay = v.clamp(2.0, 200.0),
                "tone" => d.tone = v.clamp(2000.0, 14000.0),
                "metal" => d.metal_mix = v.clamp(0.0, 1.0),
                "drive" => d.drive = v.clamp(0.0, 1.0),
                _ => {}
            },
            Drum::Clap(d) => match key {
                "tone" => d.tone = v.clamp(400.0, 4000.0),
                "body" => d.body = v.clamp(1.0, 60.0),
                "noise" => d.noise_rate = v.clamp(1.0, 60.0),
                "spread" => d.spread = v.clamp(0.0, 1.0),
                _ => {}
            },
            Drum::Tom(d) => match key {
                "pitch" => d.pitch = v.clamp(40.0, 500.0),
                "decay" => d.decay = v.clamp(0.5, 60.0),
                "sweep" => d.sweep_rate = v.clamp(0.5, 150.0),
                "noise" => d.noise_mix = v.clamp(0.0, 1.0),
                _ => {}
            },
            Drum::Perc(d) => match key {
                "decay" => d.decay = v.clamp(4.0, 300.0),
                "tone" => d.tone = v.clamp(200.0, 8000.0),
                "metal" => d.metal_mix = v.clamp(0.0, 1.0),
                _ => {}
            },
            Drum::Rim(d) => match key {
                "decay" => d.decay = v.clamp(4.0, 200.0),
                "tone" => d.tone = v.clamp(200.0, 6000.0),
                _ => {}
            },
            Drum::Cowbell(d) => match key {
                "decay" => d.decay = v.clamp(4.0, 200.0),
                "tone" => d.tone = v.clamp(200.0, 3000.0),
                _ => {}
            },
            Drum::Shaker(d) => match key {
                "decay" => d.decay = v.clamp(4.0, 300.0),
                "tone" => d.tone = v.clamp(2000.0, 14000.0),
                _ => {}
            },
            Drum::Crash(d) => match key {
                "decay" => d.decay = v.clamp(0.5, 30.0),
                "tone" => d.tone = v.clamp(2000.0, 14000.0),
                _ => {}
            },
            Drum::Ride(d) => match key {
                "decay" => d.decay = v.clamp(0.5, 30.0),
                "tone" => d.tone = v.clamp(2000.0, 14000.0),
                "bell" => d.bell = v.clamp(0.0, 1.0),
                _ => {}
            },
        }
    }
}

/// `decay`-style control (trem's per-sample rate) → envelope time in seconds.
#[inline]
fn rate_to_seconds(rate: f64) -> f64 {
    4.6 / rate.max(0.1)
}

// ─────────────────────────── Kick ───────────────────────────

#[derive(Clone)]
pub struct Kick {
    osc: Osc,
    env: PercEnv,
    click_env: PercEnv,
    noise: Noise,
    freq: f64,
    pitch: f64,
    start_ratio: f64,
    sweep_rate: f64,
    decay: f64,
    pub click: f64,
    pub drive: f64,
    hold: f64,
    click_hp: OnePole,
    dc: DcBlocker,
    vel: f64,
    last_decay: f64,
    last_hold: f64,
}

impl Kick {
    pub fn new(seed: u64) -> Self {
        let mut osc = Osc::new(Wave::Sine);
        osc.set_phase(0.25);
        let mut k = Self {
            osc,
            env: PercEnv::new(0.0004, rate_to_seconds(8.0), 0.004),
            click_env: PercEnv::new(0.0001, 0.004, 0.0),
            noise: Noise::new(seed ^ 0xA11),
            freq: 50.0,
            pitch: 50.0,
            start_ratio: 3.0,
            sweep_rate: 30.0,
            decay: 8.0,
            click: 0.22,
            drive: 0.3,
            hold: 0.004,
            click_hp: OnePole::highpass(2500.0, SR),
            dc: DcBlocker::default(),
            vel: 1.0,
            last_decay: 8.0,
            last_hold: 0.004,
        };
        k.sync_env();
        k
    }

    fn sync_env(&mut self) {
        // `sweep` is the glide depth: 0.5 → barely there, 120 → one octave.
        self.start_ratio = 2f64.powf(self.sweep_rate / 120.0);
        self.env.set(0.0004, rate_to_seconds(self.decay), self.hold);
        self.last_decay = self.decay;
        self.last_hold = self.hold;
    }

    pub fn trigger(&mut self, velocity: f64) {
        self.vel = velocity.clamp(0.05, 1.6);
        // Louder hits start slightly higher and sweep further — the classic
        // "harder you hit, the more punch" behaviour. The glide is musical:
        // `sweep` 30 (the default) is ≈2.5 semitones, 120 is one octave,
        // rather than the old arbitrary 3× frequency jump.
        self.freq = self.pitch * self.start_ratio * (0.85 + 0.2 * velocity as f64).min(1.25);
        self.osc.reset();
        self.env.trigger(self.vel);
        self.click_env.trigger(self.vel * self.click);
        self.dc.reset();
    }

    pub fn reset(&mut self) {
        self.env.reset();
        self.click_env.reset();
        self.osc.reset();
        self.freq = self.pitch * self.start_ratio;
        self.dc.reset();
    }

    pub fn active(&self) -> bool {
        self.env.is_active() || self.click_env.is_active()
    }

    #[inline]
    pub fn tick(&mut self, sr: f64) -> f32 {
        if self.decay != self.last_decay || self.hold != self.last_hold {
            self.sync_env();
        }
        let a = self.env.tick();
        if a <= 0.0 && !self.click_env.is_active() {
            return 0.0;
        }
        // Pitch envelope: exponential glide toward the fundamental.
        let coef = 1.0 - (-(self.sweep_rate.max(1.0)) / sr * 4.6).exp();
        self.freq += (self.pitch - self.freq) * coef;
        let body = self.osc.tick(self.freq, sr) as f64 * a;
        let c = self.click_env.tick() * 0.9;
        let transient = self.click_hp.tick(self.noise.tick()) * c as f32;
        let mixed = (body + transient as f64) * 1.05;
        let out = if self.drive > 0.0 {
            saturate(mixed as f32, self.drive as f32) as f64
        } else {
            mixed
        };
        self.dc.process(out as f32) * 0.98
    }
}

// ─────────────────────────── Snare ───────────────────────────

#[derive(Clone)]
pub struct Snare {
    tone_osc: Osc,
    tone_osc2: Osc,
    tone_env: PercEnv,
    noise_env: PercEnv,
    noise: Noise,
    // 1.6 kHz-ish noise body, plus a HPF for the "crack".
    bp: BandPass,
    hp: OnePole,
    dc: DcBlocker,
    tone: f64,
    body: f64,
    noise_rate: f64,
    pub snap: f64,
    pub buzz: f64,
    pub drive: f64,
    vel: f64,
    last_body: f64,
    last_noise: f64,
}

impl Snare {
    pub fn new(seed: u64) -> Self {
        let mut s = Self {
            tone_osc: Osc::new(Wave::Sine),
            tone_osc2: Osc::new(Wave::Triangle),
            tone_env: PercEnv::new(0.0003, rate_to_seconds(25.0), 0.001),
            noise_env: PercEnv::new(0.0003, rate_to_seconds(15.0), 0.002),
            noise: Noise::new(seed ^ 0x5E),
            bp: BandPass::new(1800.0, 0.9),
            hp: OnePole::highpass(400.0, SR),
            dc: DcBlocker::default(),
            tone: 200.0,
            body: 25.0,
            noise_rate: 15.0,
            snap: 0.5,
            buzz: 0.15,
            drive: 0.2,
            vel: 1.0,
            last_body: 25.0,
            last_noise: 15.0,
        };
        s.sync_env();
        s
    }

    fn sync_env(&mut self) {
        self.tone_env.set(0.0003, rate_to_seconds(self.body), 0.001);
        self.noise_env.set(0.0003, rate_to_seconds(self.noise_rate), 0.002);
        self.last_body = self.body;
        self.last_noise = self.noise_rate;
    }

    pub fn trigger(&mut self, velocity: f64) {
        self.vel = velocity.clamp(0.05, 1.6);
        self.tone_osc.reset();
        self.tone_osc2.reset();
        self.tone_env.trigger(self.vel);
        self.noise_env.trigger(self.vel);
        self.dc.reset();
    }

    pub fn reset(&mut self) {
        self.tone_env.reset();
        self.noise_env.reset();
        self.tone_osc.reset();
        self.tone_osc2.reset();
        self.bp.reset();
        self.dc.reset();
    }

    pub fn active(&self) -> bool {
        self.tone_env.is_active() || self.noise_env.is_active()
    }

    #[inline]
    pub fn tick(&mut self, sr: f64) -> f32 {
        if self.body != self.last_body || self.noise_rate != self.last_noise {
            self.sync_env();
        }
        let ta = self.tone_env.tick();
        let na = self.noise_env.tick();
        if ta <= 0.0 && na <= 0.0 {
            return 0.0;
        }
        // Two detuned bodies: fundamental + a fifth-ish overtone, with a small
        // amount of triangle for "buzz".
        let f1 = self.tone;
        let f2 = self.tone * 1.58;
        let tone = (self.tone_osc.tick(f1, sr) as f64 + self.tone_osc2.tick(f2, sr) as f64 * (0.35 + self.buzz)) * ta;
        // Noise through a resonant band-pass; `snap` adds the high crack.
        self.bp.set_cutoff(1200.0 + self.tone * 6.0);
        let n = self.bp.tick(self.noise.tick()) as f64;
        let crack = self.hp.tick(self.noise.tick()) as f64 * self.snap;
        let noise = (n * 1.4 + crack * 1.1) * na;
        let mixed = (tone * 0.9 + noise * 0.85) as f32;
        let out = if self.drive > 0.0 { saturate(mixed, self.drive as f32) } else { mixed };
        self.dc.process(out) * 0.92
    }
}

// ─────────────────────────── Hat ───────────────────────────

#[derive(Clone)]
pub struct Hat {
    metal: MetalBank,
    noise: Noise,
    env: PercEnv,
    hp: OnePole,
    bp: BandPass,
    pub decay: f64,
    tone: f64,
    /// Blend between the metallic bank (1.0) and noise (0.0).
    metal_mix: f64,
    pub drive: f64,
    vel: f64,
}

impl Hat {
    pub fn new(seed: u64) -> Self {
        let mut h = Self {
            metal: MetalBank::new(),
            noise: Noise::new(seed ^ 0x4841_5400),
            env: PercEnv::new(0.0001, rate_to_seconds(40.0), 0.0),
            hp: OnePole::highpass(7000.0, SR),
            bp: BandPass::new(9000.0, 0.7),
            decay: 40.0,
            tone: 9000.0,
            metal_mix: 0.8,
            drive: 0.1,
            vel: 1.0,
        };
        h.sync();
        h
    }

    fn sync(&mut self) {
        self.env.set(0.0001, rate_to_seconds(self.decay), 0.0);
        self.hp = OnePole::highpass(self.tone * 0.7, SR);
        self.bp.set_cutoff(self.tone);
    }

    pub fn trigger(&mut self, velocity: f64) {
        self.vel = velocity.clamp(0.05, 1.6);
        self.env.trigger(self.vel);
        self.metal.reset();
    }

    pub fn reset(&mut self) {
        self.env.reset();
        self.metal.reset();
        self.hp.reset();
        self.bp.reset();
    }

    pub fn active(&self) -> bool {
        self.env.is_active()
    }

    #[inline]
    pub fn tick(&mut self, sr: f64) -> f32 {
        let a = self.env.tick();
        if a <= 0.0 {
            return 0.0;
        }
        // The 808 hat recipe: a six-square metallic bank, high-passed, with a
        // little noise so it doesn't sound like a plain buzzer.
        let m = self.metal.tick(40.0, sr);
        let n = self.noise.tick();
        let mixed = m * self.metal_mix as f32 + n * (1.0 - self.metal_mix as f32) * 0.7;
        let shaped = self.bp.tick(mixed) * 1.2 + self.hp.tick(mixed) * 0.5;
        let out = if self.drive > 0.0 { saturate(shaped, self.drive as f32) } else { shaped };
        out * a as f32
    }
}

// ─────────────────────────── Clap ───────────────────────────

#[derive(Clone)]
pub struct Clap {
    noise: Noise,
    bp: BandPass,
    hp: OnePole,
    /// Burst envelope (three fast repeats) and the longer tail.
    burst: PercEnv,
    tail: PercEnv,
    burst_times: [f64; 3],
    burst_idx: usize,
    time: f64,
    tone: f64,
    body: f64,
    noise_rate: f64,
    pub spread: f64,
    vel: f64,
}

impl Clap {
    pub fn new(seed: u64) -> Self {
        let mut c = Self {
            noise: Noise::new(seed ^ 0xC1A9),
            bp: BandPass::new(1400.0, 1.1),
            hp: OnePole::highpass(800.0, SR),
            burst: PercEnv::new(0.0002, 0.010, 0.0),
            tail: PercEnv::new(0.0004, rate_to_seconds(10.0), 0.0),
            burst_times: [0.0, 0.011, 0.019],
            burst_idx: 0,
            time: 0.0,
            tone: 1400.0,
            body: 10.0,
            noise_rate: 35.0,
            spread: 0.5,
            vel: 1.0,
        };
        c.sync();
        c
    }

    fn sync(&mut self) {
        self.bp.set_cutoff(self.tone);
        self.tail.set(0.0004, rate_to_seconds(self.noise_rate), 0.0);
        let gap = 0.008 + self.spread as f64 * 0.008;
        self.burst_times = [0.0, gap, gap * 1.8];
    }

    pub fn trigger(&mut self, velocity: f64) {
        self.vel = velocity.clamp(0.05, 1.6);
        self.time = 0.0;
        self.burst_idx = 0;
        self.burst.trigger(self.vel);
        self.tail.trigger(self.vel);
    }

    pub fn reset(&mut self) {
        self.burst.reset();
        self.tail.reset();
        self.time = 0.0;
        self.burst_idx = 0;
    }

    pub fn active(&self) -> bool {
        self.tail.is_active()
    }

    #[inline]
    pub fn tick(&mut self, sr: f64) -> f32 {
        self.time += 1.0 / sr;
        if self.burst_idx < self.burst_times.len() && self.time >= self.burst_times[self.burst_idx] {
            self.burst.trigger(self.vel);
            self.burst_idx += 1;
        }
        let ba = self.burst.tick();
        let ta = self.tail.tick();
        if ba <= 0.0 && ta <= 0.0 {
            return 0.0;
        }
        let n = self.noise.tick();
        let shaped = self.bp.tick(n) * 1.6 + self.hp.tick(n) * 0.6;
        // Bursts dominate the attack; the tail is a shorter, darker noise bed.
        let amp = (ba * 1.0 + ta * 0.45) as f32;
        shaped * amp * 0.9
    }
}

// ─────────────────────────── Tom ───────────────────────────

#[derive(Clone)]
pub struct Tom {
    osc: Osc,
    noise: Noise,
    env: PercEnv,
    noise_env: PercEnv,
    freq: f64,
    pitch: f64,
    start_ratio: f64,
    sweep_rate: f64,
    decay: f64,
    noise_mix: f64,
    dc: DcBlocker,
    last_decay: f64,
}

impl Tom {
    pub fn new(seed: u64) -> Self {
        let mut t = Self {
            osc: Osc::new(Wave::Sine),
            noise: Noise::new(seed ^ 0x70A),
            env: PercEnv::new(0.0005, rate_to_seconds(20.0), 0.002),
            noise_env: PercEnv::new(0.0002, 0.012, 0.0),
            freq: 150.0,
            pitch: 150.0,
            start_ratio: 1.35,
            sweep_rate: 50.0,
            decay: 20.0,
            noise_mix: 0.3,
            dc: DcBlocker::default(),
            last_decay: 20.0,
        };
        t.sync();
        t
    }

    fn sync(&mut self) {
        self.env.set(0.0005, rate_to_seconds(self.decay), 0.002);
        self.last_decay = self.decay;
    }

    pub fn trigger(&mut self, velocity: f64) {
        self.freq = self.pitch * self.start_ratio;
        self.osc.reset();
        self.env.trigger(velocity.clamp(0.05, 1.6));
        self.noise_env.trigger(velocity.clamp(0.05, 1.6) * self.noise_mix);
        self.dc.reset();
    }

    pub fn reset(&mut self) {
        self.env.reset();
        self.noise_env.reset();
        self.osc.reset();
        self.freq = self.pitch * self.start_ratio;
        self.dc.reset();
    }

    pub fn active(&self) -> bool {
        self.env.is_active() || self.noise_env.is_active()
    }

    #[inline]
    pub fn tick(&mut self, sr: f64) -> f32 {
        if self.decay != self.last_decay {
            self.sync();
        }
        let a = self.env.tick();
        if a <= 0.0 {
            return 0.0;
        }
        let coef = 1.0 - (-(self.sweep_rate.max(1.0)) / sr * 4.6).exp();
        self.freq += (self.pitch - self.freq) * coef;
        let body = self.osc.tick(self.freq, sr) as f64 * a;
        let n = self.noise.tick() as f64 * self.noise_env.tick() * 0.5;
        self.dc.process((body + n) as f32) * 0.95
    }
}

// ─────────────────────────── Perc / Rim / Cowbell / Shaker ───────────────────────────

#[derive(Clone)]
pub struct Perc {
    metal: MetalBank,
    noise: Noise,
    env: PercEnv,
    bp: BandPass,
    hp: OnePole,
    decay: f64,
    tone: f64,
    metal_mix: f64,
}

impl Perc {
    pub fn new(seed: u64) -> Self {
        let mut p = Self {
            metal: MetalBank::new(),
            noise: Noise::new(seed ^ 0x9E4C),
            env: PercEnv::new(0.0002, rate_to_seconds(60.0), 0.0),
            bp: BandPass::new(2500.0, 1.4),
            hp: OnePole::highpass(1800.0, SR),
            decay: 60.0,
            tone: 2500.0,
            metal_mix: 0.5,
        };
        p.sync();
        p
    }

    fn sync(&mut self) {
        self.env.set(0.0002, rate_to_seconds(self.decay), 0.0);
        self.bp.set_cutoff(self.tone);
    }

    pub fn trigger(&mut self, velocity: f64) {
        self.env.trigger(velocity.clamp(0.05, 1.6));
        self.metal.reset();
    }

    pub fn reset(&mut self) {
        self.env.reset();
        self.metal.reset();
        self.bp.reset();
        self.hp.reset();
    }

    pub fn active(&self) -> bool {
        self.env.is_active()
    }

    #[inline]
    pub fn tick(&mut self, sr: f64) -> f32 {
        let a = self.env.tick();
        if a <= 0.0 {
            return 0.0;
        }
        let m = self.metal.tick(self.tone * 0.06, sr);
        let n = self.noise.tick();
        let mixed = m * self.metal_mix as f32 + n * 0.5;
        let out = self.bp.tick(mixed) * 1.4 + self.hp.tick(mixed) * 0.4;
        out * a as f32 * 0.8
    }
}

#[derive(Clone)]
pub struct Rim {
    osc: Osc,
    noise: Noise,
    env: PercEnv,
    bp: BandPass,
    decay: f64,
    tone: f64,
}

impl Rim {
    pub fn new(seed: u64) -> Self {
        let mut r = Self {
            osc: Osc::new(Wave::Square),
            noise: Noise::new(seed ^ 0x214D_0000),
            env: PercEnv::new(0.0001, rate_to_seconds(90.0), 0.0),
            bp: BandPass::new(1700.0, 2.0),
            decay: 90.0,
            tone: 1700.0,
        };
        r.sync();
        r
    }

    fn sync(&mut self) {
        self.env.set(0.0001, rate_to_seconds(self.decay), 0.0);
        self.bp.set_cutoff(self.tone);
    }

    pub fn trigger(&mut self, velocity: f64) {
        self.osc.reset();
        self.env.trigger(velocity.clamp(0.05, 1.6));
    }

    pub fn reset(&mut self) {
        self.env.reset();
        self.osc.reset();
        self.bp.reset();
    }

    pub fn active(&self) -> bool {
        self.env.is_active()
    }

    #[inline]
    pub fn tick(&mut self, sr: f64) -> f32 {
        let a = self.env.tick();
        if a <= 0.0 {
            return 0.0;
        }
        let o = self.osc.tick(self.tone, sr);
        let n = self.noise.tick() * 0.4;
        self.bp.tick(o + n) * 1.5 * a as f32
    }
}

#[derive(Clone)]
pub struct Cowbell {
    osc1: Osc,
    osc2: Osc,
    env: PercEnv,
    bp: BandPass,
    decay: f64,
    tone: f64,
}

impl Cowbell {
    pub fn new(_seed: u64) -> Self {
        let mut c = Self {
            osc1: Osc::new(Wave::Square),
            osc2: Osc::new(Wave::Square),
            env: PercEnv::new(0.0002, rate_to_seconds(30.0), 0.001),
            bp: BandPass::new(800.0, 1.2),
            decay: 30.0,
            tone: 540.0,
        };
        c.sync();
        c
    }

    fn sync(&mut self) {
        self.env.set(0.0002, rate_to_seconds(self.decay), 0.001);
        self.bp.set_cutoff(self.tone);
    }

    pub fn trigger(&mut self, velocity: f64) {
        self.osc1.reset();
        self.osc2.reset();
        self.env.trigger(velocity.clamp(0.05, 1.6));
    }

    pub fn reset(&mut self) {
        self.env.reset();
        self.osc1.reset();
        self.osc2.reset();
        self.bp.reset();
    }

    pub fn active(&self) -> bool {
        self.env.is_active()
    }

    #[inline]
    pub fn tick(&mut self, sr: f64) -> f32 {
        let a = self.env.tick();
        if a <= 0.0 {
            return 0.0;
        }
        let o = self.osc1.tick(self.tone, sr) * 0.5 + self.osc2.tick(self.tone * 1.5, sr) * 0.5;
        self.bp.tick(o) * 1.3 * a as f32
    }
}

#[derive(Clone)]
pub struct Shaker {
    noise: Noise,
    env: PercEnv,
    hp: OnePole,
    bp: BandPass,
    decay: f64,
    tone: f64,
}

impl Shaker {
    pub fn new(seed: u64) -> Self {
        let mut s = Self {
            noise: Noise::new(seed ^ 0x5AA0_0000),
            env: PercEnv::new(0.001, rate_to_seconds(40.0), 0.0),
            hp: OnePole::highpass(4000.0, SR),
            bp: BandPass::new(6000.0, 0.6),
            decay: 40.0,
            tone: 6000.0,
        };
        s.sync();
        s
    }

    fn sync(&mut self) {
        self.env.set(0.001, rate_to_seconds(self.decay), 0.0);
        self.hp = OnePole::highpass(self.tone * 0.6, SR);
        self.bp.set_cutoff(self.tone);
    }

    pub fn trigger(&mut self, velocity: f64) {
        self.env.trigger(velocity.clamp(0.05, 1.6));
    }

    pub fn reset(&mut self) {
        self.env.reset();
        self.hp.reset();
        self.bp.reset();
    }

    pub fn active(&self) -> bool {
        self.env.is_active()
    }

    #[inline]
    pub fn tick(&mut self, _sr: f64) -> f32 {
        let a = self.env.tick();
        if a <= 0.0 {
            return 0.0;
        }
        let n = self.noise.tick();
        (self.bp.tick(n) * 0.9 + self.hp.tick(n) * 0.7) * a as f32 * 0.7
    }
}

// ─────────────────────────── Cymbals ───────────────────────────

#[derive(Clone)]
pub struct Crash {
    metal: MetalBank,
    noise: Noise,
    env: PercEnv,
    hp: OnePole,
    bp: BandPass,
    decay: f64,
    tone: f64,
    vel: f64,
}

impl Crash {
    pub fn new(seed: u64) -> Self {
        let mut c = Self {
            metal: MetalBank::new(),
            noise: Noise::new(seed ^ 0xC2A5),
            env: PercEnv::new(0.0004, rate_to_seconds(1.6), 0.01),
            hp: OnePole::highpass(3000.0, SR),
            bp: BandPass::new(6000.0, 0.5),
            decay: 1.6,
            tone: 5000.0,
            vel: 1.0,
        };
        c.sync();
        c
    }

    fn sync(&mut self) {
        // Long, slightly non-exponential swell: model with a slower rate.
        self.env.set(0.0004, self.decay * 1.0, 0.01);
        self.hp = OnePole::highpass(self.tone * 0.5, SR);
    }

    pub fn trigger(&mut self, velocity: f64) {
        self.vel = velocity.clamp(0.05, 1.6);
        self.env.trigger(self.vel * 0.9);
        self.metal.reset();
    }

    pub fn reset(&mut self) {
        self.env.reset();
        self.metal.reset();
        self.hp.reset();
        self.bp.reset();
    }

    pub fn active(&self) -> bool {
        self.env.is_active()
    }

    #[inline]
    pub fn tick(&mut self, sr: f64) -> f32 {
        let a = self.env.tick();
        if a <= 0.0 {
            return 0.0;
        }
        let m = self.metal.tick(self.tone * 0.02, sr);
        let n = self.noise.tick();
        let mixed = m * 0.6 + n * 0.6;
        (self.bp.tick(mixed) * 0.8 + self.hp.tick(mixed) * 1.2) * a as f32 * 0.6
    }
}

#[derive(Clone)]
pub struct Ride {
    metal: MetalBank,
    noise: Noise,
    env: PercEnv,
    bp: BandPass,
    hp: OnePole,
    decay: f64,
    tone: f64,
    pub bell: f64,
}

impl Ride {
    pub fn new(seed: u64) -> Self {
        let mut r = Self {
            metal: MetalBank::new(),
            noise: Noise::new(seed ^ 0x21DE),
            env: PercEnv::new(0.0004, rate_to_seconds(2.2), 0.005),
            bp: BandPass::new(4500.0, 0.8),
            hp: OnePole::highpass(2500.0, SR),
            decay: 2.2,
            tone: 4500.0,
            bell: 0.4,
        };
        r.sync();
        r
    }

    fn sync(&mut self) {
        self.env.set(0.0004, self.decay, 0.005);
        self.bp.set_cutoff(self.tone);
    }

    pub fn trigger(&mut self, velocity: f64) {
        self.env.trigger(velocity.clamp(0.05, 1.6));
        self.metal.reset();
    }

    pub fn reset(&mut self) {
        self.env.reset();
        self.metal.reset();
        self.bp.reset();
        self.hp.reset();
    }

    pub fn active(&self) -> bool {
        self.env.is_active()
    }

    #[inline]
    pub fn tick(&mut self, sr: f64) -> f32 {
        let a = self.env.tick();
        if a <= 0.0 {
            return 0.0;
        }
        let m = self.metal.tick(self.tone * 0.03, sr);
        let n = self.noise.tick() * 0.25;
        let body = self.bp.tick(m + n);
        // The bell partial is a bright ping on top of the wash.
        let ping = (m * self.bell as f32) * 0.5;
        (self.hp.tick(body) + ping) * a as f32 * 0.7
    }
}
