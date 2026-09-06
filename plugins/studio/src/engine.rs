//! The Studio render engine — the trem wrapper shared by the AI tools, the
//! REST routes, and (indirectly) the Studio window.
//!
//! A [`TrackConfig`] (single pattern) is validated, turned into a trem
//! [`Grid`] (rows = steps, columns = voices), wired through per-voice
//! instrument chains (with synth params) into a stereo mixer, then through a
//! master FX chain (delay → reverb → limiter), and rendered offline to planar
//! f32 buffers. An [`Arrangement`] places several patterns at time offsets and
//! mixes them into one stereo buffer.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use trem::dsp::{Gain, Limiter, PlateReverb, StereoDelay, StereoMixer};
use trem::event::{GraphEvent, TimedEvent};
use trem::graph::Graph;
use trem::math::Rational;
use trem::pitch::{Pitch, Scale, Tuning};
use trem::time::beat_to_sample;

use crate::fx;
use crate::voices::{self, build_instrument, ParamDef};
use crate::wav::encode_wav;

const SAMPLE_RATE: f64 = 44100.0;

/// A single melodic note override placed at a specific step.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NoteOverride {
    pub step: u32,
    /// Note length in steps (piano-roll bars); 1 = a single 16th-note step.
    #[serde(default = "default_note_length")]
    pub length: u32,
    pub degree: i32,
    pub octave: i32,
}

fn default_note_length() -> u32 {
    1
}

impl Default for NoteOverride {
    fn default() -> Self {
        Self { step: 0, length: 1, degree: 0, octave: 0 }
    }
}

/// One insert effect in a voice's device chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EffectConfig {
    #[serde(default = "default_fx_kind")]
    pub kind: String,
    pub params: HashMap<String, f64>,
    /// When true the effect is skipped during rendering.
    pub bypass: bool,
}

fn default_fx_kind() -> String {
    "distortion".into()
}

impl Default for EffectConfig {
    fn default() -> Self {
        Self { kind: "distortion".into(), params: HashMap::new(), bypass: false }
    }
}

/// One MIDI (note-processing) effect applied before synthesis.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MidiFxConfig {
    #[serde(default = "default_midi_fx_kind")]
    pub kind: String,
    pub params: HashMap<String, f64>,
}

fn default_midi_fx_kind() -> String {
    "transpose".into()
}

impl Default for MidiFxConfig {
    fn default() -> Self {
        Self { kind: "transpose".into(), params: HashMap::new() }
    }
}

/// One pad in a `drumkit` voice.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PadConfig {
    #[serde(default = "default_pad_name")]
    pub name: String,
    #[serde(default = "default_kind")]
    pub kind: String,
}

fn default_pad_name() -> String {
    "Pad".into()
}

impl Default for PadConfig {
    fn default() -> Self {
        Self { name: "Pad".into(), kind: "kick".into() }
    }
}

/// One destination mapped onto a macro knob.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MacroAssignment {
    /// Parameter path, e.g. `cutoff`, `level`, `pan`, `fx0.drive`.
    pub path: String,
    /// Bipolar depth in [-1, 1]: how much the macro sweeps the target.
    #[serde(default = "default_amount")]
    pub amount: f64,
    /// The target's value at macro centre (knob = 0.5).
    pub base: f64,
}

fn default_amount() -> f64 {
    1.0
}

impl Default for MacroAssignment {
    fn default() -> Self {
        Self { path: String::new(), amount: 1.0, base: 0.0 }
    }
}

/// A macro knob: one value plus the list of parameters it drives.
/// (Frontend convenience — the frontend bakes these into synth/fx values before
/// serializing, so the engine never applies them; they are persisted verbatim.)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MacroConfig {
    #[serde(default = "default_macro_value")]
    pub value: f64,
    #[serde(default)]
    pub entries: Vec<MacroAssignment>,
}

fn default_macro_value() -> f64 {
    0.5
}

impl Default for MacroConfig {
    fn default() -> Self {
        Self { value: 0.5, entries: Vec::new() }
    }
}

/// The default 16-pad (4×4) drum machine layout.
pub fn default_pads() -> Vec<PadConfig> {
    const K: &[&str] = &["kick", "snare", "clap", "hat", "tom", "perc", "kick", "snare", "hat", "clap", "tom", "perc", "kick", "hat", "perc", "tom"];
    const N: &[&str] = &["Kick 1", "Snare", "Clap", "Hat", "Tom 1", "Perc", "Kick 2", "Snare 2", "Hat 2", "Clap 2", "Tom 2", "Perc 2", "Kick 3", "Hat 3", "Perc 3", "Tom 3"];
    K.iter().zip(N).map(|(k, n)| PadConfig { name: n.to_string(), kind: k.to_string() }).collect()
}

/// One voice (track) in a pattern.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceConfig {
    /// Instrument kind: `kick`, `snare`, `hat`, `bass`, `pluck`, `lead`, `pad`, `sub`.
    #[serde(default = "default_kind")]
    pub kind: String,
    /// `"e<hits>,<rot>"` (Euclidean fill) or an explicit `"x..x"` string.
    #[serde(default = "default_rhythm")]
    pub rhythm: String,
    /// Default scale degree for this voice's notes.
    pub degree: i32,
    /// Default octave offset.
    pub octave: i32,
    /// Waveform for `bass` (sine/saw/square/triangle).
    pub wave: Option<String>,
    /// Per-step note overrides (degree + octave).
    pub notes: Vec<NoteOverride>,
    /// Mix level; `None` uses the per-kind default.
    pub level: Option<f32>,
    /// Stereo pan; `None` uses the per-kind default.
    pub pan: Option<f32>,
    /// Synth parameter overrides, keyed by the kind's param names.
    pub synth: HashMap<String, f64>,
    /// Insert effect chain (mono FX → level/pan → stereo FX).
    pub fx: Vec<EffectConfig>,
    /// Pad layout for a `drumkit` voice (16 pads; empty = default kit).
    pub pads: Vec<PadConfig>,
    /// Macro rack (8 knobs) — persisted for round-trip, applied on the frontend.
    pub macros: Vec<MacroConfig>,
    /// MIDI effects (transpose/velocity/gate/ratchet) applied to note events.
    pub midi: Vec<MidiFxConfig>,
    /// Grid patch (modular) for `kind = "grid"` voices.
    pub grid: Option<crate::grid::GridPatch>,
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            kind: "kick".into(),
            rhythm: "x...".into(),
            degree: 0,
            octave: 0,
            wave: None,
            notes: Vec::new(),
            level: None,
            pan: None,
            synth: HashMap::new(),
            fx: Vec::new(),
            pads: Vec::new(),
            macros: Vec::new(),
            midi: Vec::new(),
            grid: None,
        }
    }
}

