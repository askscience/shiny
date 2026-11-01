//! The melodic voice engine: a polyphonic subtractive/FM synth.
//!
//! One [`Synth`] instance backs every pitched instrument kind (`bass`, `lead`,
//! `pad`, `strings`, `brass`, `organ`, `ep`, `bell`, `sub`, `pluck`,
//! `synthme`, `fm`). The *kind* only chooses default parameter values; the
//! DSP path is shared, so any instrument can grow into any other.
//!
//! Voice architecture (per note):
//!
//! ```text
//!  FM mod (sine) ┐
//!  Osc A (unison stack) ─┐
//!  Osc B (unison, ring)  ─┼─► stereo Filter (env + LFO + keytrack) ─► Amp (ADSR) ─► pan ─► L/R
//!  Sub sine               ─┤
//!  Noise                  ─┘
//! ```
//!
//! Professional details that matter for the sound:
//! * the filter has independent left/right state (stereo unison stays clean);
//! * stolen voices fade out over ~3 ms before the new note starts (no clicks);
//! * mono instruments (`poly = 1`) play legato with glide when notes overlap;
//! * the base cutoff and oscillator levels are smoothed, so automation and
//!   macros sweep continuously instead of stepping at control rate;
//! * analog drift is a slow, per-voice random walk of a few cents, not a fixed
//!   offset, and unison stacks get a real default detune.

use std::collections::HashMap;

