//! Drum synthesis.
//!
//! Eleven self-contained analog-style drum models (kick, snare, hat, clap,
//! tom, perc, rim, cowbell, shaker, crash, ride). Every model is built from a
//! handful of private, sample-rate-aware building blocks defined in this file:
//!
//! * `Svf`     — TPT state-variable filter (Simper/Zavalishin), f64 state
//! * `OnePole` — TPT one-pole (used as the output DC blocker)
//! * `Env`     — raised-cosine attack, hold, exponential decay
//! * `Decay`   — bare exponential decay (pitch envelopes, secondary envelopes)
//! * `Bank`    — six band-limited (PolyBLEP) squares at 808 cymbal ratios
//!
//! Design rules followed by every model:
//! * no raw full-band noise: every noise source is band-, high- or low-passed;
//! * all oscillators are phase-accumulated (no discontinuities) and squares
//!   are band-limited;
//! * every hit starts with a short fade-in, and a retrigger while ringing
//!   keeps the previous hit as a tail that is faded out over ~1.5 ms;
//! * a hit is considered finished once its envelopes fall below -90 dBFS,
//!   after which the output is exactly `0.0`.
//!
//! Parameter units (keys and ranges are the contract in `voices.rs`). Every
//! decay-style control is "bigger = longer"; defaults reproduce the timing of
//! the previous implementation.

use super::util::Rng;
use std::f64::consts::PI;

const TWO_PI: f64 = 2.0 * PI;
const DEFAULT_SR: f64 = 44_100.0;
/// -90 dBFS: below this an envelope (already scaled to output level) is
/// considered silent.
const SILENCE: f64 = 3.162e-5;
/// Length of the retrigger crossfade applied to the previous hit's tail.
const TAIL_FADE_S: f64 = 0.0015;
/// Classic 808 cymbal/hat oscillator frequencies (Hz).
const BANK_HZ: [f64; 6] = [205.3, 304.4, 369.6, 522.7, 540.0, 800.0];

/* ───────────────────────── Building blocks ───────────────────────── */

/// TPT state-variable filter. `bp()` is normalised to unity gain at the
/// centre frequency.
#[derive(Clone, Copy, Debug, Default)]
struct Svf {
    k: f64,
    a1: f64,
    a2: f64,
    a3: f64,
    ic1: f64,
    ic2: f64,
}

#[derive(Clone, Copy)]
struct SvfOut {
    lp: f64,
    bp: f64,
    hp: f64,
}

impl Svf {
    fn set(&mut self, sr: f64, fc: f64, q: f64) {
        let fc = fc.clamp(5.0, 0.45 * sr);
        let g = (PI * fc / sr).tan();
        self.k = 1.0 / q.max(0.05);
        self.a1 = 1.0 / (1.0 + g * (g + self.k));
        self.a2 = g * self.a1;
        self.a3 = g * self.a2;
    }

    #[inline]
    fn tick(&mut self, x: f64) -> SvfOut {
        let v3 = x - self.ic2;
        let v1 = self.a1 * self.ic1 + self.a2 * v3;
        let v2 = self.ic2 + self.a2 * self.ic1 + self.a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        SvfOut { lp: v2, bp: self.k * v1, hp: x - self.k * v1 - v2 }
    }

    #[inline]
    fn lp(&mut self, x: f64) -> f64 {
        self.tick(x).lp
    }

    #[inline]
    fn bp(&mut self, x: f64) -> f64 {
        self.tick(x).bp
    }

    #[inline]
    fn hp(&mut self, x: f64) -> f64 {
        self.tick(x).hp
    }

    fn reset(&mut self) {
        self.ic1 = 0.0;
        self.ic2 = 0.0;
    }
}

/// TPT one-pole high-pass (DC blocker).
#[derive(Clone, Copy, Debug, Default)]
struct OnePole {
    g: f64,
    s: f64,
}

impl OnePole {
    fn set(&mut self, sr: f64, fc: f64) {
        let g = (PI * fc.clamp(0.1, 0.45 * sr) / sr).tan();
        self.g = g / (1.0 + g);
    }

    #[inline]
    fn hp(&mut self, x: f64) -> f64 {
        let v = (x - self.s) * self.g;
        let lp = v + self.s;
        self.s = lp + v;
        x - lp
    }

    fn reset(&mut self) {
        self.s = 0.0;
    }
}

/// One-shot amplitude envelope: raised-cosine attack, hold, exponential decay.
#[derive(Clone, Copy, Debug, Default)]
struct Env {
    attack: f64,
    hold: f64,
    tau: f64,
    a_n: f64,
    h_n: f64,
    mul: f64,
    t: f64,
    peak: f64,
    level: f64,
}

impl Env {
    /// `attack`/`hold` in seconds, `tau` = exponential time constant (s).
    fn set(&mut self, sr: f64, attack: f64, hold: f64, tau: f64) {
        self.attack = attack;
        self.hold = hold;
        self.tau = tau;
        self.a_n = (attack * sr).max(1.0);
        self.h_n = (hold * sr).max(0.0);
        self.mul = (-1.0 / (tau.max(1e-5) * sr)).exp();
    }

    fn trigger(&mut self, peak: f64) {
        self.t = 0.0;
        self.peak = peak.max(0.0);
        self.level = self.peak;
    }

    #[inline]
    fn tick(&mut self) -> f64 {
        if self.level <= 0.0 {
            return 0.0;
        }
        self.t += 1.0;
        if self.t < self.a_n {
            self.peak * 0.5 * (1.0 - (PI * self.t / self.a_n).cos())
        } else if self.t < self.a_n + self.h_n {
            self.level
        } else {
            self.level *= self.mul;
            if self.level < 1e-9 {
                self.level = 0.0;
            }
            self.level
        }
    }

    /// Upper bound of the envelope from now on.
    #[inline]
    fn level(&self) -> f64 {
        self.level
    }

    fn reset(&mut self) {
        self.level = 0.0;
        self.t = 0.0;
    }
}

/// Bare exponential decay from a start value (pitch envelopes etc.).
#[derive(Clone, Copy, Debug, Default)]
struct Decay {
    v: f64,
    mul: f64,
}

impl Decay {
    fn set(&mut self, sr: f64, tau: f64) {
        self.mul = (-1.0 / (tau.max(1e-5) * sr)).exp();
    }

    #[inline]
    fn tick(&mut self) -> f64 {
        let out = self.v;
        self.v *= self.mul;
        if self.v < 1e-12 {
            self.v = 0.0;
        }
        out
    }
}

/// Phase-accumulated sine.
#[derive(Clone, Copy, Debug, Default)]
struct Sine {
    phase: f64,
}

impl Sine {
    #[inline]
    fn tick(&mut self, dt: f64) -> f64 {
        let out = (TWO_PI * self.phase).sin();
        self.phase += dt;
        self.phase -= self.phase.floor();
        out
    }
}

#[inline]
fn poly_blep(t: f64, dt: f64) -> f64 {
    if t < dt {
        let x = t / dt;
        x + x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        x * x + x + x + 1.0
    } else {
        0.0
    }
}

/// Band-limited (PolyBLEP) square wave.
#[derive(Clone, Copy, Debug, Default)]
struct Square {
    phase: f64,
}

impl Square {
    #[inline]
    fn tick(&mut self, dt: f64) -> f64 {
        if dt <= 0.0 || dt >= 0.5 {
            // Above Nyquist/2 a square cannot be represented; stay silent.
            return 0.0;
        }
        let p = self.phase;
        let mut v = if p < 0.5 { 1.0 } else { -1.0 };
        v += poly_blep(p, dt);
        let mut p2 = p + 0.5;
        if p2 >= 1.0 {
            p2 -= 1.0;
        }
        v -= poly_blep(p2, dt);
        self.phase += dt;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        v
    }
}

/// Six band-limited squares at the 808 inharmonic ratios.
#[derive(Clone, Copy, Debug, Default)]
struct Bank {
    osc: [Square; 6],
    dt: [f64; 6],
}

impl Bank {
    fn new(rng: &mut Rng) -> Self {
        let mut b = Bank::default();
        // Free-running oscillators with seeded start phases, like the analog
        // original (deterministic for a given seed).
        for o in b.osc.iter_mut() {
            o.phase = rng.unipolar() as f64;
        }
        b
    }