/// Full pattern config — the single JSON contract shared by tools, routes and UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TrackConfig {
    #[serde(default = "default_title")]
    pub title: String,
    #[serde(default = "default_bpm")]
    pub bpm: f64,
    /// Number of 16th-note steps (rows) in the pattern.
    #[serde(default = "default_steps")]
    pub steps: u32,
    /// Tuning system: `edo12`, `edo19`, or `ji7`.
    #[serde(default = "default_tuning")]
    pub tuning: String,
    /// Reference frequency for pitch resolution.
    #[serde(default = "default_ref")]
    pub ref_hz: f64,
    pub voices: Vec<VoiceConfig>,
    /// Master FX parameters (delay/reverb), keyed by name.
    pub fx: HashMap<String, f64>,
}

impl Default for TrackConfig {
    fn default() -> Self {
        Self {
            title: "Untitled".into(),
            bpm: 120.0,
            steps: 16,
            tuning: "edo12".into(),
            ref_hz: 440.0,
            voices: default_kit(),
            fx: HashMap::new(),
        }
    }
}

fn default_title() -> String {
    "Untitled".into()
}
fn default_bpm() -> f64 {
    120.0
}
fn default_steps() -> u32 {
    16
}
fn default_tuning() -> String {
    "edo12".into()
}
fn default_ref() -> f64 {
    440.0
}
fn default_kind() -> String {
    "kick".into()
}
fn default_rhythm() -> String {
    "x...".into()
}

/// A starter four-on-the-floor kit (kick + hats + backbeat snare).
pub fn default_kit() -> Vec<VoiceConfig> {
    vec![
        VoiceConfig { kind: "kick".into(), rhythm: "e4,0".into(), ..VoiceConfig::default() },
        VoiceConfig { kind: "hat".into(), rhythm: "e8,2".into(), ..VoiceConfig::default() },
        VoiceConfig { kind: "snare".into(), rhythm: "e4,8".into(), ..VoiceConfig::default() },
    ]
}

/// Master FX parameter catalog + defaults.
pub fn fx_defaults() -> &'static [ParamDef] {
    use voices::def as d;
    static FX: std::sync::OnceLock<Vec<ParamDef>> = std::sync::OnceLock::new();
    FX.get_or_init(|| {
        vec![
            d("delay_mix", "Delay Mix", 0.0, 1.0, 0.01, 0.0),
            d("delay_time", "Delay Time", 1.0, 2000.0, 1.0, 250.0),
            d("feedback", "Feedback", 0.0, 0.95, 0.01, 0.4),
            d("reverb_mix", "Reverb Mix", 0.0, 1.0, 0.01, 0.0),
            d("reverb_size", "Reverb Size", 0.0, 1.0, 0.01, 0.5),
            d("reverb_damp", "Reverb Damp", 0.0, 1.0, 0.01, 0.5),
        ]
    })
}

fn fx_val(fx: &HashMap<String, f64>, key: &str, default: f64) -> f64 {
    fx.get(key).copied().unwrap_or(default).clamp(-100_000.0, 100_000.0)
}

/// Offline-render result: planar samples are discarded; the WAV is kept.
#[derive(Debug, Clone)]
pub struct Rendered {
    pub sample_rate: u32,
    pub channels: u32,
    pub duration_ms: u32,
    pub frames: usize,
    pub wav: Vec<u8>,
}

/// Planar render: channel-major sample buffers (before WAV encoding).
#[derive(Debug, Clone)]
pub struct PlanarRender {
    pub sample_rate: u32,
    pub duration_ms: u32,
    pub frames: usize,
    pub channels: Vec<Vec<f32>>,
}

/// Parse a raw JSON value into a validated [`TrackConfig`].
pub fn parse_config(value: &serde_json::Value) -> Result<TrackConfig, String> {
    let mut cfg: TrackConfig =
        serde_json::from_value(value.clone()).map_err(|e| format!("invalid config: {e}"))?;
    cfg.steps = cfg.steps.clamp(4, 64);
    cfg.bpm = cfg.bpm.clamp(40.0, 240.0);
    Ok(cfg)
}

/// Resolve a tuning name to a trem [`Scale`].
pub fn tuning_scale(name: &str) -> Option<Scale> {
    match name.trim() {
        "edo12" => Some(Tuning::edo12().to_scale()),
        "edo19" => Some(Tuning::Equal { divisions: 19, interval: Pitch::OCTAVE }.to_scale()),
        "ji7" => Some(
            Tuning::Just {
                ratios: vec![
                    Rational::new(1, 1),
                    Rational::new(9, 8),
                    Rational::new(5, 4),
                    Rational::new(4, 3),
                    Rational::new(3, 2),
                    Rational::new(5, 3),
                    Rational::new(15, 8),
                ],
            }
            .to_scale(),
        ),
        _ => None,
    }
}

/// Fill one grid column from a voice's rhythm string.
fn resolve_frequency(degree: i32, octave: i32, scale: &Scale, reference_hz: f64) -> f64 {
    let pitch = scale.resolve(degree);
    let octave_pitch = Pitch(pitch.0 + octave as f64);
    octave_pitch.to_hz(reference_hz)
}

/// Step indices that are "on" for a rhythm string (Euclidean or explicit).
fn rhythm_hits(rhythm: &str, steps: u32) -> Vec<u32> {
    let r = rhythm.trim();
    let mut hits = Vec::new();
    if let Some(rest) = r.strip_prefix('e') {
        let mut parts = rest.split(',');
        let h: u32 = parts.next().and_then(|s| s.trim().parse().ok()).unwrap_or(0).min(steps);
        let rot: u32 = parts.next().map(|s| s.trim().parse().unwrap_or(0)).unwrap_or(0);
        let pattern = trem::euclidean::rotate(&trem::euclidean::euclidean(h, steps), rot % steps.max(1));
        for (i, b) in pattern.iter().enumerate() {
            if *b {
                hits.push(i as u32);
            }
        }
    } else {
        for (i, c) in r.chars().filter(|c| *c == 'x' || *c == '.').take(steps as usize).enumerate() {
            if c == 'x' {
                hits.push(i as u32);
            }
        }
    }
    hits
}

fn drumkit_pads(v: &VoiceConfig) -> Vec<PadConfig> {
    let pads = if v.pads.is_empty() { default_pads() } else { v.pads.clone() };
    pads.into_iter().take(16).collect()
}