use super::env::Adsr;
use super::filter::{Filter, FilterKind};
use super::lfo::{Lfo, LfoShape};
use super::osc::{Osc, Unison, Wave};
use super::util::{cents_ratio, pan_gains, Rng, Smoother};

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
    /// The percussion pad index for one-shot instruments (samplers, kits).
    pub pad: usize,
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
    /// Total unison detune spread in cents.
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
            b_detune: 0.0,
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
            if v.is_finite() {
                return *v;
            }
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
        p.a_wave = Wave::from_index(g(map, &["o1w"], p.a_wave.index()));
        p.b_wave = Wave::from_index(g(map, &["o2w"], 3.0));
        p.a_level = g(map, &["a_level", "osc1_level"], p.a_level).clamp(0.0, 2.0);
        p.b_level = g(map, &["b_level", "osc2_level"], if map.contains_key("o2w") { 0.4 } else { p.b_level })
            .clamp(0.0, 2.0);
        p.a_octave = g(map, &["a_octave"], p.a_octave).clamp(-4.0, 4.0);
        p.b_octave = g(map, &["b_octave"], p.b_octave).clamp(-4.0, 4.0);
        p.a_semi = g(map, &["a_semi"], 0.0).clamp(-48.0, 48.0);
        p.b_semi = g(map, &["b_semi", "detune"], p.b_semi).clamp(-48.0, 48.0);
        p.a_pw = g(map, &["pw", "a_pw"], 0.5);
        p.b_pw = g(map, &["b_pw"], p.a_pw);
        p.a_unison = g(map, &["unison", "a_unison"], 1.0).round().clamp(1.0, 12.0) as usize;
        p.b_unison = g(map, &["b_unison"], 1.0).round().clamp(1.0, 12.0) as usize;
        p.a_spread = g(map, &["spread", "a_spread"], 0.5).clamp(0.0, 1.0);
        p.b_spread = g(map, &["b_spread"], p.a_spread).clamp(0.0, 1.0);
        // A unison stack without detune is just a static comb filter: give it
        // a musical default spread (~22 cents) unless the user set one.
        let uni_default = if p.a_unison > 1 { 22.0 } else { 0.0 };
        p.a_detune = g(map, &["uni_detune", "a_detune"], uni_default).clamp(0.0, 100.0);
        p.b_detune = g(map, &["b_detune"], if p.b_unison > 1 { p.a_detune.max(18.0) } else { 0.0 }).clamp(0.0, 100.0);
        p.ring = g(map, &["ring"], 0.0).clamp(0.0, 1.0);
        p.sub_level = g(map, &["sub", "sub_level"], 0.0).clamp(0.0, 1.5);
        p.sub_octave = g(map, &["sub_octave"], -1.0).clamp(-3.0, 0.0);
        p.noise_level = g(map, &["noise", "noise_level"], 0.0).clamp(0.0, 1.0);
        p.fm_ratio = g(map, &["fm_ratio"], 2.0).clamp(0.05, 32.0);
        p.fm_index = g(map, &["fm_index"], 0.0).clamp(0.0, 20.0);
        p.fm_decay = g(map, &["fm_decay"], 0.3).clamp(0.005, 10.0);
        p.fm_self = g(map, &["fm_self"], 0.0) > 0.5;
        p.f_type = FilterKind::from_index(g(map, &["ftype", "type"], 0.0));
        p.f_cutoff = g(map, &["cutoff"], 3000.0).clamp(20.0, 20000.0);
        p.f_res = g(map, &["res", "resonance"], 0.9).clamp(0.4, 24.0);
        p.f_drive = g(map, &["drive"], 1.0).clamp(0.05, 24.0);
        p.f_poles = g(map, &["poles"], 1.0).round().clamp(1.0, 2.0) as u8;
        p.f_env = g(map, &["fenv", "filter_env"], 0.0).clamp(-8.0, 8.0);
        p.f_keytrack = g(map, &["keytrack"], 0.35).clamp(-1.0, 2.0);
        p.f_lfo = g(map, &["lfo_depth", "lfo_cutoff"], 0.0).clamp(-8.0, 8.0);
        p.lfo_cutoff = p.f_lfo;
        p.attack = g(map, &["attack"], 0.005).clamp(0.0005, 20.0);
        p.decay = g(map, &["decay"], 0.2).clamp(0.001, 20.0);
        p.sustain = g(map, &["sustain"], 0.7).clamp(0.0, 1.0);
        p.release = g(map, &["release"], 0.2).clamp(0.002, 20.0);
        p.env_shape = g(map, &["env_shape", "curve"], 0.0).clamp(0.0, 1.0);
        p.f_attack = g(map, &["fattack"], p.attack).clamp(0.0005, 20.0);
        p.f_decay = g(map, &["fdecay"], p.decay).clamp(0.001, 20.0);
        p.f_sustain = g(map, &["fsustain"], p.sustain).clamp(0.0, 1.0);
        p.f_release = g(map, &["frelease"], p.release).clamp(0.002, 20.0);
        p.lfo_rate = g(map, &["lfo_rate"], 3.0).clamp(0.01, 60.0);
        p.lfo_wave = LfoShape::from_index(g(map, &["lfo_wave"], 0.0));
        p.lfo_pitch = g(map, &["lfo_pitch"], 0.0).clamp(-24.0, 24.0);
        p.lfo_amp = g(map, &["lfo_amp"], 0.0).clamp(0.0, 1.0);
        p.lfo_pan = g(map, &["lfo_pan"], 0.0).clamp(0.0, 1.0);
        p.glide = g(map, &["glide", "portamento"], 0.0).clamp(0.0, 2.0);
        p.vel_amp = g(map, &["vel_amp"], 0.7).clamp(0.0, 1.0);
        p.vel_filter = g(map, &["vel_filter"], 0.4).clamp(0.0, 1.0);
        p.poly = g(map, &["poly"], 10.0).round().clamp(1.0, 24.0) as usize;
        p.pan_spread = g(map, &["pan_spread"], 0.0).clamp(0.0, 1.0);
        p.analog = g(map, &["analog", "drift"], 0.15).clamp(0.0, 1.0);
        p.level = g(map, &["level"], 1.0).clamp(0.0, 2.0);
        p
    }
}

/// Stolen voices fade out over this long before the new note starts.
const STEAL_FADE_S: f64 = 0.003;