    fn set(&mut self, sr: f64, scale: f64) {
        for (dt, hz) in self.dt.iter_mut().zip(BANK_HZ.iter()) {
            *dt = hz * scale / sr;
        }
    }

    /// Sum of the six squares, normalised to roughly ±1.
    #[inline]
    fn tick(&mut self) -> f64 {
        let mut s = 0.0;
        for i in 0..6 {
            s += self.osc[i].tick(self.dt[i]);
        }
        s * (1.0 / 6.0)
    }
}

/// Smooth saturator `tanh(k·x)/tanh(k)`: unity at |x| = 1, gentle knee.
#[derive(Clone, Copy, Debug, Default)]
struct Sat {
    k: f64,
    norm: f64,
}

impl Sat {
    /// `drive` 0..1 → `k` from `base` up to `base + range`.
    fn set(&mut self, drive: f64, base: f64, range: f64) {
        self.k = base + range * drive.clamp(0.0, 1.0);
        self.norm = 1.0 / self.k.tanh();
    }

    #[inline]
    fn tick(&self, x: f64) -> f64 {
        (self.k * x).tanh() * self.norm
    }

    /// Saturate a velocity-free signal for a hit of velocity `vel`. Velocity
    /// only nudges the drive (harder = a bit dirtier) and is otherwise
    /// applied after the saturator, so dynamics are not squashed.
    #[inline]
    fn hit(&self, x: f64, vel: f64) -> f64 {
        let d = 0.75 + 0.25 * vel.min(1.6);
        self.tick(x * d) * (vel / d)
    }

    /// Gain applied to very small signals (used for the silence threshold).
    fn small_gain(&self) -> f64 {
        self.k * self.norm
    }
}

#[inline]
fn noise(rng: &mut Rng) -> f64 {
    rng.bipolar() as f64
}

/* ───────────────────────────── Kick ───────────────────────────── */

/// Kick: sine body with an exponential pitch drop, optional filtered click,
/// soft-clip drive.
///
/// * `pitch` 20–200 Hz — final (resting) pitch.
/// * `decay` 0.5–40 — amp decay: -60 dB after `0.06 · decay` s (8 → 0.48 s,
///   40 → 2.4 s 808 boom).
/// * `sweep` 0.5–120 — pitch-drop depth: start pitch = `pitch · (1 + sweep/10)`
///   (30 → 4×); bigger sweeps also drop slightly faster.
/// * `click` 0–1 — level of a 1–2 ms band-passed beater transient.
/// * `drive` 0–1 — saturation amount (level-compensated).
/// * `hold` 0–0.2 s — time at full level before the decay starts.
#[derive(Clone)]
struct Kick {
    sr: f64,
    rng: Rng,
    pitch: f64,
    decay: f64,
    sweep: f64,
    click: f64,
    drive: f64,
    hold: f64,
    amp: Env,
    click_env: Env,
    glide: Decay,
    ratio: f64,
    osc: Sine,
    click_bp: Svf,
    click_lp: Svf,
    sat: Sat,
    vel: f64,
}

impl Kick {
    const OUT: f64 = 0.85;

    fn new(seed: u64) -> Self {
        let mut k = Kick {
            sr: DEFAULT_SR,
            rng: Rng::new(seed ^ 0x4B1C_4B1C),
            pitch: 50.0,
            decay: 8.0,
            sweep: 30.0,
            click: 0.22,
            drive: 0.3,
            hold: 0.004,
            amp: Env::default(),
            click_env: Env::default(),
            glide: Decay::default(),
            ratio: 4.0,
            osc: Sine::default(),
            click_bp: Svf::default(),
            click_lp: Svf::default(),
            sat: Sat::default(),
            vel: 1.0,
        };
        k.prepare();
        k
    }

    fn prepare(&mut self) {
        let sr = self.sr;
        let t60 = 0.06 * self.decay;
        self.amp.set(sr, 0.0006, self.hold, t60 / 6.9);
        self.click_env.set(sr, 0.0002, 0.0, 0.0012);
        self.ratio = 1.0 + self.sweep / 10.0;
        let ptau = 0.012 + 0.024 * 30.0 / (self.sweep + 30.0);
        self.glide.set(sr, ptau);
        self.click_bp.set(sr, 3200.0, 0.9);
        self.click_lp.set(sr, 9000.0, 0.707);
        self.sat.set(self.drive, 0.5, 5.0);
    }

    fn set_param(&mut self, key: &str, v: f64) {
        match key {
            "pitch" => self.pitch = v.clamp(20.0, 200.0),
            "decay" => self.decay = v.clamp(0.5, 40.0),
            "sweep" => self.sweep = v.clamp(0.5, 120.0),
            "click" => self.click = v.clamp(0.0, 1.0),
            "drive" => self.drive = v.clamp(0.0, 1.0),
            "hold" => self.hold = v.clamp(0.0, 0.2),
            _ => return,
        }
        self.prepare();
    }

    fn trigger(&mut self, vel: f64) {
        self.vel = vel;
        self.amp.trigger(1.0);
        self.click_env.trigger(self.click);
        // Harder hits start a little higher (more punch).
        self.glide.v = 0.8 + 0.2 * vel.min(1.5);
        self.osc.phase = 0.0;
        self.click_bp.reset();
        self.click_lp.reset();
    }

    fn reset(&mut self) {
        self.amp.reset();
        self.click_env.reset();
        self.glide.v = 0.0;
        self.click_bp.reset();
        self.click_lp.reset();
    }

    fn active(&self) -> bool {
        let g = self.vel * self.sat.small_gain() * Self::OUT;
        (self.amp.level() + self.click_env.level()) * g > SILENCE
    }

    #[inline]
    fn tick(&mut self) -> f64 {
        let f = self.pitch * (1.0 + (self.ratio - 1.0) * self.glide.tick());
        let body = self.osc.tick(f / self.sr) * self.amp.tick();
        let ce = self.click_env.tick();
        let mut click = 0.0;
        if ce > 0.0 {
            let n = noise(&mut self.rng);
            click = self.click_lp.lp(self.click_bp.bp(n)) * ce * 0.9;
        }
        self.sat.hit(body + click, self.vel) * Self::OUT
    }
}

/* ───────────────────────────── Snare ───────────────────────────── */

/// Snare: two detuned modal tones + filtered noise, snap transient and a
/// snare-wire buzz tail.
///
/// * `tone` 80–500 Hz — fundamental of the drum head (second mode at 1.62×).
/// * `body` 1–80 — tone decay, time constant `1.6 · body` ms (25 → 40 ms).
/// * `noise` 1–80 — noise decay, time constant `4.4 · noise` ms (15 → 66 ms).
/// * `snap` 0–1 — brightness/level of the initial noise crack.
/// * `buzz` 0–1 — level of the longer snare-wire rattle.
/// * `drive` 0–1 — saturation.
#[derive(Clone)]
struct Snare {
    sr: f64,
    rng: Rng,
    tone: f64,
    body: f64,
    noise: f64,
    snap: f64,
    buzz: f64,
    drive: f64,
    tone_env: Env,
    mode2_env: Env,
    noise_env: Env,
    snap_env: Env,
    buzz_env: Env,
    pitch_env: Decay,
    osc1: Sine,
    osc2: Sine,
    noise_hp: Svf,
    noise_lp: Svf,
    snap_hp: Svf,
    snap_lp: Svf,
    buzz_bp: Svf,
    sat: Sat,
    vel: f64,
}

impl Snare {
    const OUT: f64 = 0.8;

    fn new(seed: u64) -> Self {
        let mut s = Snare {
            sr: DEFAULT_SR,
            rng: Rng::new(seed ^ 0x5A4E_0000),
            tone: 200.0,
            body: 25.0,
            noise: 15.0,
            snap: 0.5,
            buzz: 0.15,
            drive: 0.2,
            tone_env: Env::default(),
            mode2_env: Env::default(),
            noise_env: Env::default(),
            snap_env: Env::default(),
            buzz_env: Env::default(),
            pitch_env: Decay::default(),
            osc1: Sine::default(),
            osc2: Sine::default(),
            noise_hp: Svf::default(),
            noise_lp: Svf::default(),
            snap_hp: Svf::default(),
            snap_lp: Svf::default(),
            buzz_bp: Svf::default(),
            sat: Sat::default(),
            vel: 1.0,
        };
        s.prepare();
        s
    }