/// Apply a voice's MIDI effects to one note, returning zero or more
/// `(start_beat, end_beat, degree, octave, velocity)` events (ratchet splits).
fn apply_midi(midi: &[MidiFxConfig], start: u32, len: u32, degree: i32, octave: i32, velocity: f64, steps: u32) -> Vec<(f64, f64, i32, i32, f64)> {
    let mut deg = degree;
    let mut oct = octave;
    let mut vel = velocity;
    let mut gate = 1.0;
    let mut ratchet = 0u32;
    for fx in midi {
        match fx.kind.as_str() {
            "transpose" => { deg += fx.params.get("steps").copied().unwrap_or(0.0) as i32; }
            "velocity" => { vel *= fx.params.get("amount").copied().unwrap_or(1.0).clamp(0.0, 1.0); }
            "gate" => { gate = fx.params.get("amount").copied().unwrap_or(1.0).clamp(0.1, 2.0); }
            "ratchet" => { ratchet = fx.params.get("count").copied().unwrap_or(2.0).clamp(2.0, 8.0) as u32; }
            _ => {}
        }
    }
    let start_beat = start as f64 / 4.0;
    let dur = (len as f64 / 4.0 * gate).clamp(0.0625, steps as f64 / 4.0);
    let end_beat = (start_beat + dur).min(steps as f64 / 4.0);
    let mut out = Vec::new();
    if ratchet > 1 {
        let seg = dur / ratchet as f64;
        for k in 0..ratchet {
            let s = start_beat + k as f64 * seg;
            let e = (s + seg * 0.85).min(end_beat);
            out.push((s, e, deg, oct, vel));
        }
    } else {
        out.push((start_beat, end_beat, deg, oct, vel));
    }
    out
}

/// Render a single pattern to planar stereo buffers (one pass, no modulation).
fn render_pattern_once(cfg: &TrackConfig) -> Result<PlanarRender, String> {
    let steps = cfg.steps.clamp(4, 64);
    let bpm = cfg.bpm.clamp(40.0, 240.0);
    let voices: Vec<VoiceConfig> = if cfg.voices.is_empty() {
        default_kit()
    } else {
        cfg.voices.clone()
    };
    if voices.len() > 12 {
        return Err("too many voices (max 12)".into());
    }
    for v in &voices {
        if !voices::is_kind(&v.kind) {
            return Err(format!("unknown voice kind `{}` (one of {})", v.kind, voices::KINDS.join(", ")));
        }
        if v.fx.len() > 8 {
            return Err("too many effects on one voice (max 8)".into());
        }
        for e in &v.fx {
            if !fx::is_effect(&e.kind) {
                return Err(format!("unknown effect `{}` (one of {})", e.kind, fx::EFFECT_KINDS.join(", ")));
            }
        }
        if v.midi.len() > 8 {
            return Err("too many MIDI effects on one voice (max 8)".into());
        }
        for m in &v.midi {
            if !voices::is_midi_fx(&m.kind) {
                return Err(format!("unknown MIDI effect `{}` (one of {})", m.kind, voices::MIDI_FX_KINDS.join(", ")));
            }
        }
    }
    let scale = tuning_scale(&cfg.tuning)
        .ok_or_else(|| format!("unknown tuning `{}` (use edo12, edo19, ji7)", cfg.tuning))?;

    let total_pairs: usize = voices
        .iter()
        .map(|v| if v.kind == "drumkit" { drumkit_pads(v).len().max(1) } else { 1 })
        .sum();

    let mut graph = Graph::new(512);
    let mixer = graph.add_node(Box::new(StereoMixer::with_level(total_pairs as u16, 0.85)));
    let mut mixer_pair = 0u16;

    for (col, v) in voices.iter().enumerate() {
        if v.kind == "drumkit" {
            // A drumkit is a bank of independent pads (one drum chain each),
            // keyed by sub-voice id (col*16 + pad).
            let level = v.level.unwrap_or_else(|| voices::default_level("drumkit"));
            let pan = v.pan.unwrap_or_else(|| voices::default_pan("drumkit"));
            for (pi, pad) in drumkit_pads(v).iter().enumerate() {
                let sub_id = (col as u32) * 16 + pi as u32;
                let built = build_instrument(&mut graph, &pad.kind, sub_id, None, &HashMap::new());
                let gain = graph.add_node(Box::new(Gain::new(level)));
                if pan != 0.0 {
                    graph.set_node_param(gain, 1, pan as f64);
                }
                graph.connect(built.out, 0, gain, 0);
                graph.connect(gain, 0, mixer, (mixer_pair * 2) as u16);
                graph.connect(gain, 1, mixer, (mixer_pair * 2 + 1) as u16);
                mixer_pair += 1;
            }
            continue;
        }

        if v.kind == "grid" {
            let patch = v.grid.as_ref().ok_or_else(|| "grid voice missing its patch".to_string())?;
            let compiled = crate::grid::compile_grid(&mut graph, patch, col as u32)?;
            let mut mono_out = compiled.out;
            for effect in &v.fx {
                if effect.bypass || fx::is_stereo(&effect.kind) {
                    continue;
                }
                let n = fx::build_effect(&mut graph, &effect.kind, &effect.params)?;
                graph.connect(mono_out, 0, n, 0);
                mono_out = n;
            }
            let level = v.level.unwrap_or_else(|| voices::default_level("grid"));
            let pan = v.pan.unwrap_or_else(|| voices::default_pan("grid"));
            let gain = graph.add_node(Box::new(Gain::new(level)));
            if pan != 0.0 {
                graph.set_node_param(gain, 1, pan as f64);
            }
            graph.connect(mono_out, 0, gain, 0);
            let (mut left, mut right) = (gain, gain);
            for effect in &v.fx {
                if effect.bypass || !fx::is_stereo(&effect.kind) {
                    continue;
                }
                let n = fx::build_effect(&mut graph, &effect.kind, &effect.params)?;
                graph.connect(left, 0, n, 0);
                graph.connect(right, 1, n, 1);
                left = n;
                right = n;
            }
            graph.connect(left, 0, mixer, (mixer_pair * 2) as u16);
            graph.connect(right, 1, mixer, (mixer_pair * 2 + 1) as u16);
            mixer_pair += 1;
            continue;
        }

        let built = build_instrument(&mut graph, &v.kind, col as u32, v.wave.as_deref(), &v.synth);
        for (def, node, pid) in &built.params {
            if let Some(val) = v.synth.get(def.key) {
                graph.set_node_param(*node, *pid, val.clamp(def.min, def.max));
            }
        }
        // Insert effect chain: mono FX → level/pan gain → stereo FX.
        let mut mono_out = built.out;
        for effect in &v.fx {
            if effect.bypass || fx::is_stereo(&effect.kind) {
                continue;
            }
            let n = fx::build_effect(&mut graph, &effect.kind, &effect.params)?;
            graph.connect(mono_out, 0, n, 0);
            mono_out = n;
        }

        let level = v.level.unwrap_or_else(|| voices::default_level(&v.kind));
        let pan = v.pan.unwrap_or_else(|| voices::default_pan(&v.kind));
        let gain = graph.add_node(Box::new(Gain::new(level)));
        if pan != 0.0 {
            graph.set_node_param(gain, 1, pan as f64);
        }
        graph.connect(mono_out, 0, gain, 0);

        let (mut left, mut right) = (gain, gain);
        for effect in &v.fx {
            if effect.bypass || !fx::is_stereo(&effect.kind) {
                continue;
            }
            let n = fx::build_effect(&mut graph, &effect.kind, &effect.params)?;
            graph.connect(left, 0, n, 0);
            graph.connect(right, 1, n, 1);
            left = n;
            right = n;
        }
        graph.connect(left, 0, mixer, (mixer_pair * 2) as u16);
        graph.connect(right, 1, mixer, (mixer_pair * 2 + 1) as u16);
        mixer_pair += 1;
    }

    let delay = graph.add_node(Box::new(StereoDelay::new(
        fx_val(&cfg.fx, "delay_time", 250.0),
        fx_val(&cfg.fx, "feedback", 0.4),
        fx_val(&cfg.fx, "delay_mix", 0.0),
    )));
    let reverb = graph.add_node(Box::new(PlateReverb::new(
        fx_val(&cfg.fx, "reverb_size", 0.5),
        fx_val(&cfg.fx, "reverb_damp", 0.5),
        fx_val(&cfg.fx, "reverb_mix", 0.0),
    )));
    let limiter = graph.add_node(Box::new(Limiter::new(-0.3, 100.0)));
    graph.connect(mixer, 0, delay, 0);
    graph.connect(mixer, 1, delay, 1);
    graph.connect(delay, 0, reverb, 0);
    graph.connect(delay, 1, reverb, 1);
    graph.connect(reverb, 0, limiter, 0);
    graph.connect(reverb, 1, limiter, 1);

    // Build events per voice. Melodic voices with notes use note durations
    // (piano-roll bars); drums and melodic voices without notes use rhythm hits.
    let spb = 60.0 / bpm;
    let mut events: Vec<TimedEvent> = Vec::new();
    for (col, v) in voices.iter().enumerate() {
        let vid = col as u32;

        if v.kind == "drumkit" {
            // Drum machine: each note's degree selects a pad (sub-voice).
            let pads = drumkit_pads(v);
            for n in &v.notes {
                let pad = ((n.degree % 16) + 16) % 16;
                if pad as usize >= pads.len() {
                    continue;
                }
                let sub = (col as u32) * 16 + pad as u32;
                let start = n.step.min(steps - 1);
                let on = beat_to_sample(Rational::new(start as i64, 4), bpm, SAMPLE_RATE) as usize;
                let off = beat_to_sample(Rational::new((start + 1) as i64, 4), bpm, SAMPLE_RATE) as usize;
                events.push(TimedEvent { sample_offset: on, event: GraphEvent::NoteOn { frequency: 0.0, velocity: 0.8, voice: sub } });
                events.push(TimedEvent { sample_offset: off, event: GraphEvent::NoteOff { voice: sub } });
            }
            continue;
        }

        let melodic = matches!(v.kind.as_str(), "bass" | "pluck" | "lead" | "pad" | "sub" | "organ" | "ep" | "bell" | "strings" | "brass" | "synthme" | "grid");
        let mut notes: Vec<(u32, u32, i32, i32)> = Vec::new();
        if melodic && !v.notes.is_empty() {
            for n in &v.notes {
                notes.push((n.step.min(steps - 1), n.length.max(1), n.degree, n.octave));
            }
        } else {
            for step in rhythm_hits(&v.rhythm, steps) {
                notes.push((step, 1, v.degree, v.octave));
            }
        }
        for (start, len, degree, octave) in notes {
            for (s, e, d, o, vel) in apply_midi(&v.midi, start, len, degree, octave, 0.75, steps) {
                let freq = resolve_frequency(d, o, &scale, cfg.ref_hz);
                let on = (s * spb * SAMPLE_RATE).round() as usize;
                let off = (e * spb * SAMPLE_RATE).round() as usize;
                events.push(TimedEvent { sample_offset: on, event: GraphEvent::NoteOn { frequency: freq, velocity: vel, voice: vid } });
                events.push(TimedEvent { sample_offset: off, event: GraphEvent::NoteOff { voice: vid } });
            }
        }
    }
    events.sort_by_key(|e| e.sample_offset);

    let beats = Rational::new(steps as i64, 4);
    let duration_samples = (beats.to_f64() * 60.0 / bpm * SAMPLE_RATE).ceil() as usize;
    let audio = trem::render::render(&mut graph, &events, duration_samples, SAMPLE_RATE, limiter, &[0, 1]);

    let duration_ms = (duration_samples as f64 / SAMPLE_RATE * 1000.0).round() as u32;
    Ok(PlanarRender {
        sample_rate: SAMPLE_RATE as u32,
        duration_ms,
        frames: duration_samples,
        channels: audio,
    })
}

