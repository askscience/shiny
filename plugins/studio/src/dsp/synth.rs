//! The melodic voice engine: a polyphonic subtractive/FM synth.
//!
//! One [`Synth`] instance backs every pitched instrument kind (`bass`, `lead`,
//! `pad`, `strings`, `brass`, `organ`, `ep`, `bell`, `sub`, `pluck`,
//! `synthme`). The *kind* only chooses default parameter values; the DSP path
//! is shared, so any instrument can grow into any other and the Grid can reuse
//! the same oscillators.
//!
//! Voice architecture (per note):
//!
//! ```text
//!  Osc A (unison, PWM/FM) ─┐
//!  Osc B (unison, ring)   ─┼─► Filter (SVF/2-pole, env + LFO + keytrack) ─► Amp (ADSR) ─► L/R
//!  Sub sine               ─┤
//!  Noise                  ─┘
//! ```

use std::collections::HashMap;

use super::env::Adsr;
use super::filter::{Filter, FilterKind};
use super::lfo::{Lfo, LfoShape};
use super::noise::Noise;
use super::osc::{Osc, Unison, Wave};
use super::util::{cents_ratio, DcBlocker, Rng};

/// A note message delivered to an instrument, sample-accurate within a block.
#[derive(Debug, Clone, Copy)]
pub enum NoteKind {
    On { freq: f64, velocity: f64 },
    Off,
}

#[derive(Debug, Clone, Copy)]
pub struct NoteMsg {
    /// Sample offset relative to the start of the current block.
    pub at: usize,
    /// The pattern's note id (so On/Off pair up).
    pub id: u64,
    pub kind: NoteKind,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum VoiceState {
    Idle,
    Active,
    Release,
}

/// Everything the synth needs, parsed once per parameter change.
#[derive(Clone)]
pub struct SynthParams {
    pub a_wave: Wave,
    pub a_level: f64,
    pub a_octave: f64,
    pub a_semi: f64,
    pub a_detune: f64,
    pub a_pw: f64,
    pub a_unison: usize,
    pub a_spread: f64,

    pub b_wave: Wave,
    pub b_level: f64,
    pub b_octave: f64,
    pub b_semi: f64,
    pub b_detune: f64,
    pub b_pw: f64,
    pub b_unison: usize,
    pub b_spread: f64,

    pub ring: f64,
    pub sub_level: f64,
    pub sub_octave: f64,
    pub noise_level: f64,

    pub fm_ratio: f64,
    pub fm_index: f64,
    pub fm_decay: f64,
    pub fm_self: bool,

    pub f_type: FilterKind,
    pub f_cutoff: f64,
    pub f_res: f64,
    pub f_drive: f64,
    pub f_poles: u8,
    pub f_env: f64,
    pub f_keytrack: f64,
    pub f_lfo: f64,

    pub attack: f64,
    pub decay: f64,
    pub sustain: f64,
    pub release: f64,
    pub env_shape: f64,

    pub f_attack: f64,
    pub f_decay: f64,
    pub f_sustain: f64,
    pub f_release: f64,

    pub lfo_rate: f64,
    pub lfo_wave: LfoShape,
    pub lfo_pitch: f64,
    pub lfo_cutoff: f64,
    pub lfo_amp: f64,
    pub lfo_pan: f64,