    fn prepare(&mut self) {
        let sr = self.sr;
        let tb = 0.0016 * self.body;
        let tn = 0.0044 * self.noise;
        self.tone_env.set(sr, 0.0004, 0.001, tb);
        self.mode2_env.set(sr, 0.0004, 0.0, tb * 0.6);
        self.noise_env.set(sr, 0.0005, 0.001, tn);
        self.snap_env.set(sr, 0.0002, 0.0, 0.002 + 0.003 * self.snap);
        self.buzz_env.set(sr, 0.003, 0.0, tn * 2.2);
        self.pitch_env.set(sr, 0.006);
        self.noise_hp.set(sr, 900.0, 0.707);
        self.noise_lp.set(sr, 5000.0 + 6000.0 * self.snap, 0.707);
        self.snap_hp.set(sr, 3500.0, 0.707);
        self.snap_lp.set(sr, 14000.0, 0.707);
        self.buzz_bp.set(sr, 4500.0, 0.9);
        self.sat.set(self.drive, 0.6, 4.0);
    }

    fn set_param(&mut self, key: &str, v: f64) {
        match key {
            "tone" => self.tone = v.clamp(80.0, 500.0),
            "body" => self.body = v.clamp(1.0, 80.0),
            "noise" => self.noise = v.clamp(1.0, 80.0),
            "snap" => self.snap = v.clamp(0.0, 1.0),
            "buzz" => self.buzz = v.clamp(0.0, 1.0),
            "drive" => self.drive = v.clamp(0.0, 1.0),
            _ => return,
        }
        self.prepare();
    }

    fn trigger(&mut self, vel: f64) {
        self.vel = vel;
        self.tone_env.trigger(0.62);
        self.mode2_env.trigger(0.32);
        self.noise_env.trigger(0.5);
        self.snap_env.trigger(0.6 * self.snap);
        self.buzz_env.trigger(0.45 * self.buzz);
        self.pitch_env.v = 0.2 + 0.08 * vel.min(1.5);
        self.osc1.phase = 0.0;
        self.osc2.phase = 0.0;
        for f in [&mut self.noise_hp, &mut self.noise_lp, &mut self.snap_hp, &mut self.snap_lp, &mut self.buzz_bp] {
            f.reset();
        }
    }

    fn reset(&mut self) {
        for e in [&mut self.tone_env, &mut self.mode2_env, &mut self.noise_env, &mut self.snap_env, &mut self.buzz_env] {
            e.reset();
        }
        for f in [&mut self.noise_hp, &mut self.noise_lp, &mut self.snap_hp, &mut self.snap_lp, &mut self.buzz_bp] {
            f.reset();
        }
    }

    fn active(&self) -> bool {
        let lv = self.tone_env.level()
            + self.mode2_env.level()
            + self.noise_env.level()
            + self.snap_env.level()
            + self.buzz_env.level();
        lv * self.vel * self.sat.small_gain() * Self::OUT > SILENCE
    }

    #[inline]
    fn tick(&mut self) -> f64 {
        let sr = self.sr;
        let bend = 1.0 + self.pitch_env.tick();
        let s1 = self.osc1.tick(self.tone * bend / sr);
        let s2 = self.osc2.tick(self.tone * 1.62 * bend / sr);
        let tones = s1 * self.tone_env.tick() + s2 * self.mode2_env.tick();

        let n = noise(&mut self.rng);
        let body_noise = self.noise_lp.lp(self.noise_hp.hp(n)) * self.noise_env.tick();
        let crack = self.snap_lp.lp(self.snap_hp.hp(n)) * self.snap_env.tick();
        // Snare wires rattle in sympathy with the head: modulate by the tone.
        let wires = self.buzz_bp.bp(n) * (0.7 + 0.3 * s1) * self.buzz_env.tick();

        let x = tones + 1.6 * body_noise + 1.4 * crack + 1.8 * wires;
        self.sat.hit(x, self.vel) * Self::OUT
    }
}

/* ───────────────────────────── Hat ───────────────────────────── */

/// Closed/open hi-hat: 808 square bank + filtered noise through high-pass and
/// band-pass.
///
/// * `decay` 2–200 — time constant `0.6 · decay` ms (40 → 24 ms closed hat,
///   200 → 120 ms open hat).
/// * `tone` 2–14 kHz — band-pass centre (high-pass at 0.55×); the oscillator
///   bank is also scaled by `sqrt(tone / 9 kHz)`.
/// * `metal` 0–1 — blend between filtered noise (0) and square bank (1).
/// * `drive` 0–1 — saturation.
#[derive(Clone)]
struct Hat {
    sr: f64,
    rng: Rng,
    decay: f64,
    tone: f64,
    metal: f64,
    drive: f64,
    env: Env,
    bank: Bank,
    noise_lp: Svf,
    hp1: Svf,
    hp2: Svf,
    bp: Svf,
    sat: Sat,
    vel: f64,
}

impl Hat {
    const OUT: f64 = 0.75;

    fn new(seed: u64) -> Self {
        let mut rng = Rng::new(seed ^ 0x4841_5400);
        let bank = Bank::new(&mut rng);
        let mut h = Hat {
            sr: DEFAULT_SR,
            rng,
            decay: 40.0,
            tone: 9000.0,
            metal: 0.8,
            drive: 0.1,
            env: Env::default(),
            bank,
            noise_lp: Svf::default(),
            hp1: Svf::default(),
            hp2: Svf::default(),
            bp: Svf::default(),
            sat: Sat::default(),
            vel: 1.0,
        };
        h.prepare();
        h
    }

    fn prepare(&mut self) {
        let sr = self.sr;
        self.env.set(sr, 0.00015, 0.0, 0.0006 * self.decay);
        self.bank.set(sr, (self.tone / 9000.0).sqrt());
        self.noise_lp.set(sr, (self.tone * 1.6).min(16000.0), 0.707);
        self.hp1.set(sr, self.tone * 0.55, 0.707);
        self.hp2.set(sr, self.tone * 0.55, 0.707);
        self.bp.set(sr, self.tone, 1.1);
        self.sat.set(self.drive, 0.6, 4.0);
    }

    fn set_param(&mut self, key: &str, v: f64) {
        match key {
            "decay" => self.decay = v.clamp(2.0, 200.0),
            "tone" => self.tone = v.clamp(2000.0, 14000.0),
            "metal" => self.metal = v.clamp(0.0, 1.0),
            "drive" => self.drive = v.clamp(0.0, 1.0),
            _ => return,
        }
        self.prepare();
    }

    fn trigger(&mut self, vel: f64) {
        self.vel = vel;
        self.env.trigger(1.0);
    }

    fn reset(&mut self) {
        self.env.reset();
        for f in [&mut self.noise_lp, &mut self.hp1, &mut self.hp2, &mut self.bp] {
            f.reset();
        }
    }

    fn active(&self) -> bool {
        self.env.level() * self.vel * self.sat.small_gain() * Self::OUT > SILENCE
    }

    #[inline]
    fn tick(&mut self) -> f64 {
        let m = self.bank.tick();
        let n = self.noise_lp.lp(noise(&mut self.rng));
        let x = self.metal * m * 1.6 + (1.0 - self.metal) * n * 1.2;
        let h = self.hp2.hp(self.hp1.hp(x));
        let y = self.bp.bp(h) * 1.4 + h * 0.6;
        self.sat.hit(y * self.env.tick(), self.vel) * Self::OUT
    }
}

/* ───────────────────────────── Clap ───────────────────────────── */

/// Hand clap: four rapid band-passed noise bursts then a diffuse tail.
///
/// * `tone` 400–4000 Hz — band-pass centre.
/// * `body` 1–60 — length of each burst, time constant `0.3 + 0.27 · body` ms.
/// * `noise` 1–60 — tail decay, time constant `29 · 2^((noise − 35)/10)` ms
///   (35 → 29 ms, 60 → 164 ms).
/// * `spread` 0–1 — burst spacing `5 + 11 · spread` ms.
#[derive(Clone)]
struct Clap {
    sr: f64,
    rng: Rng,
    tone: f64,
    body: f64,
    noise: f64,
    spread: f64,
    burst: Env,
    tail: Env,
    offsets: [f64; 4],
    next_burst: usize,
    t: f64,
    bp: Svf,
    tail_bp: Svf,
    hp: Svf,
    vel: f64,
}