/// One polyphonic voice.
struct SynthVoice {
    a: Unison,
    b: Unison,
    sub: Osc,
    noise: Rng,
    fm: Osc,
    fm_last: f64,
    filter: Filter,
    amp: Adsr,
    fenv: Adsr,
    fm_env: Adsr,
    freq: f64,
    target: f64,
    note: f64,
    velocity: f64,
    state: VoiceState,
    id: u64,
    order: u64,
    /// Random per-voice pan position in [-1, 1] (scaled by `pan_spread`).
    pan: f64,
    /// Slow random pitch drift (cents) and its target.
    drift: f64,
    drift_target: f64,
    drift_rng: Rng,
    drift_count: u32,
    /// Per-voice cutoff offset in octaves (analog component tolerance).
    cut_offset: f64,
    /// Steal fade: when `fade_step > 0` the voice is ramping down to start
    /// `pending`.
    fade: f64,
    fade_step: f64,
    pending: Option<(u64, f64, f64)>,
    seed: u64,
    /// Control-rate cache: pitch with drift/vibrato applied.
    cur_freq: f64,
}

impl SynthVoice {
    fn new(seed: u64, params: &SynthParams, sr: f64) -> Self {
        let mut rng = Rng::new(seed);
        let pan = rng.bipolar() as f64;
        let cut_offset = rng.bipolar() as f64 * 0.08;
        let mut v = Self {
            a: Unison::new(params.a_unison, params.a_wave, params.a_detune, params.a_spread, seed ^ 0xA1),
            b: Unison::new(params.b_unison, params.b_wave, params.b_detune, params.b_spread, seed ^ 0xB2),
            sub: Osc::new(Wave::Sine).with_seed(seed ^ 0x5B),
            noise: Rng::new(seed ^ 0x0F),
            fm: Osc::new(Wave::Sine).with_seed(seed ^ 0xF0),
            fm_last: 0.0,
            filter: Filter::new(params.f_type, params.f_cutoff, params.f_res),
            amp: Adsr::new(params.attack, params.decay, params.sustain, params.release),
            fenv: Adsr::new(params.f_attack, params.f_decay, params.f_sustain, params.f_release),
            fm_env: Adsr::new(0.001, params.fm_decay, 0.0, 0.1),
            freq: 440.0,
            target: 440.0,
            note: 69.0,
            velocity: 1.0,
            state: VoiceState::Idle,
            id: 0,
            order: 0,
            pan,
            drift: 0.0,
            drift_target: 0.0,
            drift_rng: Rng::new(seed ^ 0xD71F),
            drift_count: 0,
            cut_offset,
            fade: 1.0,
            fade_step: 0.0,
            pending: None,
            seed,
            cur_freq: 440.0,
        };
        v.set_sample_rate(sr);
        v.configure(params);
        v
    }

    fn set_sample_rate(&mut self, sr: f64) {
        self.filter.set_sample_rate(sr);
        self.amp.set_sample_rate(sr);
        self.fenv.set_sample_rate(sr);
        self.fm_env.set_sample_rate(sr);
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
        self.fm_env.set_times(0.001, p.fm_decay, 0.0, p.fm_decay.min(0.5));
        if self.a.count() != p.a_unison {
            self.a = Unison::new(p.a_unison, p.a_wave, p.a_detune, p.a_spread, self.seed ^ 0xA1);
        }
        if self.b.count() != p.b_unison {
            self.b = Unison::new(p.b_unison, p.b_wave, p.b_detune, p.b_spread, self.seed ^ 0xB2);
        }
        self.a.configure(p.a_detune, p.a_spread);
        self.b.configure(p.b_detune, p.b_spread);
        self.a.set_wave(p.a_wave);
        self.b.set_wave(p.b_wave);
        self.a.set_pulse_width(p.a_pw);
        self.b.set_pulse_width(p.b_pw);
    }

    /// Begin a note from silence (fresh oscillator phases and filter state).
    fn start(&mut self, id: u64, freq: f64, velocity: f64, order: u64) {
        self.id = id;
        self.order = order;
        self.freq = freq;
        self.target = freq;
        self.cur_freq = 0.0;
        self.note = 69.0 + 12.0 * (freq.max(1e-6) / 440.0).log2();
        self.velocity = velocity.clamp(0.05, 1.6);
        self.a.reset();
        self.b.reset();
        self.sub.set_phase(0.0);
        self.fm.set_phase(0.0);
        self.fm_last = 0.0;
        self.filter.reset();
        self.amp.reset();
        self.fenv.reset();
        self.fm_env.reset();
        self.amp.gate_on();
        self.fenv.gate_on();
        self.fm_env.gate_on();
        self.state = VoiceState::Active;
        self.fade = 1.0;
        self.fade_step = 0.0;
    }
}