    pub glide: f64,
    pub vel_amp: f64,
    pub vel_filter: f64,
    pub poly: usize,
    pub pan_spread: f64,
    pub analog: f64,
    pub level: f64,
}

impl Default for SynthParams {
    fn default() -> Self {
        Self {
            a_wave: Wave::Saw,
            a_level: 0.7,
            a_octave: 0.0,
            a_semi: 0.0,
            a_detune: 0.0,
            a_pw: 0.5,
            a_unison: 1,
            a_spread: 0.5,
            b_wave: Wave::Square,
            b_level: 0.0,
            b_octave: 0.0,
            b_semi: 0.0,
            b_detune: 0.1,
            b_pw: 0.5,
            b_unison: 1,
            b_spread: 0.5,
            ring: 0.0,
            sub_level: 0.0,
            sub_octave: -1.0,
            noise_level: 0.0,
            fm_ratio: 2.0,
            fm_index: 0.0,
            fm_decay: 0.3,
            fm_self: false,
            f_type: FilterKind::LowPass,
            f_cutoff: 3000.0,
            f_res: 0.9,
            f_drive: 1.0,
            f_poles: 1,
            f_env: 0.0,
            f_keytrack: 0.35,
            f_lfo: 0.0,
            attack: 0.005,
            decay: 0.2,
            sustain: 0.7,
            release: 0.2,
            env_shape: 0.0,
            f_attack: 0.005,
            f_decay: 0.3,
            f_sustain: 0.4,
            f_release: 0.3,
            lfo_rate: 3.0,
            lfo_wave: LfoShape::Sine,
            lfo_pitch: 0.0,
            lfo_cutoff: 0.0,
            lfo_amp: 0.0,
            lfo_pan: 0.0,
            glide: 0.0,
            vel_amp: 0.7,
            vel_filter: 0.4,
            poly: 10,
            pan_spread: 0.0,
            analog: 0.15,
            level: 1.0,
        }
    }
}

fn g(map: &HashMap<String, f64>, keys: &[&str], default: f64) -> f64 {
    for k in keys {
        if let Some(v) = map.get(*k) {
            return *v;
        }
    }
    default
}

impl SynthParams {
    /// Parse the UI/JSON parameter map. `wave` is the legacy bass `wave` field.
    pub fn from_map(map: &HashMap<String, f64>, wave: Option<&str>, kind: &str) -> Self {
        let mut p = SynthParams::default();
        if let Some(w) = wave {
            p.a_wave = match w {
                "sine" => Wave::Sine,
                "triangle" | "tri" => Wave::Triangle,
                "square" => Wave::Square,
                "pulse" => Wave::Pulse,
                _ => Wave::Saw,
            };
            if matches!(kind, "bass" | "sub") && p.a_wave == Wave::Sine {
                p.a_level = 0.9;
            }
        }
        p.a_wave = Wave::from_index(g(map, &["o1w"], p.a_wave_index()));
        p.b_wave = Wave::from_index(g(map, &["o2w"], 3.0));
        p.a_level = g(map, &["a_level", "osc1_level"], p.a_level);
        p.b_level = g(map, &["b_level", "osc2_level"], if map.contains_key("o2w") { 0.4 } else { p.b_level });
        p.a_octave = g(map, &["a_octave"], p.a_octave);
        p.b_octave = g(map, &["b_octave"], p.b_octave);
        p.a_semi = g(map, &["a_semi"], 0.0);
        p.b_semi = g(map, &["b_semi", "detune"], p.b_semi);
        p.a_detune = g(map, &["a_detune"], 0.0);
        p.b_detune = g(map, &["b_detune"], p.b_detune);
        p.a_pw = g(map, &["pw", "a_pw"], 0.5);
        p.b_pw = g(map, &["b_pw"], 0.5);
        p.a_unison = g(map, &["unison", "a_unison"], 1.0).round().clamp(1.0, 12.0) as usize;
        p.b_unison = g(map, &["b_unison"], 1.0).round().clamp(1.0, 12.0) as usize;
        p.a_spread = g(map, &["spread", "a_spread"], 0.5);
        p.b_spread = g(map, &["b_spread"], 0.5);
        p.ring = g(map, &["ring"], 0.0);
        p.sub_level = g(map, &["sub", "sub_level"], 0.0);
        p.sub_octave = g(map, &["sub_octave"], -1.0);
        p.noise_level = g(map, &["noise", "noise_level"], 0.0);
        p.fm_ratio = g(map, &["fm_ratio"], 2.0);
        p.fm_index = g(map, &["fm_index"], 0.0);
        p.fm_decay = g(map, &["fm_decay"], 0.3);
        p.fm_self = g(map, &["fm_self"], 0.0) > 0.5;
        p.f_type = FilterKind::from_index(g(map, &["ftype", "type"], 0.0));
        p.f_cutoff = g(map, &["cutoff"], 3000.0).clamp(20.0, 20000.0);
        p.f_res = g(map, &["res", "resonance"], 0.9).clamp(0.4, 24.0);
        p.f_drive = g(map, &["drive"], 1.0).clamp(0.05, 24.0);
        p.f_poles = g(map, &["poles"], 1.0).round().clamp(1.0, 2.0) as u8;
        p.f_env = g(map, &["fenv", "filter_env"], 0.0);
        p.f_keytrack = g(map, &["keytrack"], 0.35);
        p.f_lfo = g(map, &["lfo_depth", "lfo_cutoff"], 0.0);
        p.attack = g(map, &["attack"], 0.005).max(0.0002);
        p.decay = g(map, &["decay"], 0.2).max(0.0002);
        p.sustain = g(map, &["sustain"], 0.7).clamp(0.0, 1.0);
        p.release = g(map, &["release"], 0.2).max(0.0002);
        p.env_shape = g(map, &["env_shape", "curve"], 0.0).clamp(0.0, 1.0);
        p.f_attack = g(map, &["fattack"], p.attack);
        p.f_decay = g(map, &["fdecay"], p.decay);
        p.f_sustain = g(map, &["fsustain"], p.sustain);
        p.f_release = g(map, &["frelease"], p.release);
        p.lfo_rate = g(map, &["lfo_rate"], 3.0).clamp(0.01, 60.0);
        p.lfo_wave = LfoShape::from_index(g(map, &["lfo_wave"], 0.0));
        p.lfo_pitch = g(map, &["lfo_pitch"], 0.0);
        p.lfo_amp = g(map, &["lfo_amp"], 0.0);
        p.lfo_pan = g(map, &["lfo_pan"], 0.0);
        p.glide = g(map, &["glide", "portamento"], 0.0).clamp(0.0, 2.0);
        p.vel_amp = g(map, &["vel_amp"], 0.7).clamp(0.0, 1.0);
        p.vel_filter = g(map, &["vel_filter"], 0.4).clamp(0.0, 1.0);
        p.poly = g(map, &["poly"], 10.0).round().clamp(1.0, 24.0) as usize;
        p.pan_spread = g(map, &["pan_spread"], 0.0).clamp(0.0, 1.0);
        p.analog = g(map, &["analog", "drift"], 0.15).clamp(0.0, 1.0);
        p.level = g(map, &["level"], 1.0).clamp(0.0, 2.0);
        p
    }