impl Clap {
    const OUT: f64 = 0.8;
    /// Relative spacing of the four bursts (slightly irregular, like hands).
    const PATTERN: [f64; 4] = [0.0, 1.0, 1.85, 2.9];

    fn new(seed: u64) -> Self {
        let mut c = Clap {
            sr: DEFAULT_SR,
            rng: Rng::new(seed ^ 0xC1A9_0000),
            tone: 1400.0,
            body: 10.0,
            noise: 35.0,
            spread: 0.5,
            burst: Env::default(),
            tail: Env::default(),
            offsets: [0.0; 4],
            next_burst: 4,
            t: 0.0,
            bp: Svf::default(),
            tail_bp: Svf::default(),
            hp: Svf::default(),
            vel: 1.0,
        };
        c.prepare();
        c
    }

    fn prepare(&mut self) {
        let sr = self.sr;
        self.burst.set(sr, 0.0002, 0.0, 0.0003 + 0.00027 * self.body);
        let tail_tau = 0.029 * 2f64.powf((self.noise - 35.0) / 10.0);
        self.tail.set(sr, 0.0008, 0.0, tail_tau);
        let gap = (0.005 + 0.011 * self.spread) * sr;
        for (o, p) in self.offsets.iter_mut().zip(Self::PATTERN.iter()) {
            *o = p * gap;
        }
        self.bp.set(sr, self.tone, 1.8);
        self.tail_bp.set(sr, self.tone * 0.9, 1.0);
        self.hp.set(sr, self.tone * 0.35, 0.707);
    }

    fn set_param(&mut self, key: &str, v: f64) {
        match key {
            "tone" => self.tone = v.clamp(400.0, 4000.0),
            "body" => self.body = v.clamp(1.0, 60.0),
            "noise" => self.noise = v.clamp(1.0, 60.0),
            "spread" => self.spread = v.clamp(0.0, 1.0),
            _ => return,
        }
        self.prepare();
    }

    fn trigger(&mut self, vel: f64) {
        self.vel = vel;
        self.t = 0.0;
        self.next_burst = 0;
        self.burst.reset();
        self.tail.reset();
        self.bp.reset();
        self.tail_bp.reset();
        self.hp.reset();
    }

    fn reset(&mut self) {
        self.next_burst = 4;
        self.burst.reset();
        self.tail.reset();
        self.bp.reset();
        self.tail_bp.reset();
        self.hp.reset();
    }

    fn active(&self) -> bool {
        self.next_burst < 4 || (self.burst.level() + self.tail.level()) * self.vel * Self::OUT > SILENCE
    }

    #[inline]
    fn tick(&mut self) -> f64 {
        if self.next_burst < 4 && self.t >= self.offsets[self.next_burst] {
            self.burst.trigger(1.0);
            if self.next_burst == 3 {
                self.tail.trigger(0.55);
            }
            self.next_burst += 1;
        }
        self.t += 1.0;
        let n = noise(&mut self.rng);
        // Envelope before the filters so every burst onset is smoothed by
        // the band-pass instead of clicking.
        let b = self.bp.bp(n * self.burst.tick());
        let tl = self.tail_bp.bp(n * self.tail.tick());
        let y = self.hp.hp(b * 1.9 + tl * 1.6);
        y * self.vel * Self::OUT
    }
}

/* ───────────────────────────── Tom ───────────────────────────── */

/// Tom: sine body with pitch sweep, a weaker inharmonic second mode and a
/// band-passed stick noise.
///
/// * `pitch` 40–500 Hz — resting pitch (hit starts ~1.4× higher).
/// * `decay` 0.5–60 — time constant `2.5 · decay` ms (20 → 50 ms).
/// * `sweep` 0.5–150 — pitch-drop speed, time constant `1.5 / sweep` s
///   (50 → 30 ms; bigger = tighter).
/// * `noise` 0–1 — stick-noise level.
#[derive(Clone)]
struct Tom {
    sr: f64,
    rng: Rng,
    pitch: f64,
    decay: f64,
    sweep: f64,
    noise: f64,
    amp: Env,
    mode2: Env,
    noise_env: Env,
    glide: Decay,
    osc: Sine,
    osc2: Sine,
    noise_bp: Svf,
    sat: Sat,
    vel: f64,
}

impl Tom {
    const OUT: f64 = 0.82;

    fn new(seed: u64) -> Self {
        let mut t = Tom {
            sr: DEFAULT_SR,
            rng: Rng::new(seed ^ 0x70A_0000),
            pitch: 150.0,
            decay: 20.0,
            sweep: 50.0,
            noise: 0.3,
            amp: Env::default(),
            mode2: Env::default(),
            noise_env: Env::default(),
            glide: Decay::default(),
            osc: Sine::default(),
            osc2: Sine::default(),
            noise_bp: Svf::default(),
            sat: Sat::default(),
            vel: 1.0,
        };
        t.prepare();
        t
    }

    fn prepare(&mut self) {
        let sr = self.sr;
        let tau = 0.0025 * self.decay;
        self.amp.set(sr, 0.0006, 0.002, tau);
        self.mode2.set(sr, 0.0006, 0.0, tau * 0.45);
        self.noise_env.set(sr, 0.0003, 0.0, 0.010);
        self.glide.set(sr, 1.5 / self.sweep);
        self.noise_bp.set(sr, (self.pitch * 8.0).clamp(800.0, 5000.0), 0.8);
        self.sat.set(0.15, 0.6, 1.0);
    }

    fn set_param(&mut self, key: &str, v: f64) {
        match key {
            "pitch" => self.pitch = v.clamp(40.0, 500.0),
            "decay" => self.decay = v.clamp(0.5, 60.0),
            "sweep" => self.sweep = v.clamp(0.5, 150.0),
            "noise" => self.noise = v.clamp(0.0, 1.0),
            _ => return,
        }
        self.prepare();
    }

    fn trigger(&mut self, vel: f64) {
        self.vel = vel;
        self.amp.trigger(0.9);
        self.mode2.trigger(0.18);
        self.noise_env.trigger(0.35 * self.noise);
        self.glide.v = 0.4;
        self.osc.phase = 0.0;
        self.osc2.phase = 0.0;
        self.noise_bp.reset();
    }

    fn reset(&mut self) {
        self.amp.reset();
        self.mode2.reset();
        self.noise_env.reset();
        self.noise_bp.reset();
    }

    fn active(&self) -> bool {
        let lv = self.amp.level() + self.mode2.level() + self.noise_env.level();
        lv * self.vel * self.sat.small_gain() * Self::OUT > SILENCE
    }

    #[inline]
    fn tick(&mut self) -> f64 {
        let f = self.pitch * (1.0 + self.glide.tick());
        let body = self.osc.tick(f / self.sr) * self.amp.tick();
        let m2 = self.osc2.tick(f * 1.59 / self.sr) * self.mode2.tick();
        let ne = self.noise_env.tick();
        let n = if ne > 0.0 { self.noise_bp.bp(noise(&mut self.rng)) * ne } else { 0.0 };
        self.sat.hit(body + m2 + n, self.vel) * Self::OUT
    }
}

/* ───────────────────────────── Perc ───────────────────────────── */

/// Generic percussion blip (also the fallback model): a pitched sine "wood"
/// tone blended with a band-passed metallic square bank.
///
/// * `decay` 4–300 — time constant `0.28 · decay` ms (60 → 17 ms).
/// * `tone` 200–8000 Hz — pitch of the blip / band-pass centre.
/// * `metal` 0–1 — blend from sine blip (0) to metallic bank (1).
#[derive(Clone)]
struct Perc {
    sr: f64,
    rng: Rng,
    decay: f64,
    tone: f64,
    metal: f64,
    env: Env,
    click_env: Env,
    glide: Decay,
    osc: Sine,
    bank: Bank,
    bp: Svf,
    click_hp: Svf,
    click_lp: Svf,
    vel: f64,
}

impl Perc {
    const OUT: f64 = 0.8;

    fn new(seed: u64) -> Self {
        let mut rng = Rng::new(seed ^ 0x9E4C_0000);
        let bank = Bank::new(&mut rng);
        let mut p = Perc {
            sr: DEFAULT_SR,
            rng,
            decay: 60.0,
            tone: 2500.0,
            metal: 0.5,
            env: Env::default(),
            click_env: Env::default(),
            glide: Decay::default(),
            osc: Sine::default(),
            bank,
            bp: Svf::default(),
            click_hp: Svf::default(),
            click_lp: Svf::default(),
            vel: 1.0,
        };
        p.prepare();
        p
    }