/// Note on/off times (seconds) for a voice — used for grid env modulation.
fn voice_note_times(v: &VoiceConfig, steps: u32, bpm: f64) -> Vec<(f64, f64)> {
    let spb = 60.0 / bpm;
    let mut out = Vec::new();
    let melodic = matches!(v.kind.as_str(), "bass" | "pluck" | "lead" | "pad" | "sub" | "organ" | "ep" | "bell" | "strings" | "brass" | "synthme" | "grid");
    let mut notes: Vec<(u32, u32, i32, i32)> = Vec::new();
    if melodic && !v.notes.is_empty() {
        for n in &v.notes {
            notes.push((n.step.min(steps - 1), n.length.max(1), n.degree, n.octave));
        }
    } else {
        for step in rhythm_hits(&v.rhythm, steps) {
            notes.push((step, 1, v.degree, v.octave));
        }
    }
    for (start, len, _, _) in notes {
        let on = start as f64 / 4.0 * spb;
        let off = (start + len).min(steps) as f64 / 4.0 * spb;
        out.push((on, off));
    }
    out.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    out
}

/// Render a pattern whose grid voices carry control-rate modulation
/// (per-step segment re-rendering with short crossfades).
fn render_planar_grid(cfg: &TrackConfig) -> Result<PlanarRender, String> {
    let steps = cfg.steps.clamp(4, 64) as usize;
    let bpm = cfg.bpm.clamp(40.0, 240.0);
    let spb = 60.0 / bpm;
    let step_secs = spb / 4.0;
    let step_samples = (step_secs * SAMPLE_RATE).round() as usize;
    let total = (steps as f64 * step_secs * SAMPLE_RATE).ceil() as usize;
    let xf = 256usize;

    // (voice index, grid modulations, note times)
    let mut grid_info: Vec<(usize, Vec<crate::grid::GridMod>, Vec<(f64, f64)>)> = Vec::new();
    for (vi, v) in cfg.voices.iter().enumerate() {
        if v.kind != "grid" {
            continue;
        }
        let Some(patch) = &v.grid else { continue };
        let mods = crate::grid::grid_modulations(patch);
        if mods.is_empty() {
            continue;
        }
        grid_info.push((vi, mods, voice_note_times(v, steps as u32, bpm)));
    }
    if grid_info.is_empty() {
        return render_pattern_once(cfg);
    }

    let mut out = vec![vec![0.0f32; total], vec![0.0f32; total]];
    let mut prev: Option<Vec<Vec<f32>>> = None;
    for k in 0..steps {
        let t = k as f64 * step_secs;
        let mut seg_cfg = cfg.clone();
        for (vi, mods, notes) in &grid_info {
            let Some(v) = seg_cfg.voices.get_mut(*vi) else { continue };
            let Some(patch) = v.grid.as_mut() else { continue };
            for m in mods {
                let val = crate::grid::source_value(&m.source, notes, t);
                let set = (m.base + val).clamp(m.lo, m.hi);
                crate::grid::apply_grid_mod(patch, &m.module_id, m.param_key, set);
            }
        }
        let p = render_pattern_once(&seg_cfg)?;
        let start = k * step_samples;
        let end = ((k + 1) * step_samples).min(total).min(p.frames);
        if end <= start {
            break;
        }
        let seg: Vec<Vec<f32>> = p.channels.iter().map(|c| c[start..end].to_vec()).collect();
        for ch in 0..2usize {
            let dst = &mut out[ch];
            let src = seg.get(ch).map(|s| s.as_slice()).unwrap_or(&[]);
            let seglen = (end - start).min(src.len());
            if let Some(pv) = &prev {
                let pv_ch = pv.get(ch).map(|s| s.as_slice()).unwrap_or(&[]);
                let overlap = xf.min(seglen).min(pv_ch.len());
                for i in 0..overlap {
                    let t2 = i as f32 / overlap as f32;
                    dst[start + i] = pv_ch[pv_ch.len() - overlap + i] * (1.0 - t2) + src[i] * t2;
                }
                for i in overlap..seglen {
                    dst[start + i] = src[i];
                }
            } else {
                for i in 0..seglen {
                    dst[start + i] = src[i];
                }
            }
        }
        prev = Some(seg);
    }
    let duration_ms = (total as f64 / SAMPLE_RATE * 1000.0).round() as u32;
    Ok(PlanarRender { sample_rate: SAMPLE_RATE as u32, duration_ms, frames: total, channels: out })
}