    fn a_wave_index(&self) -> f64 {
        match self.a_wave {
            Wave::Sine => 0.0,
            Wave::Triangle => 1.0,
            Wave::Saw => 2.0,
            Wave::Square => 3.0,
            Wave::Pulse => 4.0,
            Wave::Noise => 5.0,
            Wave::Organ => 6.0,
        }
    }
}

/// One polyphonic voice.
struct SynthVoice {
    a: Unison,
    b: Unison,
    sub: Osc,
    noise: Noise,
    fm: Osc,
    filter: Filter,
    amp: Adsr,
    fenv: Adsr,
    freq: f64,
    target: f64,
    note: f64,
    velocity: f64,
    state: VoiceState,
    id: u64,
    order: u64,
    pan: f32,
    drift: f64,
    dc_l: DcBlocker,
    dc_r: DcBlocker,
    fm_env: Adsr,
}

impl SynthVoice {
    fn new(seed: u64, params: &SynthParams, sr: f64) -> Self {
        let mut rng = Rng::new(seed);
        let drift = (rng.bipolar() as f64) * 4.0;
        let pan = rng.bipolar();
        let mut v = Self {
            a: Unison::new(params.a_unison, params.a_wave, params.a_detune, params.a_spread, seed ^ 0xA1),
            b: Unison::new(params.b_unison.max(1), params.b_wave, params.b_detune, params.b_spread, seed ^ 0xB2),
            sub: Osc::new(Wave::Sine).with_seed(seed ^ 0x5B),
            noise: Noise::new(seed ^ 0x0F),
            fm: Osc::new(Wave::Sine).with_seed(seed ^ 0xF0),
            filter: Filter::new(params.f_type, params.f_cutoff, params.f_res),
            amp: Adsr::new(params.attack, params.decay, params.sustain, params.release),
            fenv: Adsr::new(params.f_attack, params.f_decay, params.f_sustain, params.f_release),
            freq: 440.0,
            target: 440.0,
            note: 69.0,
            velocity: 1.0,
            state: VoiceState::Idle,
            id: 0,
            order: 0,
            pan: pan * 0.0,
            drift,
            dc_l: DcBlocker::default(),
            dc_r: DcBlocker::default(),
            fm_env: Adsr::new(0.001, params.fm_decay, 0.0, 0.1),
        };
        v.pan = pan;
        v.filter.set_sample_rate(sr);
        v
    }

