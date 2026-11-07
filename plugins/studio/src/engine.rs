//! The Studio engine: the JSON contract (patterns, arrangements), note
//! scheduling, and the per-voice instrument channels.
//!
//! A [`TrackConfig`] (one pattern) becomes a set of channels — one per voice,
//! each a full instrument plus its insert chain — rendered block by block with
//! sample-accurate notes. Device automation and Grid modulation are applied to
//! live processors at control rate. Patterns repeat to fill their clip.
//!
//! Mixing (tracks, sends, returns, master, loudness) lives in
//! [`crate::mix`]; audio clips in [`crate::audio`].
//!
//! The JSON contract stays backward compatible; new fields are additive.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::dsp::drums::Drum;
use crate::dsp::fx::{Effect, EffectKind};
use crate::dsp::loudness;
use crate::dsp::sampler::Sampler;
use crate::dsp::synth::{NoteKind, NoteMsg, Synth};
use crate::dsp::util::Smoother;
use crate::dsp::SR;
use crate::grid::{GridEngine, GridPatch};
use crate::voices;
use crate::wav::encode_wav_bits;

/// Rendered sample rate (CD rate — the WAV contract every existing row uses).
pub const SAMPLE_RATE: f64 = SR;

/// Output sample rate from a config's `fx.sample_rate`.
///
/// The whole DSP layer is sample-rate-aware (oscillators take the rate per
/// tick; envelopes, filters, drums, effects and the master chain derive their
/// coefficients from the rate they are built with), so any rate in a sane
/// range renders correctly. Default is 44.1 kHz.
fn output_sr(fx: &HashMap<String, f64>) -> f64 {
    match fx.get("sample_rate").copied() {
        Some(v) if (8_000.0..=192_000.0).contains(&v) => v.round(),
        _ => SAMPLE_RATE,
    }
}

/// Output bit depth from a config's `fx.wav_bits` (16 default, 24 opt-in).
fn output_bits(fx: &HashMap<String, f64>) -> u16 {
    match fx.get("wav_bits").copied().map(|v| v.round() as i64) {
        Some(24) => 24,
        _ => 16,
    }
}

/* ═══════════════════════════════════════════════════════════════
   Config — the JSON contract shared by tools, routes and the UI
   ═══════════════════════════════════════════════════════════════ */

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
    /// Per-note strike velocity in 0.05–1.0; `None` uses the pattern default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub velocity: Option<f64>,
}

fn default_note_length() -> u32 {
    1
}