    fn prepare(&mut self) {
        let sr = self.sr;
        self.env.set(sr, 0.0003, 0.0, 0.00028 * self.decay);
        self.click_env.set(sr, 0.0001, 0.0, 0.0007);
        self.glide.set(sr, 0.004);
        self.bank.set(sr, self.tone / 1600.0);
        self.bp.set(sr, self.tone, 2.0);
        self.click_hp.set(sr, (self.tone * 1.5).min(9000.0), 0.707);
        self.click_lp.set(sr, 12000.0, 0.707);
    }

    fn set_param(&mut self, key: &str, v: f64) {
        match key {
            "decay" => self.decay = v.clamp(4.0, 300.0),
            "tone" => self.tone = v.clamp(200.0, 8000.0),
            "metal" => self.metal = v.clamp(0.0, 1.0),
            _ => return,
        }
        self.prepare();
    }

    fn trigger(&mut self, vel: f64) {
        self.vel = vel;
        self.env.trigger(1.0);
        self.click_env.trigger(0.25);
        self.glide.v = 0.3;
        self.osc.phase = 0.0;
        self.bp.reset();
        self.click_hp.reset();
        self.click_lp.reset();
    }

    fn reset(&mut self) {
        self.env.reset();
        self.click_env.reset();
        self.bp.reset();
        self.click_hp.reset();
        self.click_lp.reset();
    }

    fn active(&self) -> bool {
        (self.env.level() + self.click_env.level()) * self.vel * Self::OUT > SILENCE
    }

    #[inline]
    fn tick(&mut self) -> f64 {
        let e = self.env.tick();
        let f = self.tone * (1.0 + self.glide.tick());
        let s = self.osc.tick(f / self.sr);
        let m = self.bp.bp(self.bank.tick()) * 2.2;
        let ce = self.click_env.tick();
        let c = if ce > 0.0 { self.click_lp.lp(self.click_hp.hp(noise(&mut self.rng))) * ce } else { 0.0 };
        let x = (s * 0.9 * (1.0 - self.metal) + m * self.metal) * e + c;
        x * self.vel * Self::OUT
    }
}

/* ───────────────────────────── Rim ───────────────────────────── */

/// Rimshot: two short resonant sines (≈1 : 0.27, as in the 808) excited with
/// a high-passed noise click, high-passed and lightly saturated.
///
/// * `decay` 4–200 — time constant `0.123 · decay` ms (90 → 11 ms).
/// * `tone` 200–6000 Hz — upper resonance.
#[derive(Clone)]
struct Rim {
    sr: f64,
    rng: Rng,
    decay: f64,
    tone: f64,
    env: Env,
    low_env: Env,
    click_env: Env,
    osc_hi: Sine,
    osc_lo: Sine,
    click_bp: Svf,
    hp: Svf,
    sat: Sat,
    vel: f64,
}

impl Rim {
    const OUT: f64 = 0.75;

    fn new(seed: u64) -> Self {
        let mut r = Rim {
            sr: DEFAULT_SR,
            rng: Rng::new(seed ^ 0x214D_0000),
            decay: 90.0,
            tone: 1700.0,
            env: Env::default(),
            low_env: Env::default(),
            click_env: Env::default(),
            osc_hi: Sine::default(),
            osc_lo: Sine::default(),
            click_bp: Svf::default(),
            hp: Svf::default(),
            sat: Sat::default(),
            vel: 1.0,
        };
        r.prepare();
        r
    }

    fn prepare(&mut self) {
        let sr = self.sr;
        let tau = 0.000123 * self.decay;
        self.env.set(sr, 0.0002, 0.0, tau);
        self.low_env.set(sr, 0.0002, 0.0, tau * 1.3);
        self.click_env.set(sr, 0.0001, 0.0, 0.0006);
        self.click_bp.set(sr, (self.tone * 2.0).min(10000.0), 1.0);
        self.hp.set(sr, (self.tone * 0.15).max(60.0), 0.707);
        self.sat.set(0.4, 0.8, 2.0);
    }

    fn set_param(&mut self, key: &str, v: f64) {
        match key {
            "decay" => self.decay = v.clamp(4.0, 200.0),
            "tone" => self.tone = v.clamp(200.0, 6000.0),
            _ => return,
        }
        self.prepare();
    }

    fn trigger(&mut self, vel: f64) {
        self.vel = vel;
        self.env.trigger(0.7);
        self.low_env.trigger(0.45);
        self.click_env.trigger(0.5);
        self.osc_hi.phase = 0.0;
        self.osc_lo.phase = 0.0;
        self.click_bp.reset();
        self.hp.reset();
    }

    fn reset(&mut self) {
        self.env.reset();
        self.low_env.reset();
        self.click_env.reset();
        self.click_bp.reset();
        self.hp.reset();
    }

    fn active(&self) -> bool {
        let lv = self.env.level() + self.low_env.level() + self.click_env.level();
        lv * self.vel * self.sat.small_gain() * Self::OUT > SILENCE
    }

    #[inline]
    fn tick(&mut self) -> f64 {
        let hi = self.osc_hi.tick(self.tone / self.sr) * self.env.tick();
        let lo = self.osc_lo.tick(self.tone * 0.27 / self.sr) * self.low_env.tick();
        let ce = self.click_env.tick();
        let c = if ce > 0.0 { self.click_bp.bp(noise(&mut self.rng)) * ce } else { 0.0 };
        let y = self.hp.hp(hi + lo + c);
        self.sat.hit(y, self.vel) * Self::OUT
    }
}

/* ───────────────────────────── Cowbell ───────────────────────────── */

/// 808 cowbell: two band-limited squares (1 : 1.48) through band-passes,
/// with a two-stage envelope (sharp clank, then a ringing tail).
///
/// * `decay` 4–200 — tail time constant `1.1 · decay` ms (30 → 33 ms).
/// * `tone` 200–3000 Hz — lower square frequency (540 Hz default).
#[derive(Clone)]
struct Cowbell {
    sr: f64,
    decay: f64,
    tone: f64,
    clank: Env,
    ring: Env,
    sq1: Square,
    sq2: Square,
    bp: Svf,
    bp_hi: Svf,
    dt1: f64,
    dt2: f64,
    vel: f64,
}

impl Cowbell {
    const OUT: f64 = 0.78;

    fn new(_seed: u64) -> Self {
        let mut c = Cowbell {
            sr: DEFAULT_SR,
            decay: 30.0,
            tone: 540.0,
            clank: Env::default(),
            ring: Env::default(),
            sq1: Square::default(),
            sq2: Square::default(),
            bp: Svf::default(),
            bp_hi: Svf::default(),
            dt1: 0.0,
            dt2: 0.0,
            vel: 1.0,
        };
        c.prepare();
        c
    }

    fn prepare(&mut self) {
        let sr = self.sr;
        let tau = 0.0011 * self.decay;
        self.clank.set(sr, 0.0003, 0.0, tau.min(0.006));
        self.ring.set(sr, 0.0003, 0.0, tau * 1.6);
        self.dt1 = self.tone / sr;
        self.dt2 = self.tone * 1.4815 / sr;
        self.bp.set(sr, self.tone * 1.5, 1.6);
        self.bp_hi.set(sr, self.tone * 4.8, 3.0);
    }

    fn set_param(&mut self, key: &str, v: f64) {
        match key {
            "decay" => self.decay = v.clamp(4.0, 200.0),
            "tone" => self.tone = v.clamp(200.0, 3000.0),
            _ => return,
        }
        self.prepare();
    }

    fn trigger(&mut self, vel: f64) {
        self.vel = vel;
        self.clank.trigger(0.6);
        self.ring.trigger(0.4);
        self.sq1.phase = 0.0;
        self.sq2.phase = 0.0;
        self.bp.reset();
        self.bp_hi.reset();
    }

    fn reset(&mut self) {
        self.clank.reset();
        self.ring.reset();
        self.bp.reset();
        self.bp_hi.reset();
    }

    fn active(&self) -> bool {
        (self.clank.level() + self.ring.level()) * self.vel * Self::OUT > SILENCE
    }