    fn configure(&mut self, p: &SynthParams) {
        self.filter.kind = p.f_type;
        self.filter.set_q(p.f_res);
        self.filter.set_drive(p.f_drive);
        self.filter.set_poles(p.f_poles);
        self.amp.set_times(p.attack, p.decay, p.sustain, p.release);
        self.amp.set_shape(p.env_shape);
        self.fenv.set_times(p.f_attack, p.f_decay, p.f_sustain, p.f_release);
        self.fenv.set_shape(p.env_shape);
        self.a.set_wave(p.a_wave);
        self.b.set_wave(p.b_wave);
        self.a.set_pulse_width(p.a_pw);
        self.b.set_pulse_width(p.b_pw);
    }
}

/// The instrument: a voice pool plus a global LFO and output stage.
pub struct Synth {
    params: SynthParams,
    map: HashMap<String, f64>,
    voices: Vec<SynthVoice>,
    lfo: Lfo,
    sr: f64,
    order: u64,
    base_seed: u64,
    pub last_lfo: f32,
}

impl Synth {
    pub fn new(kind: &str, wave: Option<&str>, map: &HashMap<String, f64>, seed: u64, sr: f64) -> Self {
        let mut params = SynthParams::from_map(map, wave, kind);
        // Mono instruments get a single voice and glide; pads/leads get real
        // polyphony by default.
        let default_poly = match kind {
            "bass" | "sub" => 1,
            "pluck" | "ep" | "organ" | "bell" => 10,
            "pad" | "strings" | "brass" => 12,
            "lead" | "synthme" | "fm" => 10,
            _ => 10,
        };
        if !map.contains_key("poly") {
            params.poly = default_poly;
        }
        let mut synth = Self {
            params: params.clone(),
            map: map.clone(),
            voices: (0..params.poly).map(|i| SynthVoice::new(seed.wrapping_add(i as u64 * 2654435761), &params, sr)).collect(),
            lfo: Lfo::new(params.lfo_wave, seed ^ 0x10F0),
            sr,
            order: 0,
            base_seed: seed,
            last_lfo: 0.0,
        };
        synth.apply_map(map, wave, kind);
        synth
    }

    /// Re-apply the parameter map (called whenever automation or the UI
    /// changes a synth parameter).
    pub fn apply_map(&mut self, map: &HashMap<String, f64>, wave: Option<&str>, kind: &str) {
        let params = SynthParams::from_map(map, wave, kind);
        let poly = params.poly;
        if poly != self.voices.len() {
            // Resize the pool without dropping the notes that are still held.
            if poly > self.voices.len() {
                for i in self.voices.len()..poly {
                    self.voices.push(SynthVoice::new(self.base_seed.wrapping_add(i as u64 * 2654435761), &params, self.sr));
                }
            } else {
                // Steal from the end — prefer releasing/idle voices.
                let mut idx: Vec<usize> = (0..self.voices.len()).collect();
                idx.sort_by_key(|i| match self.voices[*i].state {
                    VoiceState::Idle => 0,
                    VoiceState::Release => 1,
                    VoiceState::Active => 2,
                });
                let mut drop: Vec<usize> = idx.into_iter().take(self.voices.len() - poly).collect();
                drop.sort_unstable_by(|a, b| b.cmp(a));
                for i in drop {
                    self.voices.remove(i);
                }
            }
        }
        for v in self.voices.iter_mut() {
            v.configure(&params);
        }
        self.lfo = Lfo::new(params.lfo_wave, self.base_seed ^ 0x10F0);
        self.params = params;
        self.map = map.clone();
    }