/// Render a pattern, dispatching to segment rendering when a grid voice has
/// control-rate modulation.
fn render_planar(cfg: &TrackConfig) -> Result<PlanarRender, String> {
    let has_grid_mod = cfg.voices.iter().any(|v| {
        v.kind == "grid" && v.grid.as_ref().map(|p| !crate::grid::grid_modulations(p).is_empty()).unwrap_or(false)
    });
    if has_grid_mod {
        render_planar_grid(cfg)
    } else {
        render_pattern_once(cfg)
    }
}

/// Resolve a device automation path onto a config and set its value.
fn apply_override(cfg: &mut TrackConfig, path: &str, value: f64) {
    if let Some(rest) = path.strip_prefix("voice.") {
        let mut parts = rest.splitn(2, '.');
        let Some(vi) = parts.next().and_then(|s| s.parse::<usize>().ok()) else { return };
        let Some(keypath) = parts.next() else { return };
        let Some(v) = cfg.voices.get_mut(vi) else { return };
        if keypath == "level" {
            v.level = Some(value as f32);
        } else if keypath == "pan" {
            v.pan = Some(value as f32);
        } else if let Some(fxpath) = keypath.strip_prefix("fx.") {
            let mut fp = fxpath.splitn(2, '.');
            let Some(fi) = fp.next().and_then(|s| s.parse::<usize>().ok()) else { return };
            let Some(key) = fp.next() else { return };
            if let Some(fx) = v.fx.get_mut(fi) {
                fx.params.insert(key.to_string(), value);
            }
        } else {
            v.synth.insert(keypath.to_string(), value);
        }
    } else if let Some(key) = path.strip_prefix("master.") {
        cfg.fx.insert(key.to_string(), value);
    }
}

/// Render a pattern with time-varying device parameters (per-beat segments,
/// short crossfade to mask oscillator phase resets). Level/pan stay post.
fn render_planar_automated(cfg: &TrackConfig, lanes: &[&AutomationLane]) -> Result<PlanarRender, String> {
    let steps = cfg.steps.clamp(4, 64);
    let bpm = cfg.bpm.clamp(40.0, 240.0);
    let nbeats = (steps / 4).max(1) as usize;
    let spb = 60.0 / bpm;
    let beat_samples = (spb * SAMPLE_RATE).round() as usize;
    let total_samples = (nbeats as f64 * spb * SAMPLE_RATE).ceil() as usize;
    let xf = 256usize; // crossfade samples

    let mut out = vec![vec![0.0f32; total_samples], vec![0.0f32; total_samples]];
    let mut prev: Option<Vec<Vec<f32>>> = None;

    for b in 0..nbeats {
        let mut seg_cfg = cfg.clone();
        for lane in lanes {
            if lane.points.is_empty() {
                continue;
            }
            let val = envelope_at(&lane.points, b as f64, 0.0);
            apply_override(&mut seg_cfg, &lane.param, val);
        }
        let p = render_planar(&seg_cfg)?;
        let start = b * beat_samples;
        let end = ((b + 1) * beat_samples).min(total_samples).min(p.frames);
        if end <= start {
            break;
        }
        let seg: Vec<Vec<f32>> = p.channels.iter().map(|c| c[start..end].to_vec()).collect();

        for ch in 0..2usize {
            let dst = &mut out[ch];
            let src = seg.get(ch).map(|s| s.as_slice()).unwrap_or(&[]);
            let seglen = (end - start).min(src.len());
            if let Some(pv) = &prev {
                let pv_ch = pv.get(ch).map(|s| s.as_slice()).unwrap_or(&[]);
                let overlap = xf.min(seglen).min(pv_ch.len());
                for i in 0..overlap {
                    let t = i as f32 / overlap as f32;
                    dst[start + i] = pv_ch[pv_ch.len() - overlap + i] * (1.0 - t) + src[i] * t;
                }
                for i in overlap..seglen {
                    dst[start + i] = src[i];
                }
            } else {
                for i in 0..seglen {
                    dst[start + i] = src[i];
                }
            }
        }
        prev = Some(seg);
    }

    let duration_ms = (total_samples as f64 / SAMPLE_RATE * 1000.0).round() as u32;
    Ok(PlanarRender { sample_rate: SAMPLE_RATE as u32, duration_ms, frames: total_samples, channels: out })
}

/// Compute a min/max peak envelope of a pattern's render (for waveform previews).
///
/// Returns one `(min, max)` pair per bucket, derived across both channels and
/// normalized by the pattern's own limiter/tanh soft clip.
pub fn waveform_peaks(cfg: &TrackConfig, buckets: usize) -> Result<Vec<(f32, f32)>, String> {
    let p = render_planar(cfg)?;
    let n = buckets.clamp(16, 512);
    let frames = p.frames;
    let per = (frames / n).max(1);
    let mut peaks = Vec::with_capacity(n);
    for b in 0..n {
        let start = b * per;
        let end = ((b + 1) * per).min(frames);
        let mut mn = f32::MAX;
        let mut mx = f32::MIN;
        for ch in &p.channels {
            for i in start..end {
                let s = ch[i];
                if s < mn {
                    mn = s;
                }
                if s > mx {
                    mx = s;
                }
            }
        }
        if mn == f32::MAX {
            mn = 0.0;
            mx = 0.0;
        }
        peaks.push((mn, mx));
    }
    Ok(peaks)
}