    #[inline]
    fn tick(&mut self) -> f64 {
        let x = 0.5 * (self.sq1.tick(self.dt1) + self.sq2.tick(self.dt2));
        let y = self.bp.bp(x) + 0.35 * self.bp_hi.bp(x);
        let e = self.clank.tick() + self.ring.tick();
        y * e * self.vel * 1.25 * Self::OUT
    }
}

/* ───────────────────────────── Shaker ───────────────────────────── */

/// Shaker: band- and high-passed noise with a soft (~4 ms) attack.
///
/// * `decay` 4–300 — time constant `0.62 · decay` ms (40 → 25 ms).
/// * `tone` 2–14 kHz — band-pass centre.
#[derive(Clone)]
struct Shaker {
    sr: f64,
    rng: Rng,
    decay: f64,
    tone: f64,
    env: Env,
    bp: Svf,
    hp: Svf,
    lp: Svf,
    vel: f64,
}

impl Shaker {
    const OUT: f64 = 0.7;

    fn new(seed: u64) -> Self {
        let mut s = Shaker {
            sr: DEFAULT_SR,
            rng: Rng::new(seed ^ 0x5AA0_0000),
            decay: 40.0,
            tone: 6000.0,
            env: Env::default(),
            bp: Svf::default(),
            hp: Svf::default(),
            lp: Svf::default(),
            vel: 1.0,
        };
        s.prepare();
        s
    }

    fn prepare(&mut self) {
        let sr = self.sr;
        let tau = 0.00062 * self.decay;
        self.env.set(sr, 0.004_f64.min(tau * 2.0), 0.0, tau);
        self.bp.set(sr, self.tone, 1.2);
        self.hp.set(sr, self.tone * 0.6, 0.707);
        self.lp.set(sr, (self.tone * 2.0).min(17000.0), 0.707);
    }

    fn set_param(&mut self, key: &str, v: f64) {
        match key {
            "decay" => self.decay = v.clamp(4.0, 300.0),
            "tone" => self.tone = v.clamp(2000.0, 14000.0),
            _ => return,
        }
        self.prepare();
    }

    fn trigger(&mut self, vel: f64) {
        self.vel = vel;
        self.env.trigger(1.0);
    }

    fn reset(&mut self) {
        self.env.reset();
        self.bp.reset();
        self.hp.reset();
        self.lp.reset();
    }

    fn active(&self) -> bool {
        self.env.level() * self.vel * Self::OUT > SILENCE
    }

    #[inline]
    fn tick(&mut self) -> f64 {
        let n = self.lp.lp(noise(&mut self.rng));
        let y = self.bp.bp(n) * 1.0 + self.hp.hp(n) * 0.5;
        y * self.env.tick() * self.vel * 1.3 * Self::OUT
    }
}

/* ───────────────────────────── Crash ───────────────────────────── */

/// Crash cymbal: two detuned 808 square banks plus filtered noise, a bright
/// initial splash and a long, slowly darkening wash.
///
/// * `decay` 0.5–30 s — time to -40 dB of the wash (time constant `decay/4.6`).
/// * `tone` 2–14 kHz — brightness (band-pass centre, high-pass at 0.45×).
#[derive(Clone)]
struct Crash {
    sr: f64,
    rng: Rng,
    decay: f64,
    tone: f64,
    splash: Env,
    wash: Env,
    hi_env: Env,
    bank_a: Bank,
    bank_b: Bank,
    noise_lp: Svf,
    hp1: Svf,
    hp2: Svf,
    bp: Svf,
    air: Svf,
    vel: f64,
}

impl Crash {
    const OUT: f64 = 0.7;

    fn new(seed: u64) -> Self {
        let mut rng = Rng::new(seed ^ 0xC2A5_0000);
        let bank_a = Bank::new(&mut rng);
        let bank_b = Bank::new(&mut rng);
        let mut c = Crash {
            sr: DEFAULT_SR,
            rng,
            decay: 1.6,
            tone: 5000.0,
            splash: Env::default(),
            wash: Env::default(),
            hi_env: Env::default(),
            bank_a,
            bank_b,
            noise_lp: Svf::default(),
            hp1: Svf::default(),
            hp2: Svf::default(),
            bp: Svf::default(),
            air: Svf::default(),
            vel: 1.0,
        };
        c.prepare();
        c
    }

    fn prepare(&mut self) {
        let sr = self.sr;
        let tau = self.decay / 4.6;
        self.splash.set(sr, 0.0006, 0.0, 0.05_f64.min(tau));
        self.wash.set(sr, 0.0015, 0.0, tau);
        self.hi_env.set(sr, 0.0006, 0.0, tau * 0.55);
        let s = (self.tone / 5000.0).sqrt();
        self.bank_a.set(sr, s * 1.7);
        self.bank_b.set(sr, s * 2.45);
        self.noise_lp.set(sr, (self.tone * 2.6).min(16000.0), 0.707);
        self.hp1.set(sr, self.tone * 0.45, 0.707);
        self.hp2.set(sr, self.tone * 0.45, 0.707);
        self.bp.set(sr, self.tone, 0.7);
        self.air.set(sr, (self.tone * 1.8).min(0.4 * sr), 0.707);
    }

    fn set_param(&mut self, key: &str, v: f64) {
        match key {
            "decay" => self.decay = v.clamp(0.5, 30.0),
            "tone" => self.tone = v.clamp(2000.0, 14000.0),
            _ => return,
        }
        self.prepare();
    }

    fn trigger(&mut self, vel: f64) {
        self.vel = vel;
        self.splash.trigger(0.5);
        self.wash.trigger(0.5);
        self.hi_env.trigger(0.35);
    }

    fn reset(&mut self) {
        self.splash.reset();
        self.wash.reset();
        self.hi_env.reset();
        for f in [&mut self.noise_lp, &mut self.hp1, &mut self.hp2, &mut self.bp, &mut self.air] {
            f.reset();
        }
    }

    fn active(&self) -> bool {
        let lv = self.splash.level() + self.wash.level() + self.hi_env.level();
        lv * self.vel * 2.0 * Self::OUT > SILENCE
    }

    #[inline]
    fn tick(&mut self) -> f64 {
        let m = 0.5 * (self.bank_a.tick() + self.bank_b.tick());
        let n = self.noise_lp.lp(noise(&mut self.rng));
        let x = m * 1.2 + n * 0.8;
        let h = self.hp2.hp(self.hp1.hp(x));
        let body = self.bp.bp(h);
        let air = self.air.hp(h);
        let e = self.splash.tick() + self.wash.tick();
        let y = body * e * 1.6 + air * self.hi_env.tick() * 1.4;
        y * self.vel * Self::OUT
    }
}

/* ───────────────────────────── Ride ───────────────────────────── */

/// Ride cymbal: 808 square bank through a band-pass (the "ping"), a little
/// noise wash and an optional bell partial cluster.
///
/// * `decay` 0.5–30 s — time to -40 dB (time constant `decay/4.6`).
/// * `tone` 2–14 kHz — band-pass centre; bell partials start at `0.165 · tone`.
/// * `bell` 0–1 — bell cluster level.
#[derive(Clone)]
struct Ride {
    sr: f64,
    rng: Rng,
    decay: f64,
    tone: f64,
    bell: f64,
    ping: Env,
    wash: Env,
    bell_env: Env,
    bank: Bank,
    bells: [Sine; 5],
    bell_dt: [f64; 5],
    noise_lp: Svf,
    hp1: Svf,
    hp2: Svf,
    bp: Svf,
    vel: f64,
}

impl Ride {
    const OUT: f64 = 0.85;
    const BELL_RATIOS: [f64; 5] = [1.0, 1.47, 2.03, 2.71, 3.34];
    const BELL_AMPS: [f64; 5] = [1.0, 0.7, 0.55, 0.4, 0.3];

    fn new(seed: u64) -> Self {
        let mut rng = Rng::new(seed ^ 0x21DE_0000);
        let bank = Bank::new(&mut rng);
        let mut r = Ride {
            sr: DEFAULT_SR,
            rng,
            decay: 2.2,
            tone: 4500.0,
            bell: 0.4,
            ping: Env::default(),
            wash: Env::default(),
            bell_env: Env::default(),
            bank,
            bells: [Sine::default(); 5],
            bell_dt: [0.0; 5],
            noise_lp: Svf::default(),
            hp1: Svf::default(),
            hp2: Svf::default(),
            bp: Svf::default(),
            vel: 1.0,
        };
        r.prepare();
        r
    }