/// The instrument: a voice pool plus a global LFO and smoothed controls.
pub struct Synth {
    params: SynthParams,
    map: HashMap<String, f64>,
    voices: Vec<SynthVoice>,
    lfo: Lfo,
    sr: f64,
    order: u64,
    base_seed: u64,
    /// Smoothed base cutoff in log2(Hz), so sweeps are exponential.
    cut_sm: Smoother,
    a_level_sm: Smoother,
    b_level_sm: Smoother,
    level_sm: Smoother,
    pub last_lfo: f32,
}

impl Synth {
    pub fn new(kind: &str, wave: Option<&str>, map: &HashMap<String, f64>, seed: u64, sr: f64) -> Self {
        let mut params = SynthParams::from_map(map, wave, kind);
        let default_poly = match kind {
            "bass" | "sub" => 1,
            "pad" | "strings" | "brass" => 12,
            _ => 10,
        };
        if !map.contains_key("poly") {
            params.poly = default_poly;
        }
        let voices = (0..params.poly)
            .map(|i| SynthVoice::new(seed.wrapping_add(i as u64 * 2_654_435_761), &params, sr))
            .collect();
        let mut synth = Self {
            lfo: Lfo::new(params.lfo_wave, seed ^ 0x10F0),
            cut_sm: Smoother::with_sr(params.f_cutoff.log2(), 0.004, sr),
            a_level_sm: Smoother::with_sr(params.a_level, 0.004, sr),
            b_level_sm: Smoother::with_sr(params.b_level, 0.004, sr),
            level_sm: Smoother::with_sr(params.level, 0.004, sr),
            params,
            map: map.clone(),
            voices,
            sr,
            order: 0,
            base_seed: seed,
            last_lfo: 0.0,
        };
        let mut m = map.clone();
        if !m.contains_key("poly") {
            m.insert("poly".into(), default_poly as f64);
        }
        synth.apply_map(&m, wave, kind);
        synth
    }