/// Validate and render a single pattern to WAV.
pub fn render_track(cfg: &TrackConfig) -> Result<Rendered, String> {
    let p = render_planar(cfg)?;
    let wav = encode_wav(&p.channels, p.sample_rate);
    Ok(Rendered {
        sample_rate: p.sample_rate,
        channels: 2,
        duration_ms: p.duration_ms,
        frames: p.frames,
        wav,
    })
}

// ─────────────────────────────────────────────────────────────
// Arrangement: several patterns mixed on a timeline.
// ─────────────────────────────────────────────────────────────

/// One arrangement lane (a horizontal row in the DAW).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ArrangementTrack {
    pub id: String,
    #[serde(default = "default_track_name")]
    pub name: String,
    pub color: u32,
    pub mute: bool,
    #[serde(default = "default_track_level")]
    pub level: f32,
    pub pan: f32,
    /// Per-track automation envelopes (time-varying level / pan).
    #[serde(default)]
    pub automation: TrackAutomation,
}

impl Default for ArrangementTrack {
    fn default() -> Self {
        Self { id: String::new(), name: "Track".into(), color: 0, mute: false, level: 0.8, pan: 0.0, automation: TrackAutomation::default() }
    }
}

/// One automation breakpoint: a value at a beat position.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationPoint {
    pub beat: f64,
    pub value: f64,
}

/// One automation lane: a parameter path plus its breakpoint envelope.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AutomationLane {
    /// Target path: `track.level` | `track.pan` (post, per-sample) or a
    /// device path — `voice.<i>.<synth>` | `voice.<i>.level` | `voice.<i>.pan`
    /// | `voice.<i>.fx.<j>.<key>` | `master.<key>` (rendered per-beat).
    #[serde(default)]
    pub param: String,
    #[serde(default)]
    pub points: Vec<AutomationPoint>,
}

/// Automation lanes for a track.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct TrackAutomation {
    #[serde(default)]
    pub lanes: Vec<AutomationLane>,
}

fn default_track_name() -> String {
    "Track".into()
}
fn default_track_level() -> f32 {
    0.8
}

/// A clip: a pattern placed on a track at a beat offset.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ArrangementClip {
    /// Which track (by id) this clip lives on.
    pub track: String,
    /// Start position in beats.
    pub start: f64,
    /// The pattern this clip plays.
    pub pattern: TrackConfig,
}

impl Default for ArrangementClip {
    fn default() -> Self {
        Self { track: String::new(), start: 0.0, pattern: TrackConfig::default() }
    }
}

/// A full arrangement to render.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Arrangement {
    pub title: String,
    pub bpm: f64,
    /// Total length in beats (4 beats = 1 bar).
    pub length_beats: f64,
    /// Master output gain applied to the final mix.
    #[serde(default = "default_master")]
    pub master: f32,
    pub tracks: Vec<ArrangementTrack>,
    pub clips: Vec<ArrangementClip>,
}

fn default_master() -> f32 {
    0.9
}

impl Default for Arrangement {
    fn default() -> Self {
        Self { title: "Untitled".into(), bpm: 120.0, length_beats: 16.0, master: 0.9, tracks: Vec::new(), clips: Vec::new() }
    }
}

/// Equal-power stereo pan gains from a pan in [-1, 1].
fn pan_gains(pan: f64) -> (f64, f64) {
    let p = pan.clamp(-1.0, 1.0);
    let a = (p + 1.0) * std::f64::consts::FRAC_PI_4;
    (a.cos(), a.sin())
}

/// Sample an automation envelope at a beat position (linear interpolation;
/// holds the end values outside the breakpoint span). `fallback` is returned
/// when the envelope is empty.
fn envelope_at(points: &[AutomationPoint], beat: f64, fallback: f64) -> f64 {
    if points.is_empty() {
        return fallback;
    }
    if beat <= points[0].beat {
        return points[0].value;
    }
    if let Some(last) = points.last() {
        if beat >= last.beat {
            return last.value;
        }
    }
    for w in points.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        if beat >= a.beat && beat <= b.beat {
            let span = b.beat - a.beat;
            if span <= 0.0 {
                return b.value;
            }
            let t = (beat - a.beat) / span;
            return a.value + (b.value - a.value) * t;
        }
    }
    fallback
}

/// Parse an arrangement from JSON.
pub fn parse_arrangement(value: &serde_json::Value) -> Result<Arrangement, String> {
    let mut a: Arrangement =
        serde_json::from_value(value.clone()).map_err(|e| format!("invalid arrangement: {e}"))?;
    a.bpm = a.bpm.clamp(40.0, 240.0);
    a.length_beats = a.length_beats.clamp(4.0, 256.0);
    Ok(a)
}