impl Default for NoteOverride {
    fn default() -> Self {
        Self { step: 0, length: 1, degree: 0, octave: 0, velocity: None }
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
    /// Optional per-pad parameter overrides.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub params: HashMap<String, f64>,
}

fn default_pad_name() -> String {
    "Pad".into()
}

impl Default for PadConfig {
    fn default() -> Self {
        Self { name: "Pad".into(), kind: "kick".into(), params: HashMap::new() }
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
///
/// Macros are applied **by the engine** — each entry writes
/// `base + (value - 0.5) * 2 * amount * range` into its target. The UI still
/// shows them, but a macro now affects the render even when the frontend is
/// not involved (AI-authored configs, automation, exports).
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
    const K: &[&str] = &[
        "kick", "snare", "clap", "hat", "tom", "perc", "kick", "snare", "hat", "clap", "tom", "perc", "kick",
        "hat", "perc", "tom",
    ];
    const N: &[&str] = &[
        "Kick 1", "Snare", "Clap", "Hat", "Tom 1", "Perc", "Kick 2", "Snare 2", "Hat 2", "Clap 2", "Tom 2",
        "Perc 2", "Kick 3", "Hat 3", "Perc 3", "Tom 3",
    ];
    K.iter().zip(N).map(|(k, n)| PadConfig { name: n.to_string(), kind: k.to_string(), params: HashMap::new() }).collect()
}

/// One voice (track) in a pattern.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceConfig {
    /// Instrument kind — see [`voices::KINDS`].
    #[serde(default = "default_kind")]
    pub kind: String,
    /// `"e<hits>,<rot>"` (Euclidean fill) or an explicit `"x..x"` string.
    #[serde(default = "default_rhythm")]
    pub rhythm: String,
    /// Default scale degree for this voice's notes.
    pub degree: i32,
    /// Default octave offset.
    pub octave: i32,
    /// Waveform for `bass`/`sub` (sine/saw/square/triangle/pulse).
    pub wave: Option<String>,
    /// Per-step note overrides (degree + octave + length + velocity).
    pub notes: Vec<NoteOverride>,
    /// Mix level; `None` uses the per-kind default.
    pub level: Option<f32>,
    /// Stereo pan; `None` uses the per-kind default.
    pub pan: Option<f32>,
    /// Velocity accent (0–0.6): quarter-note hits get `+accent`, off-beat
    /// hits get a little softer — 0 keeps every hit at the base velocity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent: Option<f64>,
    /// Synth parameter overrides, keyed by the kind's param names.
    pub synth: HashMap<String, f64>,
    /// Insert effect chain.
    pub fx: Vec<EffectConfig>,
    /// Pad layout for a `drumkit` voice (16 pads; empty = default kit).
    pub pads: Vec<PadConfig>,
    /// Macro rack (8 knobs) — applied by the engine when `entries` are set.
    pub macros: Vec<MacroConfig>,
    /// MIDI effects (transpose/velocity/gate/ratchet) applied to note events.
    pub midi: Vec<MidiFxConfig>,
    /// Grid patch (modular) for `kind = "grid"` voices.
    pub grid: Option<GridPatch>,
    /// SoundFont bank filename (in the plugin's `soundfonts/` dir) for the
    /// `sampler` / `sfkit` kinds; `None` picks the first bank found.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub soundfont: Option<String>,
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
            accent: None,
            synth: HashMap::new(),
            fx: Vec::new(),
            pads: Vec::new(),
            macros: Vec::new(),
            midi: Vec::new(),
            grid: None,
            soundfont: None,
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
    /// Groove swing in 0–1: delays every other 16th-note step, where 1 is a
    /// full triplet swing (0 = straight/quantized).
    #[serde(default = "default_swing")]
    pub swing: f64,
    /// Tuning system: `edo12`, `edo19`, `edo24`, `edo31`, `ji7`, `pyth`, `harm`.
    #[serde(default = "default_tuning")]
    pub tuning: String,
    /// Reference frequency for pitch resolution (A4).
    #[serde(default = "default_ref")]
    pub ref_hz: f64,
    pub voices: Vec<VoiceConfig>,
    /// Master FX / loudness parameters, keyed by name.
    pub fx: HashMap<String, f64>,
}

impl Default for TrackConfig {
    fn default() -> Self {
        Self {
            title: "Untitled".into(),
            bpm: 120.0,
            steps: 16,
            swing: 0.0,
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
fn default_swing() -> f64 {
    0.0
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

/// Offline-render result: planar samples are discarded; the WAV is kept.
#[derive(Debug, Clone)]
pub struct Rendered {
    pub sample_rate: u32,
    pub channels: u32,
    pub duration_ms: u32,
    /// WAV bit depth (16 default, 24 opt-in).
    pub bits: u16,
    pub wav: Vec<u8>,
    /// Integrated loudness of the finished render (LUFS).
    pub lufs: f64,
    /// Sample peak of the finished render (linear).
    pub peak: f32,
}

/// Planar render: channel-major sample buffers (before WAV encoding).
#[derive(Debug, Clone)]
pub struct PlanarRender {
    pub sample_rate: u32,
    pub duration_ms: u32,
    pub frames: usize,
    pub channels: Vec<Vec<f32>>,
}

impl PlanarRender {
    pub fn lufs(&self) -> f64 {
        loudness::measure(&self.channels, self.sample_rate as f64).integrated_lufs
    }
}

/// Parse a raw JSON value into a validated [`TrackConfig`].
pub fn parse_config(value: &serde_json::Value) -> Result<TrackConfig, String> {
    let mut cfg: TrackConfig =
        serde_json::from_value(value.clone()).map_err(|e| format!("invalid config: {e}"))?;
    cfg.steps = cfg.steps.clamp(4, 64);
    cfg.bpm = cfg.bpm.clamp(40.0, 240.0);
    cfg.swing = cfg.swing.clamp(0.0, 1.0);
    cfg.ref_hz = cfg.ref_hz.clamp(220.0, 880.0);
    Ok(cfg)
}

/// Tuning systems the engine knows.
pub const TUNINGS: &[&str] = &["edo12", "edo19", "edo24", "edo31", "ji7", "pyth", "harm"];

/// Resolve a scale degree (plus octave) to a frequency in a tuning.
pub fn resolve_frequency(degree: i32, octave: i32, tuning: &str, ref_hz: f64) -> f64 {
    match tuning {
        "edo19" => ref_hz * 2f64.powf((degree + 19 * octave) as f64 / 19.0),
        "edo24" => ref_hz * 2f64.powf((degree + 24 * octave) as f64 / 24.0),
        "edo31" => ref_hz * 2f64.powf((degree + 31 * octave) as f64 / 31.0),
        "ji7" => {
            const R: [f64; 7] = [1.0, 9.0 / 8.0, 5.0 / 4.0, 4.0 / 3.0, 3.0 / 2.0, 5.0 / 3.0, 15.0 / 8.0];
            let oct = degree.div_euclid(7) + octave;
            let idx = degree.rem_euclid(7) as usize;
            ref_hz * R[idx] * 2f64.powi(oct)
        }
        "pyth" => {
            // Pythagorean: stacked perfect fifths, folded to one octave.
            let oct = degree.div_euclid(12) + octave;
            let idx = degree.rem_euclid(12) as usize;
            let mut ratio = 2f64.powf(idx as f64 * 7.0 / 12.0);
            while ratio >= 2.0 {
                ratio /= 2.0;
            }
            ref_hz * ratio * 2f64.powi(oct)
        }
        "harm" => {
            // Harmonic series: degree n is the (n+1)-th harmonic of the root.
            let n = (degree + 12 * octave).max(0) as f64;
            ref_hz * (n + 1.0)
        }
        // edo12 is the default and the fallback for unknown names.
        _ => ref_hz * 2f64.powf((degree + 12 * octave) as f64 / 12.0),
    }
}

/// True when `name` is a known tuning.
pub fn is_tuning(name: &str) -> bool {
    TUNINGS.contains(&name)
}

/* ═══════════════════════════════════════════════════════════════
   Step helpers — rhythm, swing, velocity, MIDI FX
   ═══════════════════════════════════════════════════════════════ */

/// Step indices that are "on" for a rhythm string (Euclidean or explicit).
pub fn rhythm_hits(rhythm: &str, steps: u32) -> Vec<u32> {
    let r = rhythm.trim();
    let mut hits = Vec::new();
    if let Some(rest) = r.strip_prefix('e') {
        let mut parts = rest.split(',');
        let h: u32 = parts.next().and_then(|s| s.trim().parse().ok()).unwrap_or(0).min(steps);
        let rot: u32 = parts.next().map(|s| s.trim().parse().unwrap_or(0)).unwrap_or(0);
        // Bjorklund's algorithm, inlined (no external dependency).
        let pattern = euclidean(h, steps);
        let rot = if steps == 0 { 0 } else { rot % steps };
        for (i, b) in pattern.iter().enumerate() {
            if *b {
                let shifted = (i + rot as usize) % steps.max(1) as usize;
                hits.push(shifted as u32);
            }
        }
        hits.sort_unstable();
    } else {
        for (i, c) in r.chars().filter(|c| *c == 'x' || *c == '.').take(steps as usize).enumerate() {
            if c == 'x' {
                hits.push(i as u32);
            }
        }
    }
    hits
}

/// Euclidean rhythm: `hits` onsets spread over `steps`.
pub fn euclidean(hits: u32, steps: u32) -> Vec<bool> {
    let steps = steps.max(1);
    let hits = hits.min(steps);
    let mut out = vec![false; steps as usize];
    if hits == 0 {
        return out;
    }
    // Bresenham-style distribution: onset when the accumulator wraps.
    let mut acc = 0u32;
    for (i, slot) in out.iter_mut().enumerate() {
        acc += hits;
        if acc >= steps {
            acc -= steps;
            *slot = true;
        }
        let _ = i;
    }
    // Guarantee exactly `hits` onsets (the wrap test can drop or add one at
    // the very start depending on phase).
    let count = out.iter().filter(|b| **b).count() as u32;
    if count < hits {
        for slot in out.iter_mut() {
            if !*slot && count < hits {
                *slot = true;
            }
        }
    }
    out
}

/// Swing onset offset in beats for a step index: odd 16th-note steps are
/// delayed by up to 1/12 beat (a full triplet shuffle) as `swing` goes 0→1.
#[inline]
pub fn swing_offset_beats(step: u32, swing: f64) -> f64 {
    if step % 2 == 1 {
        swing / 12.0
    } else {
        0.0
    }
}

/// Strike velocity for one hit. A per-note `velocity` override wins outright;
/// otherwise the kind's base is accented on quarter notes and slightly
/// softened on off-beats by the voice's `accent` (0–0.6).
#[inline]
pub fn hit_velocity(note_vel: Option<f64>, step: u32, accent: f64, base: f64) -> f64 {
    if let Some(v) = note_vel {
        return v.clamp(0.05, 1.0);
    }
    let v = if step % 4 == 0 { base + accent } else { base - accent * 0.4 };
    v.clamp(0.05, 1.0)
}

/// One scheduled note before it becomes a frame offset.
#[derive(Debug, Clone, Copy)]
struct PlannedNote {
    start: f64,
    end: f64,
    degree: i32,
    octave: i32,
    velocity: f64,
    /// Drum pad index (drumkit only).
    pad: usize,
}

/// Apply a voice's MIDI effects to one note, returning zero or more notes
/// (ratchet splits). Times are in beats.
fn apply_midi(
    midi: &[MidiFxConfig],
    start_step: u32,
    len_steps: u32,
    degree: i32,
    octave: i32,
    velocity: f64,
    steps: u32,
    pad: usize,
) -> Vec<PlannedNote> {
    let mut deg = degree;
    let oct = octave;
    let mut vel = velocity;
    let mut gate = 1.0;
    let mut ratchet = 0u32;
    let mut probability = 1.0;
    // Timing offset in beats from `humanize`; applied after the whole chain so
    // every MIDI effect (before and after it) still takes part.
    let mut shift = 0.0;
    for fx in midi {
        match fx.kind.as_str() {
            "transpose" => deg += fx.params.get("steps").copied().unwrap_or(0.0) as i32,
            "velocity" => vel *= fx.params.get("amount").copied().unwrap_or(1.0).clamp(0.0, 2.0),
            "gate" => gate = fx.params.get("amount").copied().unwrap_or(1.0).clamp(0.1, 4.0),
            "ratchet" => ratchet = fx.params.get("count").copied().unwrap_or(2.0).clamp(2.0, 8.0) as u32,
            "probability" | "chance" => probability = fx.params.get("amount").copied().unwrap_or(1.0).clamp(0.0, 1.0),
            "humanize" => {
                // Deterministic pseudo-humanize: a fixed fraction of a step,
                // alternating direction by note, so renders stay reproducible.
                let amt = fx.params.get("amount").copied().unwrap_or(0.0).clamp(0.0, 1.0);
                let even = (start_step as i64 + deg as i64).rem_euclid(2) == 0;
                shift = if even { amt } else { -amt } * 0.02;
            }
            _ => {}
        }
    }
    if probability < 1.0 {
        // Deterministic "probability": drop when a hash of the note lands
        // above the threshold.
        let h = (start_step as f64 * 2654435761.0 + deg as f64 * 40503.0) as u64;
        let r = (h % 1000) as f64 / 1000.0;
        if r > probability {
            return Vec::new();
        }
    }
    let start_beat = (start_step as f64 / 4.0 + shift).max(0.0);
    let dur = (len_steps as f64 / 4.0 * gate).clamp(0.02, steps as f64 / 4.0);
    let end_beat = (start_beat + dur).min(steps as f64 / 4.0);
    let mut out = Vec::new();
    if ratchet > 1 {
        let seg = dur / ratchet as f64;
        for k in 0..ratchet {
            let s = start_beat + k as f64 * seg;
            let e = (s + seg * 0.85).min(end_beat.max(s + 0.01));
            out.push(PlannedNote { start: s, end: e, degree: deg, octave: oct, velocity: vel.clamp(0.05, 1.2), pad });
        }
    } else {
        out.push(PlannedNote { start: start_beat, end: end_beat.max(start_beat + 0.02), degree: deg, octave: oct, velocity: vel.clamp(0.05, 1.2), pad });
    }
    out
}

/// Rhythm/notes → planned notes in beats for one voice.
fn plan_voice(v: &VoiceConfig, steps: u32) -> Vec<PlannedNote> {
    let accent = v.accent.unwrap_or(0.0).clamp(0.0, 0.6);
    let mut notes: Vec<(u32, u32, i32, i32, Option<f64>, usize)> = Vec::new();

    if v.kind == "drumkit" {
        if v.notes.is_empty() {
            // A drumkit with only a rhythm string triggers pad 0 — friendlier
            // for quick AI patterns than rendering silence.
            for step in rhythm_hits(&v.rhythm, steps) {
                notes.push((step, 1, 0, 0, None, 0));
            }
        } else {
            for n in &v.notes {
                let pad = ((n.degree % 16) + 16) % 16;
                let start = n.step.min(steps.saturating_sub(1));
                notes.push((start, 1, 0, 0, n.velocity, pad as usize));
            }
        }
    } else {
        let melodic = voices::is_melodic(&v.kind);
        if melodic && !v.notes.is_empty() {
            for n in &v.notes {
                notes.push((n.step.min(steps.saturating_sub(1)), n.length.max(1), n.degree, n.octave, n.velocity, 0));
            }
        } else {
            for step in rhythm_hits(&v.rhythm, steps) {
                notes.push((step, 1, v.degree, v.octave, None, 0));
            }
        }
    }

    let mut out = Vec::new();
    for (start, len, degree, octave, note_vel, pad) in notes {
        let base = if v.kind == "drumkit" { 0.85 } else if voices::is_drum(&v.kind) { 0.85 } else { 0.75 };
        let vel = hit_velocity(note_vel, start, accent, base);
        out.extend(apply_midi(&v.midi, start, len, degree, octave, vel, steps, pad));
    }
    // Swing shifts every odd 16th later (all voices share the groove).
    out
}

/// Sample an automation envelope at a beat position (linear interpolation;
/// holds the end values outside the breakpoint span). `fallback` is returned
/// when the envelope is empty.
pub fn envelope_at(points: &[AutomationPoint], beat: f64, fallback: f64) -> f64 {
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


/* ═══════════════════════════════════════════════════════════════
   Channels — a built instrument plus its insert chain
   ═══════════════════════════════════════════════════════════════ */

/// A note scheduled at an absolute frame (relative to the clip start).
#[derive(Debug, Clone, Copy)]
struct Ev {
    at: usize,
    id: u64,
    on: bool,
    freq: f64,
    vel: f64,
    pad: usize,
}

enum Instrument {
    Synth(Box<Synth>),
    Drum(Box<Drum>),
    Drumkit(Vec<Drum>),
    Grid(Box<GridEngine>),
    Sampler(Box<Sampler>),
}

/// The Web Audio `StereoPannerNode` law for a stereo input, implemented
/// exactly so the browser's live mixer and the offline render agree: centre
/// is unity on both sides, panning folds the far side into the near one.
#[inline]
pub fn stereo_pan(l: f32, r: f32, pan: f64) -> (f32, f32) {
    let p = pan.clamp(-1.0, 1.0);
    if p == 0.0 {
        return (l, r);
    }
    let x = if p <= 0.0 { p + 1.0 } else { p };
    let a = x * std::f64::consts::FRAC_PI_2;
    let (gl, gr) = (a.cos() as f32, a.sin() as f32);
    if p <= 0.0 {
        (l + r * gl, r * gr)
    } else {
        (l * gl, r + l * gr)
    }
}

/// One voice's live processing chain.
struct Channel {
    instrument: Instrument,
    params: HashMap<String, f64>,
    wave: Option<String>,
    kind: String,
    effects: Vec<Effect>,
    level: f64,
    pan: f64,
    level_sm: Smoother,
    pan_sm: Smoother,
    events: Vec<Ev>,
    cursor: usize,
    msgs: Vec<NoteMsg>,
    pending: Vec<(usize, Ev)>,
    scratch_l: [f32; crate::dsp::BLOCK],
    scratch_r: [f32; crate::dsp::BLOCK],
    sr: f64,
    /// Whether a synth parameter changed since the last block.
    params_dirty: bool,
    /// Frames rendered since the instrument went quiet (insert tails).
    quiet_frames: usize,
    tail_frames: usize,
}

impl Channel {
    fn new(v: &VoiceConfig, vi: usize, seed: u64, sr: f64) -> Result<Channel, String> {
        let mut params = voices::kind_params(&v.kind);
        for (k, val) in &v.synth {
            if val.is_finite() {
                params.insert(k.clone(), *val);
            }
        }
        apply_macros(&mut params, &v.macros);

        let mut instrument = if v.kind == "drumkit" {
            let pads = if v.pads.is_empty() { default_pads() } else { v.pads.clone() };
            let mut drums = Vec::with_capacity(pads.len().min(16));
            for (pi, pad) in pads.iter().take(16).enumerate() {
                let mut d = Drum::from_kind(&pad.kind, seed ^ ((vi * 31 + pi) as u64));
                d.set_sample_rate(sr);
                for (k, val) in &pad.params {
                    d.set_param(k, *val);
                }
                drums.push(d);
            }
            Instrument::Drumkit(drums)
        } else if v.kind == "grid" {
            let patch = v.grid.as_ref().ok_or_else(|| "grid voice missing its patch".to_string())?;
            Instrument::Grid(Box::new(GridEngine::compile(patch, seed ^ vi as u64, sr)?))
        } else if v.kind == "sampler" || v.kind == "sfkit" {
            let drum = v.kind == "sfkit";
            let sf = crate::dsp::sampler::load(v.soundfont.as_deref())?;
            let program = params.get("program").copied().unwrap_or(0.0).round() as i32;
            let bank = params.get("bank").copied().unwrap_or(if drum { 128.0 } else { 0.0 }).round() as i32;
            Instrument::Sampler(Box::new(Sampler::new(sf, drum, program, bank, sr)?))
        } else if voices::is_drum(&v.kind) {
            let mut d = Drum::from_kind(&v.kind, seed ^ vi as u64);
            d.set_sample_rate(sr);
            for (k, val) in &params {
                d.set_param(k, *val);
            }
            Instrument::Drum(Box::new(d))
        } else {
            Instrument::Synth(Box::new(Synth::new(&v.kind, v.wave.as_deref(), &params, seed ^ vi as u64, sr)))
        };
        if let Instrument::Synth(s) = &mut instrument {
            s.set_sample_rate(sr);
        }

        let mut effects = Vec::new();
        for e in &v.fx {
            if e.bypass {
                continue;
            }
            match EffectKind::from_name(&e.kind) {
                Some(k) => effects.push(Effect::new(k, &e.params, sr)),
                None => return Err(format!("unknown effect `{}` (one of {})", e.kind, crate::fx::EFFECT_KINDS.join(", "))),
            }
        }
        let tail_frames = (effects.iter().map(|e| e.tail_seconds()).fold(0.0, f64::max).min(20.0) * sr) as usize;

        let level = v.level.unwrap_or_else(|| voices::default_level(&v.kind)) as f64;
        let pan = v.pan.unwrap_or_else(|| voices::default_pan(&v.kind)) as f64;
        Ok(Channel {
            instrument,
            params,
            wave: v.wave.clone(),
            kind: v.kind.clone(),
            effects,
            level,
            pan,
            level_sm: Smoother::with_sr(level, 0.01, sr),
            pan_sm: Smoother::with_sr(pan, 0.01, sr),
            events: Vec::new(),
            cursor: 0,
            msgs: Vec::new(),
            pending: Vec::new(),
            scratch_l: [0.0; crate::dsp::BLOCK],
            scratch_r: [0.0; crate::dsp::BLOCK],
            sr,
            params_dirty: false,
            quiet_frames: 0,
            tail_frames,
        })
    }

    /// Apply a device-automation path relative to this channel.
    fn set_param(&mut self, path: &str, value: f64) {
        if let Some(rest) = path.strip_prefix("fx.") {
            let mut parts = rest.splitn(2, '.');
            let idx = parts.next().and_then(|s| s.parse::<usize>().ok());
            if let (Some(i), Some(k)) = (idx, parts.next()) {
                if let Some(e) = self.effects.get_mut(i) {
                    e.set_param(k, value);
                }
            }
            return;
        }
        match path {
            "level" => self.level = value.clamp(0.0, 2.0),
            "pan" => self.pan = value.clamp(-1.0, 1.0),
            "accent" => {}
            _ => match &mut self.instrument {
                Instrument::Synth(_) => {
                    self.params.insert(path.to_string(), value);
                    self.params_dirty = true;
                }
                Instrument::Drum(d) => d.set_param(path, value),
                Instrument::Drumkit(pads) => {
                    for d in pads.iter_mut() {
                        d.set_param(path, value);
                    }
                }
                Instrument::Grid(_) | Instrument::Sampler(_) => {}
            },
        }
    }

    fn instrument_active(&self) -> bool {
        match &self.instrument {
            Instrument::Synth(s) => s.active_voices() > 0,
            Instrument::Drum(d) => d.is_active(),
            Instrument::Drumkit(pads) => pads.iter().any(|d| d.is_active()),
            Instrument::Grid(g) => g.is_active(),
            Instrument::Sampler(s) => s.is_active(),
        }
    }

    /// Whether this channel still produces (or may produce) sound.
    fn is_active(&self) -> bool {
        self.cursor < self.events.len() || self.instrument_active() || self.quiet_frames < self.tail_frames
    }

    /// Render one block starting at clip frame `block_start`, adding into
    /// `l`/`r`.
    fn render_block(&mut self, block_start: usize, frames: usize, l: &mut [f32], r: &mut [f32]) {
        if self.params_dirty {
            if let Instrument::Synth(s) = &mut self.instrument {
                s.apply_map(&self.params, self.wave.as_deref(), &self.kind);
            }
            self.params_dirty = false;
        }
        self.msgs.clear();
        let one_shot = !matches!(self.instrument, Instrument::Synth(_) | Instrument::Sampler(_));
        while self.cursor < self.events.len() && self.events[self.cursor].at < block_start + frames {
            let ev = self.events[self.cursor];
            self.cursor += 1;
            let at = ev.at.saturating_sub(block_start).min(frames.saturating_sub(1));
            if one_shot {
                self.pending.push((at, ev));
            } else {
                self.msgs.push(NoteMsg {
                    at,
                    id: ev.id,
                    pad: ev.pad,
                    kind: if ev.on { NoteKind::On { freq: ev.freq, velocity: ev.vel } } else { NoteKind::Off },
                });
            }
        }
        let was_active = self.instrument_active() || !self.pending.is_empty() || !self.msgs.is_empty();

        self.scratch_l[..frames].fill(0.0);
        self.scratch_r[..frames].fill(0.0);
        let sr = self.sr;
        match &mut self.instrument {
            Instrument::Synth(s) => {
                s.render(&mut self.scratch_l[..frames], &mut self.scratch_r[..frames], &self.msgs, frames, sr);
            }
            Instrument::Drum(d) => {
                let mut pi = 0usize;
                for k in 0..frames {
                    while pi < self.pending.len() && self.pending[pi].0 <= k {
                        if self.pending[pi].1.on {
                            d.trigger(self.pending[pi].1.vel);
                        }
                        pi += 1;
                    }
                    let v = d.tick(sr);
                    self.scratch_l[k] = v;
                    self.scratch_r[k] = v;
                }
            }
            Instrument::Drumkit(pads) => {
                let mut pi = 0usize;
                for k in 0..frames {
                    while pi < self.pending.len() && self.pending[pi].0 <= k {
                        let ev = self.pending[pi].1;
                        if ev.on {
                            if let Some(pad) = pads.get_mut(ev.pad) {
                                pad.trigger(ev.vel);
                            }
                        }
                        pi += 1;
                    }
                    let mut v = 0.0;
                    for pad in pads.iter_mut() {
                        if pad.is_active() {
                            v += pad.tick(sr);
                        }
                    }
                    self.scratch_l[k] = v;
                    self.scratch_r[k] = v;
                }
            }
            Instrument::Grid(g) => {
                // Grid processes whole blocks; split the block at each event
                // so notes still land on their sample.
                let mut pos = 0usize;
                let mut pi = 0usize;
                while pos < frames {
                    while pi < self.pending.len() && self.pending[pi].0 <= pos {
                        let ev = self.pending[pi].1;
                        if ev.on {
                            g.note_on(ev.freq, ev.vel);
                        } else {
                            g.note_off();
                        }
                        pi += 1;
                    }
                    let next = self.pending.get(pi).map(|p| p.0).unwrap_or(frames).clamp(pos + 1, frames);
                    g.render(&mut self.scratch_l[pos..next], &mut self.scratch_r[pos..next], next - pos);
                    pos = next;
                }
            }
            Instrument::Sampler(s) => {
                s.render(&mut self.scratch_l[..frames], &mut self.scratch_r[..frames], &self.msgs, frames);
            }
        }
        self.pending.clear();
        if was_active {
            self.quiet_frames = 0;
        } else {
            self.quiet_frames += frames;
        }

        for e in self.effects.iter_mut() {
            for k in 0..frames {
                let (nl, nr) = e.tick(self.scratch_l[k], self.scratch_r[k]);
                self.scratch_l[k] = nl;
                self.scratch_r[k] = nr;
            }
        }

        self.level_sm.aim(self.level);
        self.pan_sm.aim(self.pan);
        for k in 0..frames {
            let g = self.level_sm.next() as f32;
            let p = self.pan_sm.next();
            let (pl, pr) = stereo_pan(self.scratch_l[k] * g, self.scratch_r[k] * g, p);
            l[k] += pl;
            r[k] += pr;
        }
    }
}

/// Expand a macro rack into parameter values.
fn apply_macros(params: &mut HashMap<String, f64>, macros: &[MacroConfig]) {
    for mac in macros.iter().take(8) {
        let norm = (mac.value.clamp(0.0, 1.0) - 0.5) * 2.0;
        for entry in &mac.entries {
            if entry.path.is_empty() {
                continue;
            }
            let base = if entry.base != 0.0 { entry.base } else { params.get(&entry.path).copied().unwrap_or(0.0) };
            let range = voices::param_range(&entry.path).unwrap_or(1.0);
            let mut v = base + norm * entry.amount * range * 0.5;
            if let Some(d) = voices::param_def(&entry.path) {
                v = v.clamp(d.min, d.max);
            }
            params.insert(entry.path.clone(), v);
        }
    }
}

/* ═══════════════════════════════════════════════════════════════
   Pattern scheduling and rendering
   ═══════════════════════════════════════════════════════════════ */

fn validate(cfg: &TrackConfig) -> Result<(), String> {
    if cfg.voices.len() > voices::MAX_VOICES {
        return Err(format!("too many voices (max {})", voices::MAX_VOICES));
    }
    for v in &cfg.voices {
        if !voices::is_kind(&v.kind) {
            return Err(format!("unknown voice kind `{}` (one of {})", v.kind, voices::KINDS.join(", ")));
        }
        if v.fx.len() > 8 {
            return Err("too many effects on one voice (max 8)".into());
        }
        for e in &v.fx {
            if EffectKind::from_name(&e.kind).is_none() {
                return Err(format!("unknown effect `{}` (one of {})", e.kind, crate::fx::EFFECT_KINDS.join(", ")));
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
    if !is_tuning(&cfg.tuning) {
        return Err(format!("unknown tuning `{}` (use {})", cfg.tuning, TUNINGS.join(", ")));
    }
    Ok(())
}

/// Length of one pass of a pattern, in beats.
pub fn pattern_beats(cfg: &TrackConfig) -> f64 {
    cfg.steps.clamp(4, 64) as f64 / 4.0
}

/// Per-voice event lists (frames relative to the clip start) for a pattern
/// played for `clip_beats`: the pattern repeats when `looped`, events at or
/// past the clip end are dropped and notes still held are released at it.
fn build_events(cfg: &TrackConfig, sr: f64, bpm: f64, clip_beats: f64, looped: bool, offset_beats: f64) -> Vec<Vec<Ev>> {
    let steps = cfg.steps.clamp(4, 64);
    let loop_beats = steps as f64 / 4.0;
    let spb = sr * 60.0 / bpm;
    let clip_end = (clip_beats * spb).round().max(1.0) as usize;
    // `offset_beats` starts playback inside the pattern (a clip split in two
    // keeps the second half where it was in the loop).
    let start_at = if looped { offset_beats.max(0.0) % loop_beats } else { offset_beats.max(0.0) };
    let passes = if looped { ((start_at + clip_beats) / loop_beats).ceil().max(1.0) as usize } else { 1 };
    let mut out = Vec::with_capacity(cfg.voices.len());
    for v in &cfg.voices {
        let melodic = voices::is_melodic(&v.kind) || v.kind == "grid";
        let planned = plan_voice(v, steps);
        let mut evs: Vec<Ev> = Vec::new();
        for pass in 0..passes {
            let offset = pass as f64 * loop_beats - start_at;
            for (ni, note) in planned.iter().enumerate() {
                let swing = swing_offset_beats((note.start * 4.0).floor().max(0.0) as u32, cfg.swing);
                let on_beat = offset + note.start + swing;
                if on_beat < -1e-9 {
                    continue; // starts before the clip's window into the pattern
                }
                let on = (on_beat * spb).round().max(0.0) as usize;
                if on >= clip_end {
                    continue;
                }
                let off = (((offset + note.end + swing) * spb).round().max(0.0) as usize).min(clip_end).max(on + 1);
                let id = (pass as u64) << 20 | (ni as u64 + 1);
                let freq = if melodic { resolve_frequency(note.degree, note.octave, &cfg.tuning, cfg.ref_hz) } else { 0.0 };
                evs.push(Ev { at: on, id, on: true, freq, vel: note.velocity, pad: note.pad });
                if melodic {
                    evs.push(Ev { at: off, id, on: false, freq, vel: 0.0, pad: note.pad });
                }
            }
        }
        // Offs before ons at the same frame, so a repeated note retriggers.
        evs.sort_by_key(|e| (e.at, e.on));
        out.push(evs);
    }
    out
}

/// Longest tail rendered past a clip's end (releases, insert reverbs).
const MAX_TAIL_SECS: f64 = 12.0;

/// One automation lane resolved for a channel or the clip.
struct LaneTarget<'a> {
    voice: Option<usize>,
    path: &'a str,
    lane: &'a AutomationLane,
    cached: f64,
}

/// Render a pattern clip into fresh stereo buffers: `clip_beats` of music
/// (looping the pattern when `looped`) plus the natural tail. Device
/// automation lanes (`voice.<i>.…`) are evaluated at the absolute beat
/// `base_beat + local`.
#[allow(clippy::too_many_arguments)]
pub fn render_pattern_clip(
    cfg: &TrackConfig,
    bpm: f64,
    base_beat: f64,
    clip_beats: f64,
    looped: bool,
    offset_beats: f64,
    lanes: &[AutomationLane],
    seed: u64,
    sr: f64,
) -> Result<(Vec<f32>, Vec<f32>), String> {
    validate(cfg)?;
    let bpm = bpm.clamp(40.0, 240.0);
    let spb = sr * 60.0 / bpm;
    let music = (clip_beats * spb).ceil() as usize;
    let max_total = music + (MAX_TAIL_SECS * sr) as usize;

    let mut channels = Vec::with_capacity(cfg.voices.len());
    for (vi, v) in cfg.voices.iter().enumerate() {
        channels.push(Channel::new(v, vi, seed, sr)?);
    }
    for (c, evs) in channels.iter_mut().zip(build_events(cfg, sr, bpm, clip_beats, looped, offset_beats)) {
        c.events = evs;
    }
    let mut targets: Vec<LaneTarget> = Vec::new();
    for lane in lanes {
        if let Some(rest) = lane.param.strip_prefix("voice.") {
            let mut parts = rest.splitn(2, '.');
            if let (Some(vi), Some(path)) = (parts.next().and_then(|s| s.parse::<usize>().ok()), parts.next()) {
                if vi < channels.len() && !lane.points.is_empty() {
                    targets.push(LaneTarget { voice: Some(vi), path, lane, cached: f64::NAN });
                }
            }
        }
    }

    let mut l = vec![0.0f32; max_total];
    let mut r = vec![0.0f32; max_total];
    let block = crate::dsp::BLOCK;
    let mut start = 0usize;
    let mut end = music;
    while start < max_total {
        let frames = block.min(max_total - start);
        if !targets.is_empty() {
            let beat = base_beat + start as f64 / spb;
            for t in targets.iter_mut() {
                let value = envelope_at(&t.lane.points, beat, f64::NAN);
                if value.is_finite() && (t.cached.is_nan() || (t.cached - value).abs() > 1e-9) {
                    if let Some(ch) = t.voice.and_then(|vi| channels.get_mut(vi)) {
                        ch.set_param(t.path, value);
                    }
                    t.cached = value;
                }
            }
        }
        let mut any = false;
        for ch in channels.iter_mut() {
            if !ch.is_active() {
                continue;
            }
            any = true;
            ch.render_block(start, frames, &mut l[start..start + frames], &mut r[start..start + frames]);
        }
        start += frames;
        if start >= music {
            end = start;
            if !any {
                break;
            }
        }
    }
    let end = end.max(music).min(max_total);
    l.truncate(end);
    r.truncate(end);
    Ok((l, r))
}

/// Compute a min/max peak envelope of a pattern's render (for previews).
pub fn waveform_peaks(cfg: &TrackConfig, buckets: usize) -> Result<Vec<(f32, f32)>, String> {
    let p = render_planar_track(cfg)?;
    Ok(peaks_of(&p.channels, buckets))
}

pub fn peaks_of(channels: &[Vec<f32>], buckets: usize) -> Vec<(f32, f32)> {
    let n = buckets.clamp(16, 4096);
    let frames = channels.first().map(|c| c.len()).unwrap_or(0);
    let mut peaks = Vec::with_capacity(n);
    for b in 0..n {
        let s = b * frames / n;
        let e = ((b + 1) * frames / n).max(s + 1).min(frames);
        let mut mn = 0.0f32;
        let mut mx = 0.0f32;
        for ch in channels {
            for x in &ch[s.min(ch.len())..e.min(ch.len())] {
                mn = mn.min(*x);
                mx = mx.max(*x);
            }
        }
        peaks.push((mn, mx));
    }
    peaks
}

/// A standalone pattern as a one-track arrangement: its `fx` become the
/// master options, `reverb_mix`/`delay_mix` become the track's sends into
/// the shared returns (parallel, the way a mixing desk does it).
pub fn pattern_arrangement(cfg: &TrackConfig) -> Arrangement {
    let beats = pattern_beats(cfg);
    let track = ArrangementTrack { id: "pattern".into(), name: cfg.title.clone(), level: 1.0, ..ArrangementTrack::default() };
    Arrangement {
        title: cfg.title.clone(),
        bpm: cfg.bpm,
        length_beats: beats,
        master: 1.0,
        fx: cfg.fx.clone(),
        tracks: vec![track],
        clips: vec![ArrangementClip {
            track: "pattern".into(),
            start: 0.0,
            length_beats: Some(beats),
            looped: false,
            pattern: cfg.clone(),
            ..ArrangementClip::default()
        }],
        ..Arrangement::default()
    }
}

/// Render a standalone pattern (no automation) through the full mix path.
pub fn render_planar_track(cfg: &TrackConfig) -> Result<PlanarRender, String> {
    let a = pattern_arrangement(cfg);
    let mix = crate::mix::render_mix(&a, &crate::audio::NoAudio, &crate::mix::MixOptions::export())?;
    Ok(mix.master)
}

/// Validate and render a single pattern to WAV (loudness target honoured).
pub fn render_track(cfg: &TrackConfig) -> Result<Rendered, String> {
    let p = render_planar_track(cfg)?;
    Ok(encode_rendered(&p, output_bits(&cfg.fx)))
}

/// Encode a finished render and measure it.
pub fn encode_rendered(p: &PlanarRender, bits: u16) -> Rendered {
    let report = loudness::measure(&p.channels, p.sample_rate as f64);
    Rendered {
        sample_rate: p.sample_rate,
        channels: 2,
        duration_ms: p.duration_ms,
        bits,
        wav: encode_wav_bits(&p.channels, p.sample_rate, bits),
        lufs: report.integrated_lufs,
        peak: report.peak,
    }
}

pub fn arrangement_bits(a: &Arrangement) -> u16 {
    output_bits(&a.fx)
}

pub fn arrangement_sr(a: &Arrangement) -> f64 {
    output_sr(&a.fx)
}

/* ═══════════════════════════════════════════════════════════════
   Arrangement
   ═══════════════════════════════════════════════════════════════ */

/// Post-fader send levels into the shared reverb and delay returns (0–1).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct TrackSends {
    pub reverb: f64,
    pub delay: f64,
}

/// One arrangement lane (a horizontal row in the DAW).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ArrangementTrack {
    pub id: String,
    #[serde(default = "default_track_name")]
    pub name: String,
    /// `instrument` (pattern clips) or `audio` (WAV clips). A hint for the
    /// UI — the renderer plays whatever clips a track holds.
    #[serde(default = "default_track_kind")]
    pub kind: String,
    pub color: u32,
    pub mute: bool,
    /// Solo is honoured by the renderer as well as the UI.
    pub solo: bool,
    /// Fader gain, linear (1.0 = 0 dB, 2.0 = +6 dB).
    #[serde(default = "default_track_level")]
    pub level: f32,
    pub pan: f32,
    /// Track insert chain (after the clips, before the fader).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fx: Vec<EffectConfig>,
    /// Post-fader sends. `None` derives them from the clips' pattern
    /// `reverb_mix`/`delay_mix` (projects made before sends existed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sends: Option<TrackSends>,
    /// Per-track automation envelopes.
    #[serde(default)]
    pub automation: TrackAutomation,
}

impl Default for ArrangementTrack {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: "Track".into(),
            kind: default_track_kind(),
            color: 0,
            mute: false,
            solo: false,
            level: 0.8,
            pan: 0.0,
            fx: Vec::new(),
            sends: None,
            automation: TrackAutomation::default(),
        }
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
    /// Target path: `track.level` | `track.pan` | `send.reverb` |
    /// `send.delay` (per-sample, at the fader) or a device path —
    /// `voice.<i>.<param>` | `voice.<i>.level` | `voice.<i>.pan` |
    /// `voice.<i>.fx.<j>.<key>` | `fx.<j>.<key>` (track insert) |
    /// `master.<key>`.
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
fn default_track_kind() -> String {
    "instrument".into()
}
fn default_track_level() -> f32 {
    0.8
}
fn default_true() -> bool {
    true
}
fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

/// An audio region: a WAV file (relative to the user's Files home) placed on
/// the timeline.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AudioClipConfig {
    /// Path relative to the Files home, e.g. `Music/Generated/song.wav`.
    pub src: String,
    /// Display name (defaults to the file name in the UI).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Seconds into the file where playback starts (trim).
    pub offset: f64,
    /// Fade-in / fade-out lengths in seconds (equal-power curves).
    pub fade_in: f64,
    pub fade_out: f64,
    /// Tempo of the source when known (shown in the UI, used by "match tempo").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bpm: Option<f64>,
}

/// A clip on a track: a pattern (looped to `length_beats`) or an audio file.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ArrangementClip {
    /// Which track (by id) this clip lives on.
    pub track: String,
    /// Start position in beats.
    pub start: f64,
    /// Clip length in beats; `None` uses the pattern's own length (or the
    /// rest of the audio file).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length_beats: Option<f64>,
    /// Clip gain in dB.
    #[serde(default)]
    pub gain_db: f64,
    /// Where playback starts inside the pattern, in beats (set when a clip is
    /// split or its start is trimmed).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub offset_beats: f64,
    /// Repeat the pattern to fill `length_beats` (default true).
    #[serde(default = "default_true", rename = "loop")]
    pub looped: bool,
    /// When set, the clip plays this audio file instead of `pattern`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<AudioClipConfig>,
    pub pattern: TrackConfig,
}

impl Default for ArrangementClip {
    fn default() -> Self {
        Self {
            track: String::new(),
            start: 0.0,
            length_beats: None,
            gain_db: 0.0,
            offset_beats: 0.0,
            looped: true,
            audio: None,
            pattern: TrackConfig::default(),
        }
    }
}

/// The shared reverb and delay returns.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ReturnsConfig {
    /// Reverb parameters (`size`, `damping`, `predelay`, `width`, `mod`).
    pub reverb: HashMap<String, f64>,
    /// Delay parameters (`time` ms or `sync` beats, `feedback`, `damp`,
    /// `ping_pong`). The return delay has no modulation; `mod` is only read by
    /// the delay insert effect.
    pub delay: HashMap<String, f64>,
    /// Return fader gains (linear, default 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reverb_level: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delay_level: Option<f64>,
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
    /// Master bus options (loudness target, glue, ceiling, width, …).
    #[serde(default)]
    pub fx: HashMap<String, f64>,
    /// Master insert chain (before the bus glue and the limiter).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub master_fx: Vec<EffectConfig>,
    #[serde(default)]
    pub returns: ReturnsConfig,
    pub tracks: Vec<ArrangementTrack>,
    pub clips: Vec<ArrangementClip>,
}

fn default_master() -> f32 {
    0.9
}

impl Default for Arrangement {
    fn default() -> Self {
        Self {
            title: "Untitled".into(),
            bpm: 120.0,
            length_beats: 16.0,
            master: 0.9,
            fx: HashMap::new(),
            master_fx: Vec::new(),
            returns: ReturnsConfig::default(),
            tracks: Vec::new(),
            clips: Vec::new(),
        }
    }
}

/// Engine limits for arrangements.
pub const MAX_TRACKS: usize = 32;
pub const MAX_CLIPS: usize = 256;
pub const MAX_BEATS: f64 = 1024.0;

/// Parse an arrangement from JSON (clamped and validated).
pub fn parse_arrangement(value: &serde_json::Value) -> Result<Arrangement, String> {
    let mut a: Arrangement =
        serde_json::from_value(value.clone()).map_err(|e| format!("invalid arrangement: {e}"))?;
    a.bpm = a.bpm.clamp(40.0, 240.0);
    a.length_beats = a.length_beats.clamp(4.0, MAX_BEATS);
    for track in &mut a.tracks {
        // `envelope_at` assumes breakpoints are ordered by beat.
        for lane in &mut track.automation.lanes {
            lane.points.retain(|p| p.beat.is_finite() && p.value.is_finite());
            lane.points.sort_by(|x, y| x.beat.partial_cmp(&y.beat).unwrap_or(std::cmp::Ordering::Equal));
        }
        if track.fx.len() > 8 {
            return Err(format!("track `{}` has too many effects (max 8)", track.name));
        }
        for e in &track.fx {
            if EffectKind::from_name(&e.kind).is_none() {
                return Err(format!("unknown effect `{}` (one of {})", e.kind, crate::fx::EFFECT_KINDS.join(", ")));
            }
        }
        track.level = track.level.clamp(0.0, 4.0);
        track.pan = track.pan.clamp(-1.0, 1.0);
    }
    if a.master_fx.len() > 8 {
        return Err("too many master effects (max 8)".into());
    }
    for e in &a.master_fx {
        if EffectKind::from_name(&e.kind).is_none() {
            return Err(format!("unknown effect `{}` (one of {})", e.kind, crate::fx::EFFECT_KINDS.join(", ")));
        }
    }
    for c in &mut a.clips {
        c.start = c.start.max(0.0);
        if let Some(l) = c.length_beats {
            c.length_beats = Some(l.clamp(0.25, MAX_BEATS));
        }
        c.gain_db = c.gain_db.clamp(-60.0, 24.0);
        c.offset_beats = if c.offset_beats.is_finite() { c.offset_beats.clamp(0.0, MAX_BEATS) } else { 0.0 };
        c.pattern.bpm = a.bpm;
        c.pattern.steps = c.pattern.steps.clamp(4, 64);
    }
    Ok(a)
}

/// Render an arrangement to WAV with the pattern-only resolver (audio clips
/// fail). Routes and tools use [`crate::mix`] with a user resolver.
pub fn render_arrangement(a: &Arrangement) -> Result<Rendered, String> {
    let mix = crate::mix::render_mix(a, &crate::audio::NoAudio, &crate::mix::MixOptions::export())?;
    Ok(encode_rendered(&mix.master, output_bits(&a.fx)))
}

/// Analysis of a finished render — returned by the AI-facing analysis tool so
/// the model can *hear* (statistically) what it built and iterate.
#[derive(Debug, Clone, Serialize)]
pub struct Analysis {
    pub duration_ms: u32,
    pub lufs: f64,
    pub peak: f32,
    pub rms_db: f64,
    pub crest_db: f64,
    pub clipped: f64,
    pub range_lu: f64,
    /// Energy in five bands: sub, low, mid, high-mid, air (fractions of total).
    pub bands: [f64; 5],
    pub channels: u32,
}

/// Measure a pattern: loudness, peak, crest and a coarse spectral balance.
pub fn analyze(cfg: &TrackConfig) -> Result<Analysis, String> {
    let p = render_planar_track(cfg)?;
    Ok(analyze_planar(&p))
}

pub fn analyze_planar(p: &PlanarRender) -> Analysis {
    analyze_channels(&p.channels, p.sample_rate as f64)
}

pub fn analyze_channels(channels: &[Vec<f32>], sr: f64) -> Analysis {
    let report = loudness::measure(channels, sr);
    let frames = channels.first().map(|c| c.len()).unwrap_or(0);
    Analysis {
        duration_ms: (frames as f64 / sr * 1000.0).round() as u32,
        lufs: report.integrated_lufs,
        peak: report.peak,
        rms_db: report.rms_db,
        crest_db: report.crest_db,
        clipped: report.clipped,
        range_lu: report.range_lu,
        bands: band_energy(channels, sr),
        channels: channels.len() as u32,
    }
}

/// Coarse 5-band energy split (Linkwitz-ish complementary one-pole taps) —
/// enough for an LLM to reason about "too much sub" or "too bright".
fn band_energy(channels: &[Vec<f32>], sr: f64) -> [f64; 5] {
    const SPLITS: [f64; 4] = [120.0, 500.0, 2000.0, 6000.0];
    let mut energy = [0.0f64; 5];
    let coefs: Vec<f64> = SPLITS.iter().map(|f| 1.0 - (-2.0 * std::f64::consts::PI * f / sr).exp()).collect();
    for ch in channels {
        let mut lp = [0.0f64; 4];
        for &sample in ch {
            let x = sample as f64;
            for i in 0..4 {
                lp[i] += (x - lp[i]) * coefs[i];
            }
            energy[0] += lp[0] * lp[0];
            energy[1] += (lp[1] - lp[0]) * (lp[1] - lp[0]);
            energy[2] += (lp[2] - lp[1]) * (lp[2] - lp[1]);
            energy[3] += (lp[3] - lp[2]) * (lp[3] - lp[2]);
            energy[4] += (x - lp[3]) * (x - lp[3]);
        }
    }
    let total: f64 = energy.iter().sum();
    if total > 0.0 {
        for e in energy.iter_mut() {
            *e /= total;
        }
    }
    energy
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fx(kind: &str, key: &str, v: f64) -> MidiFxConfig {
        MidiFxConfig { kind: kind.into(), params: HashMap::from([(key.to_string(), v)]) }
    }

    #[test]
    fn humanize_does_not_stop_the_midi_chain() {
        // Effects before and after `humanize` must all apply.
        let chain = vec![
            fx("gate", "amount", 0.5),
            fx("ratchet", "count", 2.0),
            fx("humanize", "amount", 0.5),
            fx("transpose", "steps", 3.0),
            fx("velocity", "amount", 0.5),
        ];
        let notes = apply_midi(&chain, 4, 4, 0, 0, 0.8, 16, 0);
        assert_eq!(notes.len(), 2, "ratchet set before humanize must still split the note");
        for n in &notes {
            assert_eq!(n.degree, 3, "transpose after humanize must apply");
            assert!((n.velocity - 0.4).abs() < 1e-9, "velocity after humanize must apply");
        }
        // gate 0.5 on a 1-beat note → 0.5 beat total, starting near beat 1.
        assert!((notes[0].start - 1.0).abs() <= 0.02 + 1e-9);
        assert!(notes[1].end <= notes[0].start + 0.5 + 1e-9);

        // Probability 0 before humanize still drops the note.
        let drop = vec![fx("probability", "amount", 0.0), fx("humanize", "amount", 1.0)];
        assert!(apply_midi(&drop, 4, 1, 0, 0, 0.8, 16, 0).is_empty());
    }
}