    fn prepare(&mut self) {
        let sr = self.sr;
        let tau = self.decay / 4.6;
        self.ping.set(sr, 0.0004, 0.0, 0.03_f64.min(tau));
        self.wash.set(sr, 0.0008, 0.0, tau);
        self.bell_env.set(sr, 0.0004, 0.0, tau * 0.7);
        self.bank.set(sr, (self.tone / 4500.0).sqrt() * 1.5);
        let fb = self.tone * 0.165;
        for (dt, r) in self.bell_dt.iter_mut().zip(Self::BELL_RATIOS.iter()) {
            *dt = fb * r / sr;
            if *dt >= 0.45 {
                *dt = 0.0;
            }
        }
        self.noise_lp.set(sr, (self.tone * 2.4).min(16000.0), 0.707);
        self.hp1.set(sr, self.tone * 0.5, 0.707);
        self.hp2.set(sr, self.tone * 0.5, 0.707);
        self.bp.set(sr, self.tone, 1.2);
    }

    fn set_param(&mut self, key: &str, v: f64) {
        match key {
            "decay" => self.decay = v.clamp(0.5, 30.0),
            "tone" => self.tone = v.clamp(2000.0, 14000.0),
            "bell" => self.bell = v.clamp(0.0, 1.0),
            _ => return,
        }
        self.prepare();
    }

    fn trigger(&mut self, vel: f64) {
        self.vel = vel;
        self.ping.trigger(0.5);
        self.wash.trigger(0.45);
        self.bell_env.trigger(0.12 * self.bell);
        for b in self.bells.iter_mut() {
            b.phase = 0.0;
        }
    }

    fn reset(&mut self) {
        self.ping.reset();
        self.wash.reset();
        self.bell_env.reset();
        for f in [&mut self.noise_lp, &mut self.hp1, &mut self.hp2, &mut self.bp] {
            f.reset();
        }
    }

    fn active(&self) -> bool {
        let lv = self.ping.level() + self.wash.level() + self.bell_env.level() * 3.0;
        lv * self.vel * 2.0 * Self::OUT > SILENCE
    }

    #[inline]
    fn tick(&mut self) -> f64 {
        let m = self.bank.tick();
        let n = self.noise_lp.lp(noise(&mut self.rng));
        let x = m + n * 0.25;
        let h = self.hp2.hp(self.hp1.hp(x));
        let body = self.bp.bp(h) * 1.5 + h * 0.4;
        let e = self.ping.tick() + self.wash.tick();
        let be = self.bell_env.tick();
        let mut bell = 0.0;
        if be > 0.0 {
            for i in 0..5 {
                bell += self.bells[i].tick(self.bell_dt[i]) * Self::BELL_AMPS[i];
            }
        }
        (body * e + bell * be) * self.vel * Self::OUT
    }
}

/* ───────────────────────────── Dispatch ───────────────────────────── */

#[derive(Clone)]
enum Model {
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

macro_rules! each {
    ($m:expr, $d:ident => $body:expr) => {
        match $m {
            Model::Kick($d) => $body,
            Model::Snare($d) => $body,
            Model::Hat($d) => $body,
            Model::Clap($d) => $body,
            Model::Tom($d) => $body,
            Model::Perc($d) => $body,
            Model::Rim($d) => $body,
            Model::Cowbell($d) => $body,
            Model::Shaker($d) => $body,
            Model::Crash($d) => $body,
            Model::Ride($d) => $body,
        }
    };
}

impl Model {
    fn from_kind(kind: &str, seed: u64) -> Model {
        match kind {
            "kick" => Model::Kick(Kick::new(seed)),
            "snare" => Model::Snare(Snare::new(seed)),
            "hat" => Model::Hat(Hat::new(seed)),
            "clap" => Model::Clap(Clap::new(seed)),
            "tom" => Model::Tom(Tom::new(seed)),
            "rim" => Model::Rim(Rim::new(seed)),
            "cowbell" => Model::Cowbell(Cowbell::new(seed)),
            "shaker" => Model::Shaker(Shaker::new(seed)),
            "crash" => Model::Crash(Crash::new(seed)),
            "ride" => Model::Ride(Ride::new(seed)),
            _ => Model::Perc(Perc::new(seed)),
        }
    }

    fn set_sample_rate(&mut self, sr: f64) {
        each!(self, d => {
            d.sr = sr;
            d.prepare();
        })
    }

    fn set_param(&mut self, key: &str, v: f64) {
        if v.is_finite() {
            each!(self, d => d.set_param(key, v))
        }
    }

    fn trigger(&mut self, vel: f64) {
        each!(self, d => d.trigger(vel))
    }

    fn reset(&mut self) {
        each!(self, d => d.reset())
    }

    fn active(&self) -> bool {
        each!(self, d => d.active())
    }

    #[inline]
    fn tick(&mut self) -> f64 {
        each!(self, d => d.tick())
    }
}

/* ───────────────────────────── Public voice ───────────────────────────── */

/// A drum voice: one model plus a short-lived copy of the previous hit that
/// is faded out on retrigger, and an output DC blocker.
#[derive(Clone)]
pub struct Drum {
    voice: Model,
    /// The previous hit, kept ringing for `TAIL_FADE_S` after a retrigger.
    tail: Model,
    tail_gain: f64,
    tail_step: f64,
    tail_on: bool,
    dc: OnePole,
    sr: f64,
}

impl Drum {
    /// Build a drum model by kind (`kick`, `snare`, `hat`, `clap`, `tom`,
    /// `perc`, `rim`, `cowbell`, `shaker`, `crash`, `ride`). Unknown kinds
    /// fall back to `perc`. `seed` makes all noise deterministic.
    pub fn from_kind(kind: &str, seed: u64) -> Drum {
        let voice = Model::from_kind(kind, seed);
        let mut d = Drum {
            tail: voice.clone(),
            voice,
            tail_gain: 0.0,
            tail_step: 0.0,
            tail_on: false,
            dc: OnePole::default(),
            sr: DEFAULT_SR,
        };
        d.set_sample_rate(DEFAULT_SR);
        d
    }

    /// Start a new hit. `velocity` is linear amplitude (0.05..~1.6).
    pub fn trigger(&mut self, velocity: f64) {
        let vel = if velocity.is_finite() { velocity.clamp(0.0, 2.0) } else { 1.0 };
        if self.voice.active() {
            // Keep the ringing hit alive for a short crossfade so the
            // retrigger does not click.
            self.tail.clone_from(&self.voice);
            self.tail_gain = 1.0;
            self.tail_on = true;
        }
        self.voice.trigger(vel);
    }

    pub fn reset(&mut self) {
        self.voice.reset();
        self.tail.reset();
        self.tail_on = false;
        self.tail_gain = 0.0;
        self.dc.reset();
    }

    /// True while the drum is still audible (above -90 dBFS).
    pub fn is_active(&self) -> bool {
        self.tail_on || self.voice.active()
    }

    /// One mono sample. Exactly `0.0` once the hit has decayed.
    #[inline]
    pub fn tick(&mut self, sr: f64) -> f32 {
        if sr != self.sr && sr.is_finite() && sr > 0.0 {
            self.set_sample_rate(sr);
        }
        let mut out = 0.0;
        let mut any = false;
        if self.voice.active() {
            out += self.voice.tick();
            any = true;
        }
        if self.tail_on {
            let g = self.tail_gain;
            out += self.tail.tick() * g * g * (3.0 - 2.0 * g);
            self.tail_gain -= self.tail_step;
            if self.tail_gain <= 0.0 {
                self.tail_on = false;
            }
            any = true;
        }
        if !any {
            self.dc.reset();
            return 0.0;
        }
        self.dc.hp(out) as f32
    }

    /// Re-derive every sample-rate-dependent coefficient.
    pub fn set_sample_rate(&mut self, sr: f64) {
        if !(sr.is_finite() && sr > 0.0) {
            return;
        }
        self.sr = sr;
        self.voice.set_sample_rate(sr);
        self.tail.set_sample_rate(sr);
        self.dc.set(sr, 5.0);
        self.tail_step = 1.0 / (TAIL_FADE_S * sr).max(1.0);
    }

    /// Apply one parameter by its catalog key. Unknown keys are ignored.
    pub fn set_param(&mut self, key: &str, value: f64) {
        self.voice.set_param(key, value);
    }
}

/* ───────────────────────────── Tests ───────────────────────────── */

#[cfg(test)]
mod tests {
    use super::*;