/// Render an arrangement by mixing each clip's pattern at its offset.
pub fn render_arrangement(a: &Arrangement) -> Result<Rendered, String> {
    if a.tracks.is_empty() {
        return Err("arrangement needs at least one track".into());
    }
    if a.clips.is_empty() {
        return Err("arrangement needs at least one clip".into());
    }
    if a.tracks.len() > 16 {
        return Err("too many tracks (max 16)".into());
    }
    if a.clips.len() > 256 {
        return Err("too many clips (max 256)".into());
    }

    let bpm = a.bpm.clamp(40.0, 240.0);
    let length_beats = a.length_beats.clamp(4.0, 256.0);
    let total = (length_beats * 60.0 / bpm * SAMPLE_RATE).ceil() as usize;
    let mut mix: Vec<Vec<f32>> = vec![vec![0.0f32; total], vec![0.0f32; total]];

    for clip in &a.clips {
        let track = a.tracks.iter().find(|t| t.id == clip.track);
        let Some(track) = track else {
            return Err(format!("clip references unknown track `{}`", clip.track));
        };
        if track.mute {
            continue;
        }
        let mut cfg = clip.pattern.clone();
        cfg.bpm = bpm; // align timing to the arrangement tempo

        // Split this track's lanes into post (level/pan, per-sample) and
        // device lanes (per-beat segment rendering of synth/FX/master params).
        let level_lane = track.automation.lanes.iter().find(|l| l.param == "track.level");
        let pan_lane = track.automation.lanes.iter().find(|l| l.param == "track.pan");
        let device_lanes: Vec<&AutomationLane> = track
            .automation
            .lanes
            .iter()
            .filter(|l| l.param != "track.level" && l.param != "track.pan")
            .collect();

        let p = if device_lanes.is_empty() {
            render_planar(&cfg)?
        } else {
            render_planar_automated(&cfg, &device_lanes)?
        };

        let offset = (clip.start * 60.0 / bpm * SAMPLE_RATE).round() as usize;
        if offset >= total {
            continue;
        }
        let frames = p.frames.min(total - offset);
        let samples_per_beat = SAMPLE_RATE * 60.0 / bpm;
        let base_level = track.level.clamp(0.0, 2.0) as f64;
        let base_pan = track.pan.clamp(-1.0, 1.0) as f64;
        let c0 = p.channels.get(0);
        let c1 = p.channels.get(1);
        for i in 0..frames {
            let beat = (offset + i) as f64 / samples_per_beat;
            let level = level_lane
                .map(|l| envelope_at(&l.points, beat, base_level))
                .unwrap_or(base_level)
                .clamp(0.0, 2.0);
            let pan = pan_lane
                .map(|l| envelope_at(&l.points, beat, base_pan))
                .unwrap_or(base_pan)
                .clamp(-1.0, 1.0);
            let (gl, gr) = pan_gains(pan);
            let l = (level * gl) as f32;
            let r = (level * gr) as f32;
            if let Some(c) = c0 {
                mix[0][offset + i] += c[i] * l;
            }
            if let Some(c) = c1 {
                mix[1][offset + i] += c[i] * r;
            }
        }
    }

    // Master gain + soft clip to keep the summed mix within range.
    let master = a.master.clamp(0.0, 2.0) as f32;
    for ch in 0..2 {
        for s in mix[ch].iter_mut() {
            *s = (*s * master).tanh().clamp(-0.99, 0.99);
        }
    }

    let wav = encode_wav(&mix, SAMPLE_RATE as u32);
    let duration_ms = (total as f64 / SAMPLE_RATE * 1000.0).round() as u32;
    Ok(Rendered {
        sample_rate: SAMPLE_RATE as u32,
        channels: 2,
        duration_ms,
        frames: total,
        wav,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn renders_a_kit() {
        let cfg = TrackConfig {
            bpm: 120.0,
            steps: 16,
            voices: vec![
                VoiceConfig { kind: "kick".into(), rhythm: "e4,0".into(), ..VoiceConfig::default() },
                VoiceConfig { kind: "hat".into(), rhythm: "e8,2".into(), ..VoiceConfig::default() },
                VoiceConfig { kind: "snare".into(), rhythm: "e4,8".into(), ..VoiceConfig::default() },
                VoiceConfig { kind: "bass".into(), rhythm: "x.x.x.x".into(), degree: 0, octave: 2, wave: Some("triangle".into()), ..VoiceConfig::default() },
            ],
            ..TrackConfig::default()
        };
        let out = render_track(&cfg).expect("render should succeed");
        assert_eq!(out.channels, 2);
        assert!(out.duration_ms > 500);
        assert_eq!(&out.wav[0..4], b"RIFF");
    }

    #[test]
    fn melodic_voice_renders_nonzero_audio() {
        let cfg = TrackConfig {
            steps: 8,
            voices: vec![VoiceConfig {
                kind: "lead".into(),
                rhythm: "x.x.x.x.".into(),
                degree: 4,
                octave: 3,
                ..VoiceConfig::default()
            }],
            ..TrackConfig::default()
        };
        let out = render_track(&cfg).unwrap();
        let energy: i64 = out.wav[44..]
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]) as i64)
            .map(|s| s * s)
            .sum();
        assert!(energy > 0);
    }

    #[test]
    fn new_preset_instruments_render_nonzero() {
        for kind in ["organ", "ep", "bell", "strings", "brass"] {
            let cfg = TrackConfig {
                steps: 8,
                voices: vec![VoiceConfig {
                    kind: kind.into(),
                    rhythm: "x.x.x.x.".into(),
                    degree: 0,
                    octave: 3,
                    ..VoiceConfig::default()
                }],
                ..TrackConfig::default()
            };
            let out = render_track(&cfg).unwrap();
            let energy: i64 = out.wav[44..]
                .chunks_exact(2)
                .map(|b| i16::from_le_bytes([b[0], b[1]]) as i64)
                .map(|s| s * s)
                .sum();
            assert!(energy > 0, "{kind} should render non-zero audio");
        }
    }

    #[test]
    fn grid_patch_renders_with_modulation() {
        use crate::grid::{GridCable, GridModule, GridPatch};
        let p = |k: &str| -> HashMap<String, f64> { HashMap::new() };
        let mut patch = GridPatch::default();
        patch.modules = vec![
            GridModule { id: "o".into(), kind: "osc".into(), params: p("o") },
            GridModule { id: "f".into(), kind: "filter".into(), params: p("f") },
            GridModule { id: "e".into(), kind: "env".into(), params: p("e") },
            GridModule { id: "l".into(), kind: "lfo".into(), params: [("rate".to_string(), 2.0), ("depth".to_string(), 0.5)].into_iter().collect() },
            GridModule { id: "out".into(), kind: "out".into(), params: p("out") },
        ];
        patch.cables = vec![
            GridCable { from: ("o".into(), "out".into()), to: ("f".into(), "in".into()) },
            GridCable { from: ("f".into(), "out".into()), to: ("e".into(), "in".into()) },
            GridCable { from: ("e".into(), "out".into()), to: ("out".into(), "in".into()) },
            GridCable { from: ("l".into(), "ctrl".into()), to: ("f".into(), "mod".into()) },
        ];
        let cfg = TrackConfig {
            steps: 8,
            voices: vec![VoiceConfig {
                kind: "grid".into(),
                rhythm: "x.x.x.x.".into(),
                degree: 0,
                octave: 3,
                grid: Some(patch),
                ..VoiceConfig::default()
            }],
            ..TrackConfig::default()
        };
        let out = render_track(&cfg).unwrap();
        let energy: i64 = out.wav[44..]
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]) as i64)
            .map(|s| s * s)
            .sum();
        assert!(energy > 0, "grid patch should render non-zero audio");
    }

    #[test]
    fn renders_an_arrangement() {
        let kit = TrackConfig {
            steps: 16,
            voices: vec![VoiceConfig { kind: "kick".into(), rhythm: "e4,0".into(), ..VoiceConfig::default() }],
            ..TrackConfig::default()
        };
        let bass = TrackConfig {
            steps: 16,
            voices: vec![VoiceConfig { kind: "bass".into(), rhythm: "x.x.x.x".into(), degree: 0, octave: 2, ..VoiceConfig::default() }],
            ..TrackConfig::default()
        };
        let a = Arrangement {
            bpm: 120.0,
            length_beats: 16.0,
            tracks: vec![
                ArrangementTrack { id: "t0".into(), name: "Drums".into(), color: 0, ..ArrangementTrack::default() },
                ArrangementTrack { id: "t1".into(), name: "Bass".into(), color: 3, ..ArrangementTrack::default() },
            ],
            clips: vec![
                ArrangementClip { track: "t0".into(), start: 0.0, pattern: kit },
                ArrangementClip { track: "t1".into(), start: 0.0, pattern: bass },
            ],
            ..Arrangement::default()
        };
        let out = render_arrangement(&a).expect("arrangement should render");
        assert_eq!(&out.wav[0..4], b"RIFF");
        assert!(out.duration_ms >= 3000); // 16 beats @ 120bpm = 8s
    }

    #[test]
    fn automation_modulates_track_level() {
        let kit = TrackConfig {
            steps: 16,
            voices: vec![VoiceConfig { kind: "kick".into(), rhythm: "e4,0".into(), ..VoiceConfig::default() }],
            ..TrackConfig::default()
        };
        let auto = TrackAutomation {
            lanes: vec![AutomationLane {
                param: "track.level".into(),
                points: vec![
                    AutomationPoint { beat: 0.0, value: 1.0 },
                    AutomationPoint { beat: 16.0, value: 0.0 },
                ],
            }],
            ..TrackAutomation::default()
        };
        let a = Arrangement {
            bpm: 120.0,
            length_beats: 16.0,
            tracks: vec![ArrangementTrack {
                id: "t0".into(),
                name: "Drums".into(),
                level: 0.8,
                automation: auto,
                ..ArrangementTrack::default()
            }],
            clips: vec![ArrangementClip { track: "t0".into(), start: 0.0, pattern: kit }],
            ..Arrangement::default()
        };
        let out = render_arrangement(&a).unwrap();
        let samples: Vec<i16> = out.wav[44..]
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect();
        let sr = 44100usize;
        let first: i64 = samples[..sr].iter().map(|s| i64::from(*s) * i64::from(*s)).sum();
        let last: i64 = samples[samples.len() - sr..].iter().map(|s| i64::from(*s) * i64::from(*s)).sum();
        assert!(first > last, "first second should be louder than last ({first} vs {last})");
    }

    #[test]
    fn device_automation_renders() {
        let mut synth = std::collections::HashMap::new();
        synth.insert("cutoff".to_string(), 500.0);
        let bass = TrackConfig {
            steps: 16,
            voices: vec![VoiceConfig {
                kind: "bass".into(),
                rhythm: "x.x.x.x".into(),
                degree: 0,
                octave: 2,
                wave: Some("triangle".into()),
                synth,
                ..VoiceConfig::default()
            }],
            ..TrackConfig::default()
        };
        let auto = TrackAutomation {
            lanes: vec![AutomationLane {
                param: "voice.0.cutoff".into(),
                points: vec![
                    AutomationPoint { beat: 0.0, value: 200.0 },
                    AutomationPoint { beat: 4.0, value: 8000.0 },
                ],
            }],
            ..TrackAutomation::default()
        };
        let a = Arrangement {
            bpm: 120.0,
            length_beats: 16.0,
            tracks: vec![ArrangementTrack {
                id: "t0".into(),
                name: "Bass".into(),
                automation: auto,
                ..ArrangementTrack::default()
            }],
            clips: vec![ArrangementClip { track: "t0".into(), start: 0.0, pattern: bass }],
            ..Arrangement::default()
        };
        let out = render_arrangement(&a).expect("device automation should render");
        let energy: i64 = out.wav[44..]
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]) as i64)
            .map(|s| s * s)
            .sum();
        assert!(energy > 0);
    }

    #[test]
    fn arrangement_rejects_bad_clip_track() {
        let a = Arrangement {
            tracks: vec![ArrangementTrack { id: "t0".into(), ..ArrangementTrack::default() }],
            clips: vec![ArrangementClip { track: "missing".into(), start: 0.0, pattern: TrackConfig::default() }],
            ..Arrangement::default()
        };
        assert!(render_arrangement(&a).is_err());
    }

    #[test]
    fn euclidean_hits_count() {
        assert_eq!(rhythm_hits("e5,0", 16).len(), 5);
        assert_eq!(rhythm_hits("x.x", 16), vec![0, 2]);
    }

    #[test]
    fn note_duration_renders() {
        let cfg = TrackConfig {
            steps: 16,
            voices: vec![VoiceConfig {
                kind: "lead".into(),
                rhythm: "".into(),
                notes: vec![NoteOverride { step: 0, length: 8, degree: 4, octave: 3 }],
                ..VoiceConfig::default()
            }],
            ..TrackConfig::default()
        };
        let out = render_track(&cfg).expect("note duration render");
        assert_eq!(&out.wav[0..4], b"RIFF");
    }

    #[test]
    fn rejects_unknown_tuning_and_kind() {
        let bad_tuning = TrackConfig { tuning: "nope".into(), ..TrackConfig::default() };
        assert!(render_track(&bad_tuning).is_err());
        let bad_kind = TrackConfig {
            voices: vec![VoiceConfig { kind: "flute".into(), rhythm: "x...".into(), ..VoiceConfig::default() }],
            ..TrackConfig::default()
        };
        assert!(render_track(&bad_kind).is_err());
    }

    #[test]
    fn parses_json_with_defaults() {
        let cfg = parse_config(&json!({ "title": "Beat", "bpm": 100, "steps": 8 })).unwrap();
        assert_eq!(cfg.title, "Beat");
        assert_eq!(cfg.bpm, 100.0);
        assert_eq!(cfg.steps, 8);
        assert_eq!(cfg.voices.len(), 3);
    }

    #[test]
    fn renders_effect_chain_and_new_synths() {
        let cfg = TrackConfig {
            steps: 8,
            voices: vec![
                VoiceConfig {
                    kind: "pad".into(),
                    rhythm: "x.x.x.x.".into(),
                    degree: 0,
                    octave: 3,
                    fx: vec![
                        EffectConfig { kind: "filter".into(), params: [("cutoff".to_string(), 800.0)].into_iter().collect(), ..EffectConfig::default() },
                        EffectConfig { kind: "delay".into(), params: [("mix".to_string(), 0.3)].into_iter().collect(), ..EffectConfig::default() },
                    ],
                    ..VoiceConfig::default()
                },
                VoiceConfig { kind: "sub".into(), rhythm: "x...".into(), degree: 0, octave: 2, ..VoiceConfig::default() },
            ],
            ..TrackConfig::default()
        };
        let out = render_track(&cfg).expect("fx chain + pad/sub should render");
        assert_eq!(&out.wav[0..4], b"RIFF");
    }

    #[test]
    fn rejects_unknown_effect() {
        let cfg = TrackConfig {
            steps: 8,
            voices: vec![VoiceConfig {
                kind: "bass".into(),
                rhythm: "x...".into(),
                fx: vec![EffectConfig { kind: "flanger".into(), params: HashMap::new(), ..EffectConfig::default() }],
                ..VoiceConfig::default()
            }],
            ..TrackConfig::default()
        };
        assert!(render_track(&cfg).is_err());
    }
}