    pub fn params(&self) -> &SynthParams {
        &self.params
    }

    /// Number of voices currently producing sound (for UI metering).
    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.state != VoiceState::Idle).count()
    }

    /// Rebuild unison stacks after a structural change (already handled by
    /// `apply_map`; kept for API symmetry with the drums).
    pub fn map(&self) -> &HashMap<String, f64> {
        &self.map
    }

    pub fn note_on(&mut self, id: u64, freq: f64, velocity: f64) {
        self.order += 1;
        let order = self.order;
        // Pick an idle voice; else the oldest releasing; else steal the oldest.
        let mut chosen: Option<usize> = None;
        let mut best_score = i64::MIN;
        for (i, v) in self.voices.iter().enumerate() {
            let score = match v.state {
                VoiceState::Idle => 1_000_000,
                VoiceState::Release => 500_000 - v.order as i64,
                VoiceState::Active => {
                    // Steal the quietest/oldest active voice.
                    if v.id == id {
                        2_000_000
                    } else {
                        100_000 - v.order as i64
                    }
                }
            };
            if score > best_score {
                best_score = score;
                chosen = Some(i);
            }
        }
        let Some(i) = chosen else { return };
        let v = &mut self.voices[i];
        let retrigger = v.state == VoiceState::Idle || v.id != id;
        v.id = id;
        v.order = order;
        v.note = 69.0 + 12.0 * (freq.max(1e-6) / 440.0).log2();
        v.velocity = velocity.clamp(0.05, 1.6);
        if v.state == VoiceState::Idle || self.params.glide <= 0.0 {
            v.target = freq;
            if v.state == VoiceState::Idle {
                v.freq = freq;
            }
        } else {
            v.target = freq;
        }
        if retrigger || v.state == VoiceState::Idle {
            v.amp.gate_on();
            v.fenv.gate_on();
            v.fm_env.gate_on();
            if v.state == VoiceState::Idle {
                v.a.reset();
                v.b.reset();
                v.sub.reset();
                v.filter.reset();
            }
        }
        v.state = VoiceState::Active;
    }

    pub fn note_off(&mut self, id: u64) {
        for v in self.voices.iter_mut() {
            if v.id == id && v.state == VoiceState::Active {
                v.amp.gate_off();
                v.fenv.gate_off();
                v.fm_env.gate_off();
                v.state = VoiceState::Release;
            }
        }
    }

    pub fn all_off(&mut self) {
        for v in self.voices.iter_mut() {
            v.amp.gate_off();
            v.fenv.gate_off();
            v.fm_env.gate_off();
            if v.state == VoiceState::Active {
                v.state = VoiceState::Release;
            }
        }
    }

    pub fn reset(&mut self) {
        for v in self.voices.iter_mut() {
            v.amp.reset();
            v.fenv.reset();
            v.fm_env.reset();
            v.filter.reset();
            v.a.reset();
            v.b.reset();
            v.state = VoiceState::Idle;
        }
        self.lfo.reset();
    }

    /// Render `frames` samples into the stereo buses, applying the messages
    /// whose offsets fall inside this block.
    pub fn render(&mut self, out_l: &mut [f32], out_r: &mut [f32], msgs: &[NoteMsg], frames: usize, sr: f64) {
        self.sr = sr;
        let p = self.params.clone();
        let mut mi = 0usize;
        let glide_coef = if p.glide > 0.0 { 1.0 - (-1.0 / (p.glide * sr)).exp() } else { 1.0 };
        let lfo_rate = p.lfo_rate;
        let lfo_pitch = p.lfo_pitch;
        let lfo_cut = p.lfo_cutoff;
        let lfo_amp = p.lfo_amp;
        let lfo_pan = p.lfo_pan;

        for i in 0..frames {
            while mi < msgs.len() && msgs[mi].at <= i {
                match msgs[mi].kind {
                    NoteKind::On { freq, velocity } => self.note_on(msgs[mi].id, freq, velocity),
                    NoteKind::Off => self.note_off(msgs[mi].id),
                }
                mi += 1;
            }
            let lfo = self.lfo.tick(lfo_rate, sr);
            self.last_lfo = lfo;
            let mut sum_l = 0.0f32;
            let mut sum_r = 0.0f32;
            for v in self.voices.iter_mut() {
                if v.state == VoiceState::Idle {
                    continue;
                }
                if v.state == VoiceState::Release && !v.amp.is_active() {
                    v.state = VoiceState::Idle;
                    continue;
                }
                // Portamento.
                if p.glide > 0.0 {
                    v.freq += (v.target - v.freq) * glide_coef;
                } else {
                    v.freq = v.target;
                }
                let amp = v.amp.tick();
                let fenv = v.fenv.tick();
                let fm_env = v.fm_env.tick();
                if v.state == VoiceState::Release && amp <= 0.0 {
                    v.state = VoiceState::Idle;
                    continue;
                }

                // Analog drift: a fixed per-voice detune keeps stacked voices
                // from phase-locking into a comb.
                let drift = 1.0 + v.drift * 1e-5 * p.analog;
                let lfo_f = lfo as f64;
                let pitch_mod = lfo_f * lfo_pitch;
                let f = v.freq * drift * cents_ratio(pitch_mod * 100.0);

                // Oscillator section.
                let (mut l, mut r) = if p.fm_index > 0.0 {
                    let modv = v.fm.tick(f * p.fm_ratio, sr) as f64 * p.fm_index * fm_env * 0.5;
                    v.a.tick_pm(f * 2f64.powf(p.a_semi / 12.0) * 2f64.powf(p.a_octave), sr, modv)
                } else {
                    v.a.tick(f * 2f64.powf(p.a_semi / 12.0) * 2f64.powf(p.a_octave), sr)
                };
                l *= p.a_level as f32;
                r *= p.a_level as f32;
                if p.b_level > 0.0 {
                    let (bl, br) = v.b.tick(f * 2f64.powf(p.b_semi / 12.0) * 2f64.powf(p.b_octave), sr);
                    if p.ring > 0.0 {
                        // Ring modulation multiplies the two oscillators.
                        let ring = p.ring as f32;
                        l = l * (1.0 - ring) + l * bl * ring;
                        r = r * (1.0 - ring) + r * br * ring;
                    } else {
                        l += bl * p.b_level as f32;
                        r += br * p.b_level as f32;
                    }
                }
                if p.sub_level > 0.0 {
                    let s = v.sub.tick(f * 2f64.powf(p.sub_octave), sr) * p.sub_level as f32;
                    l += s;
                    r += s;
                }
                if p.noise_level > 0.0 {
                    let n = v.noise.tick() * p.noise_level as f32;
                    l += n;
                    r += n;
                }

                // Filter: env + LFO + key tracking, in octaves.
                let key_oct = (v.note - 60.0) / 12.0 * p.f_keytrack;
                let env_oct = fenv * p.f_env * (1.0 - p.vel_filter + p.vel_filter * v.velocity);
                let cut = p.f_cutoff * 2f64.powf(env_oct + key_oct + lfo_f * lfo_cut);
                v.filter.set_cutoff(cut);
                l = v.filter.tick(l);
                r = v.filter.tick(r);

                // Velocity → amp uses the square law: peak amplitude is the
                // square of the MIDI velocity in commercial instruments
                // (Dannenberg, ICMC 2006), which is what makes soft hits sit
                // back instead of just getting quieter.
                let vel_sq = v.velocity * v.velocity;
                let vel_gain = (1.0 - p.vel_amp + p.vel_amp * vel_sq) as f32;
                let trem = (1.0 - lfo_amp as f32 * 0.5 + lfo * lfo_amp as f32 * 0.5).clamp(0.0, 2.0);
                let g = amp as f32 * vel_gain * trem * p.level as f32 * 0.5;
                let pan = (v.pan as f64 * p.pan_spread + lfo_f * lfo_pan).clamp(-1.0, 1.0);
                let (pl, pr) = super::util::pan_gains(pan);
                sum_l += v.dc_l.process(l * g) * pl;
                sum_r += v.dc_r.process(r * g) * pr;
            }
            out_l[i] += sum_l;
            out_r[i] += sum_r;
        }
    }
}