    const KINDS: [&str; 11] =
        ["kick", "snare", "hat", "clap", "tom", "perc", "rim", "cowbell", "shaker", "crash", "ride"];

    fn render(kind: &str, seed: u64, sr: f64, secs: f64) -> Vec<f32> {
        let mut d = Drum::from_kind(kind, seed);
        d.trigger(1.0);
        (0..(secs * sr) as usize).map(|_| d.tick(sr)).collect()
    }

    fn peak(x: &[f32]) -> f32 {
        x.iter().fold(0.0f32, |m, v| m.max(v.abs()))
    }

    /// Generous upper bound on the ring-out time at default params.
    fn max_len(kind: &str) -> f64 {
        match kind {
            "crash" => 6.0,
            "ride" => 8.0,
            _ => 2.5,
        }
    }

    #[test]
    fn finite_peak_and_inactive() {
        for kind in KINDS {
            for seed in [1u64, 7, 12345, 0xDEAD_BEEF] {
                let sr = 44_100.0;
                let mut d = Drum::from_kind(kind, seed);
                d.trigger(1.0);
                assert!(d.is_active(), "{kind} inactive right after trigger");
                let n = (max_len(kind) * sr) as usize;
                let mut pk = 0.0f32;
                let mut end = None;
                for i in 0..n {
                    let v = d.tick(sr);
                    assert!(v.is_finite(), "{kind} produced non-finite output");
                    pk = pk.max(v.abs());
                    if !d.is_active() {
                        end = Some(i);
                        break;
                    }
                }
                assert!(pk > 0.05 && pk <= 1.0, "{kind} seed {seed}: peak {pk}");
                assert!(end.is_some(), "{kind} still active after {} s", max_len(kind));
                for _ in 0..1000 {
                    assert_eq!(d.tick(sr), 0.0, "{kind} not silent after going inactive");
                }
            }
        }
    }

    #[test]
    fn deterministic() {
        for kind in KINDS {
            let a = render(kind, 42, 44_100.0, 0.3);
            let b = render(kind, 42, 44_100.0, 0.3);
            assert_eq!(a, b, "{kind} not deterministic");
        }
    }

    #[test]
    fn all_sample_rates_work() {
        for kind in KINDS {
            for sr in [8_000.0, 22_050.0, 48_000.0, 96_000.0, 192_000.0] {
                let x = render(kind, 3, sr, 0.5);
                assert!(x.iter().all(|v| v.is_finite()), "{kind}@{sr}");
                let pk = peak(&x);
                assert!(pk <= 1.2, "{kind}@{sr}: peak {pk}");
            }
        }
    }

    /// Frequency from the zero-crossing rate over a window.
    fn zc_freq(x: &[f32], sr: f64, from: f64, to: f64) -> f64 {
        let a = (from * sr) as usize;
        let b = (to * sr) as usize;
        let mut crossings = Vec::new();
        for i in a + 1..b {
            if x[i - 1] < 0.0 && x[i] >= 0.0 {
                let fr = x[i - 1] as f64 / (x[i - 1] as f64 - x[i] as f64);
                crossings.push((i - 1) as f64 + fr);
            }
        }
        let n = crossings.len();
        assert!(n >= 3);
        (n - 1) as f64 / ((crossings[n - 1] - crossings[0]) / sr)
    }

    #[test]
    fn kick_pitch_is_sample_rate_independent() {
        let mut fs = Vec::new();
        for sr in [48_000.0, 96_000.0] {
            let mut d = Drum::from_kind("kick", 9);
            d.set_param("click", 0.0);
            d.set_param("decay", 40.0);
            d.trigger(1.0);
            let x: Vec<f32> = (0..(sr as usize)).map(|_| d.tick(sr)).collect();
            fs.push(zc_freq(&x, sr, 0.25, 0.75));
        }
        assert!((fs[0] / fs[1] - 1.0).abs() < 0.03, "{fs:?}");
        assert!((fs[0] - 50.0).abs() < 3.0, "{fs:?}");
    }

    #[test]
    fn retrigger_does_not_click() {
        for kind in KINDS {
            let sr = 44_100.0;
            let mut d = Drum::from_kind(kind, 5);
            d.trigger(1.0);
            let mut prev = 0.0f32;
            // Retrigger at several offsets, including mid-cycle points.
            for i in 0..(0.4 * sr) as usize {
                if i == 1234 || i == 4567 || i == 9001 {
                    d.trigger(1.0);
                }
                let v = d.tick(sr);
                if i == 1234 || i == 4567 || i == 9001 {
                    assert!((v - prev).abs() < 0.5, "{kind}: jump {} at {i}", (v - prev).abs());
                }
                prev = v;
            }
        }
    }

    #[test]
    fn retrigger_max_jump_tonal() {
        // For purely tonal models the whole signal must stay smooth.
        for kind in ["kick", "tom"] {
            let sr = 44_100.0;
            let mut d = Drum::from_kind(kind, 5);
            d.set_param("click", 0.0);
            d.set_param("noise", 0.0);
            d.trigger(1.0);
            let mut prev = 0.0f32;
            for i in 0..(0.5 * sr) as usize {
                if i % 3001 == 1500 {
                    d.trigger(1.4);
                }
                let v = d.tick(sr);
                assert!((v - prev).abs() < 0.2, "{kind}: jump at {i}");
                prev = v;
            }
        }
    }

    #[test]
    fn velocity_scales_level() {
        for kind in KINDS {
            let sr = 44_100.0;
            let p = |vel: f64| {
                let mut d = Drum::from_kind(kind, 4);
                d.trigger(vel);
                (0..(0.3 * sr) as usize).map(|_| d.tick(sr).abs()).fold(0.0f32, f32::max)
            };
            let r = p(0.5) / p(1.0);
            assert!(r > 0.4 && r < 0.65, "{kind}: vel 0.5/1.0 peak ratio {r}");
        }
    }

    #[test]
    fn unknown_kind_and_params() {
        let mut d = Drum::from_kind("nope", 1);
        d.set_param("bogus", 3.0);
        d.set_param("tone", f64::NAN);
        d.trigger(1.0);
        let pk = (0..4410).map(|_| d.tick(44_100.0).abs()).fold(0.0f32, f32::max);
        assert!(pk > 0.05);
        d.reset();
        assert!(!d.is_active());
        assert_eq!(d.tick(44_100.0), 0.0);
    }

    #[test]
    fn extreme_params_stay_finite() {
        let sets: &[(&str, &[(&str, f64)])] = &[
            ("kick", &[("pitch", 200.0), ("sweep", 120.0), ("click", 1.0), ("drive", 1.0), ("decay", 40.0), ("hold", 0.2)]),
            ("snare", &[("tone", 500.0), ("body", 80.0), ("noise", 80.0), ("snap", 1.0), ("buzz", 1.0), ("drive", 1.0)]),
            ("hat", &[("decay", 200.0), ("tone", 14000.0), ("metal", 0.0), ("drive", 1.0)]),
            ("clap", &[("tone", 4000.0), ("body", 60.0), ("noise", 60.0), ("spread", 1.0)]),
            ("tom", &[("pitch", 500.0), ("decay", 60.0), ("sweep", 0.5), ("noise", 1.0)]),
            ("perc", &[("decay", 300.0), ("tone", 8000.0), ("metal", 1.0)]),
            ("rim", &[("decay", 4.0), ("tone", 6000.0)]),
            ("cowbell", &[("decay", 200.0), ("tone", 3000.0)]),
            ("shaker", &[("decay", 300.0), ("tone", 14000.0)]),
            ("crash", &[("decay", 0.5), ("tone", 14000.0)]),
            ("ride", &[("decay", 0.5), ("tone", 2000.0), ("bell", 1.0)]),
        ];
        for (kind, params) in sets {
            for sr in [8_000.0, 44_100.0, 192_000.0] {
                let mut d = Drum::from_kind(kind, 2);
                for (k, v) in params.iter() {
                    d.set_param(k, *v);
                }
                d.trigger(1.6);
                for _ in 0..(sr as usize) {
                    let v = d.tick(sr);
                    assert!(v.is_finite() && v.abs() < 3.0, "{kind}@{sr}: {v}");
                }
            }
        }
    }
}