    /// Re-apply the parameter map (called whenever automation or the UI
    /// changes a synth parameter). Held notes keep sounding.
    pub fn apply_map(&mut self, map: &HashMap<String, f64>, wave: Option<&str>, kind: &str) {
        let params = SynthParams::from_map(map, wave, kind);
        let poly = params.poly;
        if poly > self.voices.len() {
            for i in self.voices.len()..poly {
                self.voices
                    .push(SynthVoice::new(self.base_seed.wrapping_add(i as u64 * 2_654_435_761), &params, self.sr));
            }
        } else if poly < self.voices.len() {
            // Drop idle voices first, then the oldest releasing ones.
            while self.voices.len() > poly {
                let idx = self
                    .voices
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, v)| match v.state {
                        VoiceState::Idle => (0, 0),
                        VoiceState::Release => (1, v.order),
                        VoiceState::Active => (2, v.order),
                    })
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                self.voices.remove(idx);
            }
        }
        for v in self.voices.iter_mut() {
            v.configure(&params);
        }
        if params.lfo_wave != self.params.lfo_wave {
            self.lfo.shape = params.lfo_wave;
        }
        self.cut_sm.aim(params.f_cutoff.log2());
        self.a_level_sm.aim(params.a_level);
        self.b_level_sm.aim(params.b_level);
        self.level_sm.aim(params.level);
        self.params = params;
        self.map = map.clone();
    }

    pub fn params(&self) -> &SynthParams {
        &self.params
    }

    /// Number of voices currently producing sound.
    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.state != VoiceState::Idle || v.pending.is_some()).count()
    }

    pub fn map(&self) -> &HashMap<String, f64> {
        &self.map
    }

    pub fn note_on(&mut self, id: u64, freq: f64, velocity: f64) {
        self.order += 1;
        let order = self.order;
        let glide = self.params.glide > 0.0;

        // Mono: legato glide into the held/releasing voice.
        if self.voices.len() == 1 {
            let v = &mut self.voices[0];
            if v.state == VoiceState::Idle && v.pending.is_none() {
                v.start(id, freq, velocity, order);
            } else if glide || v.state == VoiceState::Active {
                // Legato: keep phase and filter state, retarget the pitch and
                // re-open the envelopes from where they are (no click).
                v.id = id;
                v.order = order;
                v.target = freq;
                if !glide {
                    v.freq = freq;
                }
                v.note = 69.0 + 12.0 * (freq.max(1e-6) / 440.0).log2();
                v.velocity = velocity.clamp(0.05, 1.6);
                if v.state == VoiceState::Release || !glide {
                    v.amp.gate_on();
                    v.fenv.gate_on();
                    v.fm_env.gate_on();
                }
                v.state = VoiceState::Active;
            } else {
                v.pending = Some((id, freq, velocity));
                v.fade_step = 1.0 / (STEAL_FADE_S * self.sr).max(1.0);
            }
            return;
        }

        // Poly: an idle voice, else steal (releasing before active, oldest
        // first) with a short fade.
        let mut best: Option<(usize, (u8, u64))> = None;
        for (i, v) in self.voices.iter().enumerate() {
            let rank = if v.pending.is_some() {
                (3, v.order)
            } else {
                match v.state {
                    VoiceState::Idle => (0, 0),
                    VoiceState::Release => (1, v.order),
                    VoiceState::Active => (2, v.order),
                }
            };
            if best.map(|(_, b)| rank < b).unwrap_or(true) {
                best = Some((i, rank));
            }
        }
        let Some((i, _)) = best else { return };
        let v = &mut self.voices[i];
        if v.state == VoiceState::Idle && v.pending.is_none() {
            v.start(id, freq, velocity, order);
        } else {
            v.pending = Some((id, freq, velocity));
            v.order = order;
            v.fade_step = 1.0 / (STEAL_FADE_S * self.sr).max(1.0);
        }
    }

    pub fn note_off(&mut self, id: u64) {
        for v in self.voices.iter_mut() {
            if let Some((pid, _, _)) = v.pending {
                if pid == id {
                    // The note ended before the steal fade finished.
                    v.pending = None;
                    v.state = VoiceState::Release;
                    v.amp.gate_off();
                    continue;
                }
            }
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
            v.pending = None;
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
            v.pending = None;
            v.state = VoiceState::Idle;
        }
        self.lfo.reset();
    }

    /// Re-derive every sample-rate-dependent coefficient.
    pub fn set_sample_rate(&mut self, sr: f64) {
        if (sr - self.sr).abs() < 1e-9 {
            return;
        }
        self.sr = sr;
        for v in self.voices.iter_mut() {
            v.set_sample_rate(sr);
        }
        self.cut_sm = Smoother::with_sr(self.params.f_cutoff.log2(), 0.004, sr);
        self.a_level_sm = Smoother::with_sr(self.params.a_level, 0.004, sr);
        self.b_level_sm = Smoother::with_sr(self.params.b_level, 0.004, sr);
        self.level_sm = Smoother::with_sr(self.params.level, 0.004, sr);
    }

    /// Render `frames` samples (added into the stereo buses), applying the
    /// messages whose offsets fall inside this block.
    pub fn render(&mut self, out_l: &mut [f32], out_r: &mut [f32], msgs: &[NoteMsg], frames: usize, sr: f64) {
        self.set_sample_rate(sr);
        let p = self.params.clone();
        let mut mi = 0usize;
        let glide_coef = if p.glide > 0.0 { 1.0 - (-1.0 / (p.glide * 0.25 * sr)).exp() } else { 1.0 };
        let a_ratio = 2f64.powf(p.a_semi / 12.0 + p.a_octave);
        let b_ratio = 2f64.powf(p.b_semi / 12.0 + p.b_octave);
        let sub_ratio = 2f64.powf(p.sub_octave);
        // Drift: a new random target every ~80 ms, glided toward.
        let drift_period = (0.08 * sr) as u32;
        let drift_coef = 1.0 - (-1.0 / (0.12 * sr)).exp();
        let drift_cents = 6.0 * p.analog;
        let nyq = sr * 0.49;

        for i in 0..frames {
            while mi < msgs.len() && msgs[mi].at <= i {
                match msgs[mi].kind {
                    NoteKind::On { freq, velocity } => self.note_on(msgs[mi].id, freq, velocity),
                    NoteKind::Off => self.note_off(msgs[mi].id),
                }
                mi += 1;
            }
            let lfo = self.lfo.tick(p.lfo_rate, sr) as f64;
            self.last_lfo = lfo as f32;
            let base_cut = self.cut_sm.next();
            let a_level = self.a_level_sm.next() as f32;
            let b_level = self.b_level_sm.next() as f32;
            let level = self.level_sm.next();
            let mut sum_l = 0.0f32;
            let mut sum_r = 0.0f32;
            for v in self.voices.iter_mut() {
                if v.state == VoiceState::Idle && v.pending.is_none() {
                    continue;
                }
                // Steal fade → start the pending note at silence.
                if v.fade_step > 0.0 {
                    v.fade -= v.fade_step;
                    if v.fade <= 0.0 {
                        if let Some((id, f, vel)) = v.pending.take() {
                            let order = v.order;
                            v.start(id, f, vel, order);
                        } else {
                            v.fade = 1.0;
                            v.fade_step = 0.0;
                            v.state = VoiceState::Idle;
                            continue;
                        }
                    }
                }

                v.freq += (v.target - v.freq) * glide_coef;

                let amp = v.amp.tick();
                let fenv = v.fenv.tick();
                let fm_env = v.fm_env.tick();
                if !v.amp.is_active() {
                    v.state = VoiceState::Idle;
                    continue;
                }

                // Analog drift: slow random walk of a few cents.
                if drift_cents > 0.0 {
                    v.drift_count += 1;
                    if v.drift_count >= drift_period {
                        v.drift_count = 0;
                        v.drift_target = v.drift_rng.bipolar() as f64 * drift_cents;
                    }
                    v.drift += (v.drift_target - v.drift) * drift_coef;
                }
                // Pitch and cutoff modulation run at control rate (every
                // 8 samples ≈ 0.18 ms): far finer than any envelope or LFO.
                let control = i % 8 == 0 || v.cur_freq <= 0.0;
                if control {
                    v.cur_freq = v.freq * cents_ratio(v.drift + lfo * p.lfo_pitch * 100.0);
                }
                let f = v.cur_freq;

                // Oscillators.
                let fa = f * a_ratio;
                let (mut l, mut r) = if p.fm_index > 0.0 {
                    let fb = if p.fm_self { v.fm_last * 0.35 } else { 0.0 };
                    let m = v.fm.tick_pm(f * p.fm_ratio, sr, fb) as f64;
                    v.fm_last = m;
                    // Index is the peak phase deviation in radians.
                    let pm = m * p.fm_index * fm_env / std::f64::consts::TAU;
                    v.a.tick_pm(fa, sr, pm)
                } else {
                    v.a.tick(fa, sr)
                };
                l *= a_level;
                r *= a_level;
                if b_level > 0.0 || p.ring > 0.0 {
                    let (bl, br) = v.b.tick(f * b_ratio, sr);
                    if p.ring > 0.0 {
                        let ring = p.ring as f32;
                        l = l * (1.0 - ring) + l * bl * ring * 1.5;
                        r = r * (1.0 - ring) + r * br * ring * 1.5;
                    }
                    l += bl * b_level;
                    r += br * b_level;
                }
                if p.sub_level > 0.0 {
                    let s = v.sub.tick(f * sub_ratio, sr) * p.sub_level as f32;
                    l += s;
                    r += s;
                }
                if p.noise_level > 0.0 {
                    let n = v.noise.bipolar() * p.noise_level as f32 * 0.5;
                    l += n;
                    r += n;
                }

                // Filter: base (smoothed, log2) + env + LFO + key tracking.
                let key_oct = (v.note - 60.0) / 12.0 * p.f_keytrack;
                let vel_f = 1.0 - p.vel_filter + p.vel_filter * v.velocity.min(1.0);
                let env_oct = fenv * p.f_env * vel_f;
                if control || p.f_attack < 0.01 {
                    let oct = base_cut + env_oct + key_oct + lfo * p.lfo_cutoff + v.cut_offset * p.analog;
                    v.filter.set_cutoff(oct.exp2().clamp(16.0, nyq));
                }
                let (fl, fr) = v.filter.tick_stereo(l, r);

                // Velocity → amp (square law, as in commercial instruments).
                let vel = v.velocity.min(1.25);
                let vel_gain = 1.0 - p.vel_amp + p.vel_amp * vel * vel;
                let trem = (1.0 - p.lfo_amp * 0.5 + lfo * p.lfo_amp * 0.5).max(0.0);
                let gain = (amp * vel_gain * trem * level * 0.5 * v.fade) as f32;
                let pan = (v.pan * p.pan_spread + lfo * p.lfo_pan).clamp(-1.0, 1.0);
                let (pl, pr) = if pan == 0.0 { (1.0, 1.0) } else { pan_gains(pan) };
                sum_l += fl * gain * pl;
                sum_r += fr * gain * pr;
            }
            out_l[i] += sum_l;
            out_r[i] += sum_r;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(kind: &str, map: HashMap<String, f64>, notes: &[(usize, usize, f64)], frames: usize) -> (Vec<f32>, Vec<f32>) {
        let sr = 44_100.0;
        let mut s = Synth::new(kind, None, &map, 7, sr);
        let mut l = vec![0.0f32; frames];
        let mut r = vec![0.0f32; frames];
        let block = 64;
        let mut start = 0;
        let mut id = 0u64;
        let mut evs: Vec<(usize, NoteMsg)> = Vec::new();
        for (on, off, f) in notes {
            id += 1;
            evs.push((*on, NoteMsg { at: 0, id, pad: 0, kind: NoteKind::On { freq: *f, velocity: 0.9 } }));
            evs.push((*off, NoteMsg { at: 0, id, pad: 0, kind: NoteKind::Off }));
        }
        evs.sort_by_key(|e| e.0);
        while start < frames {
            let n = block.min(frames - start);
            let msgs: Vec<NoteMsg> = evs
                .iter()
                .filter(|(at, _)| *at >= start && *at < start + n)
                .map(|(at, m)| NoteMsg { at: at - start, ..*m })
                .collect();
            s.render(&mut l[start..start + n], &mut r[start..start + n], &msgs, n, sr);
            start += n;
        }
        (l, r)
    }

    #[test]
    fn silence_without_notes_is_exact_zero() {
        let (l, r) = render("pad", HashMap::new(), &[], 44_100);
        assert!(l.iter().chain(r.iter()).all(|x| *x == 0.0));
    }

    #[test]
    fn voices_finish_and_return_to_exact_silence() {
        let (l, _) = render("lead", HashMap::new(), &[(0, 4410, 440.0)], 44_100 * 2);
        assert!(l[..4410].iter().any(|x| x.abs() > 0.05));
        assert!(l[44_100..].iter().all(|x| *x == 0.0), "tail never reached silence");
    }

    #[test]
    fn chord_stays_in_range_and_stereo_is_clean() {
        let mut m = HashMap::new();
        m.insert("unison".into(), 5.0);
        m.insert("spread".into(), 1.0);
        let notes = [(0, 30_000, 220.0), (0, 30_000, 277.18), (0, 30_000, 329.63)];
        let (l, r) = render("pad", m, &notes, 44_100);
        let peak = l.iter().chain(r.iter()).fold(0.0f32, |a, x| a.max(x.abs()));
        assert!(peak > 0.05 && peak < 1.5, "peak {peak}");
        // Sample-to-sample differences of a 1.2 kHz-filtered pad are small;
        // the old shared-state filter produced Nyquist-rate jumps.
        let max_jump = l.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
        assert!(max_jump < 0.2, "jumpy output: {max_jump}");
    }

    #[test]
    fn stealing_does_not_click() {
        let mut m = HashMap::new();
        m.insert("poly".into(), 1.0);
        m.insert("sustain".into(), 1.0);
        m.insert("o1w".into(), 0.0);
        m.insert("cutoff".into(), 20000.0);
        let notes = [(0, 20_000, 220.0), (10_000, 30_000, 330.0)];
        let (l, _) = render("lead", m, &notes, 32_000);
        let max_jump = l.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
        assert!(max_jump < 0.1, "click on retrigger: {max_jump}");
    }
}
