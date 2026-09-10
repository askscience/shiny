//! The Studio render engine.
//!
//! A [`TrackConfig`] (one pattern) is turned into a set of [`Channel`]s — one
//! per voice, each a full instrument plus its insert chain — and rendered in
//! one block-based pass. An [`Arrangement`] renders each clip the same way and
//! mixes it onto the timeline with per-sample level/pan automation.
//!
//! Everything the old `trem`-based engine did by *re-rendering* (device
//! automation, Grid modulation) now happens continuously: parameters are
//! applied to live processors at control rate, so a filter sweep is a real
//! exponential sweep instead of a chain of crossfaded segments. The result is
//! both better sounding and dramatically faster.
//!
//! The JSON contract (config fields, tool inputs, the WAV output) stays
//! backward compatible; new instrument kinds and parameters are additive.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::dsp::drums::Drum;
use crate::dsp::fx::{Effect, EffectKind};
use crate::dsp::limiter::{Compressor, Limiter, MasterBus};
use crate::dsp::loudness;
use crate::dsp::synth::{NoteKind, NoteMsg, Synth};
use crate::dsp::util::{pan_gains, Smoother};
use crate::dsp::SR;
use crate::grid::{GridEngine, GridPatch};
use crate::voices;
use crate::wav::encode_wav;

/// Rendered sample rate (CD rate — the WAV contract every existing row uses).
pub const SAMPLE_RATE: f64 = SR;

/// Extra time rendered past the musical end so releases and reverb tails are
/// not chopped off mid-decay.
const TAIL_SECS: f64 = 0.6;

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
        // Bjorklund's algorithm, inlined (the trem dependency is gone).
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
    for fx in midi {
        match fx.kind.as_str() {
            "transpose" => deg += fx.params.get("steps").copied().unwrap_or(0.0) as i32,
            "velocity" => vel *= fx.params.get("amount").copied().unwrap_or(1.0).clamp(0.0, 2.0),
            "gate" => gate = fx.params.get("amount").copied().unwrap_or(1.0).clamp(0.1, 4.0),
            "ratchet" => ratchet = fx.params.get("count").copied().unwrap_or(2.0).clamp(2.0, 8.0) as u32,
            "probability" | "chance" => probability = fx.params.get("amount").copied().unwrap_or(1.0).clamp(0.0, 1.0),
            "humanize" => {
                // Deterministic pseudo-humanize: a fixed fraction of a step,
                // alternating direction by note index, so renders stay
                // reproducible.
                let amt = fx.params.get("amount").copied().unwrap_or(0.0).clamp(0.0, 1.0);
                let jitter = if (start_step + deg as u32) % 2 == 0 { amt } else { -amt };
                let start_beat = start_step as f64 / 4.0 + jitter * 0.02;
                let dur = (len_steps as f64 / 4.0 * gate).max(0.02);
                return vec![PlannedNote {
                    start: start_beat,
                    end: start_beat + dur,
                    degree: deg,
                    octave: oct,
                    velocity: vel.clamp(0.05, 1.2),
                    pad,
                }];
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
    let start_beat = start_step as f64 / 4.0;
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

/// A note scheduled at an absolute frame, tagged with its channel.
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
}

impl Instrument {
    fn set_param(&mut self, key: &str, value: f64) {
        match self {
            Instrument::Synth(s) => {
                // Parameter changes go through the map so the next control-rate
                // apply picks them up.
                let _ = (s, key, value);
            }
            Instrument::Drum(d) => d.set_param(key, value),
            Instrument::Drumkit(pads) => {
                for d in pads.iter_mut() {
                    d.set_param(key, value);
                }
            }
            Instrument::Grid(_) => {}
        }
    }
}

/// One voice's live processing chain.
struct Channel {
    instrument: Instrument,
    /// The voice's parameter map (kept so `apply_map` can rebuild `Synth`).
    params: HashMap<String, f64>,
    wave: Option<String>,
    kind: String,
    effects: Vec<Effect>,
    level: f32,
    pan: f32,
    /// Smoothed level/pan so automation doesn't zipper.
    level_sm: Smoother,
    pan_sm: Smoother,
    events: Vec<Ev>,
    cursor: usize,
    msgs: Vec<NoteMsg>,
    /// Sample-accurate triggers for one-shot instruments (drums, Grid).
    pending: Vec<(usize, Ev)>,
    scratch_l: [f32; crate::dsp::BLOCK],
    scratch_r: [f32; crate::dsp::BLOCK],
}

impl Channel {
    fn new(v: &VoiceConfig, vi: usize, seed: u64) -> Result<Channel, String> {
        let mut params = voices::kind_params(&v.kind);
        for (k, val) in &v.synth {
            params.insert(k.clone(), *val);
        }
        // Apply macro rack entries over the base parameters.
        apply_macros(&mut params, &v.macros);

        let instrument = if v.kind == "drumkit" {
            let pads = if v.pads.is_empty() { default_pads() } else { v.pads.clone() };
            let mut drums = Vec::with_capacity(pads.len().min(16));
            for (pi, pad) in pads.iter().take(16).enumerate() {
                let mut d = Drum::from_kind(&pad.kind, seed ^ ((vi * 31 + pi) as u64));
                for (k, val) in &pad.params {
                    d.set_param(k, *val);
                }
                drums.push(d);
            }
            Instrument::Drumkit(drums)
        } else if v.kind == "grid" {
            let patch = v.grid.as_ref().ok_or_else(|| "grid voice missing its patch".to_string())?;
            Instrument::Grid(Box::new(GridEngine::compile(patch, seed ^ vi as u64, SAMPLE_RATE)?))
        } else if voices::is_drum(&v.kind) {
            Instrument::Drum(Box::new(Drum::from_kind(&v.kind, seed ^ vi as u64)))
        } else {
            Instrument::Synth(Box::new(Synth::new(&v.kind, v.wave.as_deref(), &params, seed ^ vi as u64, SAMPLE_RATE)))
        };

        let mut effects = Vec::new();
        for e in &v.fx {
            if e.bypass {
                continue;
            }
            match EffectKind::from_name(&e.kind) {
                Some(k) => effects.push(Effect::new(k, &e.params)),
                None => return Err(format!("unknown effect `{}` (one of {})", e.kind, crate::fx::EFFECT_KINDS.join(", "))),
            }
        }

        let level = v.level.unwrap_or_else(|| voices::default_level(&v.kind));
        let pan = v.pan.unwrap_or_else(|| voices::default_pan(&v.kind));
        Ok(Channel {
            instrument,
            params,
            wave: v.wave.clone(),
            kind: v.kind.clone(),
            effects,
            level,
            pan,
            level_sm: Smoother::new(level as f64, 0.005),
            pan_sm: Smoother::new(pan as f64, 0.005),
            events: Vec::new(),
            cursor: 0,
            msgs: Vec::new(),
            pending: Vec::new(),
            scratch_l: [0.0; crate::dsp::BLOCK],
            scratch_r: [0.0; crate::dsp::BLOCK],
        })
    }

    /// Apply a device-automation path relative to this channel.
    fn set_param(&mut self, path: &str, value: f64) {
        if let Some(rest) = path.strip_prefix("fx.") {
            let mut parts = rest.splitn(2, '.');
            let idx = parts.next().and_then(|s| s.parse::<usize>().ok());
            let key = parts.next();
            if let (Some(i), Some(k)) = (idx, key) {
                if let Some(e) = self.effects.get_mut(i) {
                    e.set_param(k, value);
                }
            }
            return;
        }
        match path {
            "level" => {
                self.level = value as f32;
                self.level_sm.snap(value);
            }
            "pan" => {
                self.pan = value as f32;
                self.pan_sm.snap(value);
            }
            "accent" => {}
            _ => match &mut self.instrument {
                Instrument::Synth(s) => {
                    self.params.insert(path.to_string(), value);
                    s.apply_map(&self.params, self.wave.as_deref(), &self.kind);
                }
                other => other.set_param(path, value),
            },
        }
    }

    fn is_active(&self) -> bool {
        if self.cursor < self.events.len() {
            return true;
        }
        match &self.instrument {
            Instrument::Synth(s) => s.active_voices() > 0,
            Instrument::Drum(d) => d.is_active(),
            Instrument::Drumkit(pads) => pads.iter().any(|d| d.is_active()),
            Instrument::Grid(g) => g.is_active(),
        }
    }

    /// Render one block starting at absolute frame `block_start`.
    fn render_block(&mut self, block_start: usize, frames: usize, l: &mut [f32], r: &mut [f32]) {
        let mut n = 0usize;
        self.msgs.clear();
        while self.cursor < self.events.len() && self.events[self.cursor].at < block_start + frames {
            let ev = self.events[self.cursor];
            self.cursor += 1;
            let at = ev.at.saturating_sub(block_start).min(frames.saturating_sub(1));
            if !matches!(self.instrument, Instrument::Synth(_)) {
                // Drums and Grid are triggered directly (sample-accurate below).
                self.pending.push((at, ev));
            } else {
                self.msgs.push(NoteMsg {
                    at,
                    id: ev.id,
                    kind: if ev.on { NoteKind::On { freq: ev.freq, velocity: ev.vel } } else { NoteKind::Off },
                });
            }
            n += 1;
        }
        let _ = n;

        // Instrument output.
        match &mut self.instrument {
            Instrument::Synth(s) => {
                for k in 0..frames {
                    self.scratch_l[k] = 0.0;
                    self.scratch_r[k] = 0.0;
                }
                s.render(&mut self.scratch_l[..frames], &mut self.scratch_r[..frames], &self.msgs, frames, SAMPLE_RATE);
            }
            Instrument::Drum(d) => {
                for k in 0..frames {
                    self.scratch_l[k] = 0.0;
                }
                let mut pi = 0usize;
                for k in 0..frames {
                    while pi < self.pending.len() && self.pending[pi].0 <= k {
                        if self.pending[pi].1.on {
                            d.trigger(self.pending[pi].1.vel);
                        }
                        pi += 1;
                    }
                    self.scratch_l[k] = d.tick(SAMPLE_RATE);
                }
                let l0 = self.scratch_l;
                self.scratch_r[..frames].copy_from_slice(&l0[..frames]);
            }
            Instrument::Drumkit(pads) => {
                for k in 0..frames {
                    self.scratch_l[k] = 0.0;
                }
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
                        v += pad.tick(SAMPLE_RATE);
                    }
                    self.scratch_l[k] = v;
                }
                let l0 = self.scratch_l;
                self.scratch_r[..frames].copy_from_slice(&l0[..frames]);
            }
            Instrument::Grid(g) => {
                for k in 0..frames {
                    self.scratch_l[k] = 0.0;
                    self.scratch_r[k] = 0.0;
                }
                let mut pi = 0usize;
                let mut block_started = false;
                for k in 0..frames {
                    while pi < self.pending.len() && self.pending[pi].0 <= k {
                        if !block_started {
                            // Grid processes whole blocks; notes inside a block
                            // are applied before it renders.
                            block_started = true;
                        }
                        let ev = self.pending[pi].1;
                        if ev.on {
                            g.note_on(ev.freq, ev.vel);
                        } else {
                            g.note_off();
                        }
                        pi += 1;
                    }
                }
                g.render(&mut self.scratch_l[..frames], &mut self.scratch_r[..frames], frames);
            }
        }
        self.pending.clear();

        // Insert chain.
        for e in self.effects.iter_mut() {
            for k in 0..frames {
                let (nl, nr) = e.tick(self.scratch_l[k], self.scratch_r[k]);
                self.scratch_l[k] = nl;
                self.scratch_r[k] = nr;
            }
        }

        // Level / pan (smoothed toward the automated target).
        for k in 0..frames {
            self.level_sm.set_target(self.level as f64);
            self.pan_sm.set_target(self.pan as f64);
            let g = self.level_sm.value() as f32;
            let (pl, pr) = pan_gains(self.pan_sm.value());
            l[k] += self.scratch_l[k] * g * pl;
            r[k] += self.scratch_r[k] * g * pr;
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
            let base = if entry.base != 0.0 {
                entry.base
            } else {
                params.get(&entry.path).copied().unwrap_or(0.0)
            };
            // The range comes from the catalog when we know the parameter, so
            // a macro sweeps a musically meaningful span rather than raw units.
            let range = voices::param_range(&entry.path).unwrap_or(1.0);
            params.insert(entry.path.clone(), base + norm * entry.amount * range * 0.5);
        }
    }
}

/* ═══════════════════════════════════════════════════════════════
   Pattern rendering
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

/// Build the per-voice event lists (absolute frames) for a pattern.
fn build_events(cfg: &TrackConfig) -> Vec<Vec<Ev>> {
    let steps = cfg.steps.max(1) as u32;
    let bpm = cfg.bpm;
    let samples_per_beat = SAMPLE_RATE * 60.0 / bpm;
    let mut out: Vec<Vec<Ev>> = Vec::with_capacity(cfg.voices.len());
    for (vi, v) in cfg.voices.iter().enumerate() {
        let mut evs: Vec<Ev> = Vec::new();
        let mut id = 1u64;
        for note in plan_voice(v, steps) {
            let swing = swing_offset_beats((note.start * 4.0).floor().max(0.0) as u32, cfg.swing);
            let on_beat = note.start + swing;
            let off_beat = note.end + swing;
            let on = (on_beat * samples_per_beat).round().max(0.0) as usize;
            let off = (off_beat * samples_per_beat).round().max(0.0) as usize;
            let is_melodic = voices::is_melodic(&v.kind);
            let freq = if is_melodic {
                resolve_frequency(note.degree, note.octave, &cfg.tuning, cfg.ref_hz)
            } else {
                0.0
            };
            evs.push(Ev { at: on, id, on: true, freq, vel: note.velocity, pad: note.pad });
            // Drums are one-shots: no note-off needed (and an off would cut a
            // ringing cymbal).
            if is_melodic {
                let my_id = id;
                evs.push(Ev { at: off.max(on + 1), id: my_id, on: false, freq, vel: 0.0, pad: note.pad });
            }
            id += 1;
        }
        evs.sort_by_key(|e| (e.at, e.on));
        out.push(evs);
        let _ = vi;
    }
    out
}

/// Build the live channels for a pattern.
fn build_channels(cfg: &TrackConfig, seed: u64) -> Result<Vec<Channel>, String> {
    let mut channels = Vec::with_capacity(cfg.voices.len());
    for (vi, v) in cfg.voices.iter().enumerate() {
        channels.push(Channel::new(v, vi, seed)?);
    }
    let events = build_events(cfg);
    for (c, evs) in channels.iter_mut().zip(events) {
        c.events = evs;
    }
    Ok(channels)
}

/// The pattern-level master chain (delay → reverb → bus glue).
struct MasterChain {
    delay: crate::dsp::delay::StereoDelay,
    reverb: crate::dsp::reverb::Reverb,
    bus: MasterBus,
    comp: Option<Compressor>,
    limiter: Limiter,
}

fn fx_val(fx: &HashMap<String, f64>, key: &str, default: f64) -> f64 {
    fx.get(key).copied().unwrap_or(default).clamp(-100_000.0, 100_000.0)
}

impl MasterChain {
    fn new(fx: &HashMap<String, f64>) -> Self {
        let mut delay = crate::dsp::delay::StereoDelay::new(
            fx_val(fx, "delay_time", 250.0),
            fx_val(fx, "feedback", 0.4),
            fx_val(fx, "delay_mix", 0.0),
        );
        delay.set_sample_rate(SAMPLE_RATE);
        delay.ping_pong = fx_val(fx, "delay_ping_pong", 0.45);
        delay.damp = fx_val(fx, "delay_damp", 0.4);
        delay.update_damping();

        let mut reverb = crate::dsp::reverb::Reverb::new(
            fx_val(fx, "reverb_size", 0.5),
            fx_val(fx, "reverb_damp", 0.5),
            fx_val(fx, "reverb_mix", 0.0),
        );
        reverb.set_sample_rate(SAMPLE_RATE);
        reverb.predelay_ms = fx_val(fx, "reverb_predelay", 12.0);
        reverb.width = fx_val(fx, "reverb_width", 1.0);
        reverb.update_damping();

        let mut bus = MasterBus::new();
        bus.gain = fx_val(fx, "master_gain", 1.0);
        bus.drive = fx_val(fx, "master_drive", 0.0);
        bus.width = fx_val(fx, "master_width", 1.0);

        let comp = if fx_val(fx, "glue", 0.0) > 0.0 {
            let mut c = Compressor::new(-16.0, 2.0, 30.0, 250.0);
            c.knee_db = 8.0;
            c.makeup_db = 1.5;
            c.mix = fx_val(fx, "glue", 0.3).clamp(0.0, 1.0);
            Some(c)
        } else {
            None
        };

        Self {
            delay,
            reverb,
            bus,
            comp,
            limiter: Limiter::new(fx_val(fx, "ceiling", -0.4), 120.0),
        }
    }

    /// Apply a `master.<key>` automation value to the live chain.
    fn set_param(&mut self, key: &str, value: f64) {
        match key {
            "delay_mix" => self.delay.mix = value.clamp(0.0, 1.0),
            "delay_time" => self.delay.time_ms = value.clamp(1.0, 4000.0),
            "feedback" => self.delay.feedback = value.clamp(0.0, 0.98),
            "delay_ping_pong" => self.delay.ping_pong = value.clamp(0.0, 1.0),
            "delay_damp" => {
                self.delay.damp = value.clamp(0.0, 1.0);
                self.delay.update_damping();
            }
            "reverb_mix" => self.reverb.mix = value.clamp(0.0, 1.0),
            "reverb_size" => self.reverb.size = value,
            "reverb_damp" => {
                self.reverb.damping = value.clamp(0.0, 1.0);
                self.reverb.update_damping();
            }
            "reverb_predelay" => self.reverb.predelay_ms = value.clamp(0.0, 500.0),
            "reverb_width" => self.reverb.width = value,
            "master_gain" => self.bus.gain = value.clamp(0.0, 4.0),
            "master_drive" => self.bus.drive = value.clamp(0.0, 1.0),
            "master_width" => self.bus.width = value.clamp(0.0, 2.0),
            "ceiling" => self.limiter.set_ceiling(value),
            _ => {}
        }
    }

    fn tick(&mut self, l: f32, r: f32) -> (f32, f32) {
        let (l, r) = self.delay.tick(l, r);
        let (l, r) = self.reverb.tick(l, r);
        let (l, r) = self.bus.tick(l, r);
        let (l, r) = match &mut self.comp {
            Some(c) => c.tick(l, r),
            None => (l, r),
        };
        self.limiter.tick(l, r)
    }
}

/// Device automation lanes for one clip's track, evaluated at absolute beats.
struct LaneSet<'a> {
    voice_lanes: Vec<(usize, &'a str, &'a AutomationLane, Option<f64>)>,
    master_lanes: Vec<(&'a str, &'a AutomationLane, Option<f64>)>,
}

fn split_lanes<'a>(lanes: &'a [AutomationLane]) -> (LaneSet<'a>, Option<&'a AutomationLane>, Option<&'a AutomationLane>) {
    let mut set = LaneSet { voice_lanes: Vec::new(), master_lanes: Vec::new() };
    let mut level = None;
    let mut pan = None;
    for lane in lanes {
        match lane.param.as_str() {
            "track.level" => level = Some(lane),
            "track.pan" => pan = Some(lane),
            _ => {
                if let Some(rest) = lane.param.strip_prefix("voice.") {
                    let mut parts = rest.splitn(2, '.');
                    if let (Some(vi), Some(rest2)) = (
                        parts.next().and_then(|s| s.parse::<usize>().ok()),
                        parts.next(),
                    ) {
                        set.voice_lanes.push((vi, rest2, lane, None));
                    }
                } else if let Some(k) = lane.param.strip_prefix("master.") {
                    set.master_lanes.push((k, lane, None));
                }
            }
        }
    }
    (set, level, pan)
}

/// Render a pattern into fresh stereo buffers.
///
/// `base_beat` is the arrangement position of the clip's first beat (0 for a
/// standalone pattern); device automation is evaluated at `base_beat + local`.
fn render_pattern(
    cfg: &TrackConfig,
    base_beat: f64,
    lanes: &[AutomationLane],
    seed: u64,
) -> Result<PlanarRender, String> {
    validate(cfg)?;
    let steps = cfg.steps.clamp(4, 64);
    let bpm = cfg.bpm.clamp(40.0, 240.0);
    let samples_per_beat = SAMPLE_RATE * 60.0 / bpm;
    let musical_frames = (steps as f64 / 4.0 * samples_per_beat).ceil() as usize;
    let total = musical_frames + (TAIL_SECS * SAMPLE_RATE) as usize;

    let mut channels = build_channels(cfg, seed)?;
    let (mut lane_set, _, _) = split_lanes(lanes);
    let mut master = MasterChain::new(&cfg.fx);

    let mut mix_l = vec![0.0f32; total];
    let mut mix_r = vec![0.0f32; total];

    let block = crate::dsp::BLOCK;
    let mut start = 0usize;
    while start < total {
        let frames = block.min(total - start);
        // Device automation is applied at control rate. Values are cached so a
        // static lane costs nothing after the first block.
        if !lane_set.voice_lanes.is_empty() || !lane_set.master_lanes.is_empty() {
            let beat = base_beat + start as f64 / samples_per_beat;
            for (vi, path, lane, cached) in lane_set.voice_lanes.iter_mut() {
                let value = envelope_at(&lane.points, beat, f64::NAN);
                if value.is_finite() && cached.map(|c| (c - value).abs() > 1e-9).unwrap_or(true) {
                    if let Some(ch) = channels.get_mut(*vi) {
                        ch.set_param(path, value);
                    }
                    *cached = Some(value);
                }
            }
            for (key, lane, cached) in lane_set.master_lanes.iter_mut() {
                let value = envelope_at(&lane.points, beat, f64::NAN);
                if value.is_finite() && cached.map(|c| (c - value).abs() > 1e-9).unwrap_or(true) {
                    master.set_param(key, value);
                    *cached = Some(value);
                }
            }
        }

        for ch in channels.iter_mut() {
            if !ch.is_active() {
                continue;
            }
            ch.render_block(start, frames, &mut mix_l[start..start + frames], &mut mix_r[start..start + frames]);
        }
        start += frames;
    }

    // Pattern master chain (delay → reverb → glue → limiter).
    let mut out_l = std::mem::take(&mut mix_l);
    let mut out_r = std::mem::take(&mut mix_r);
    for i in 0..total {
        let (l, r) = master.tick(out_l[i], out_r[i]);
        out_l[i] = l;
        out_r[i] = r;
    }

    Ok(PlanarRender {
        sample_rate: SAMPLE_RATE as u32,
        duration_ms: (total as f64 / SAMPLE_RATE * 1000.0).round() as u32,
        frames: total,
        channels: vec![out_l, out_r],
    })
}

/// Compute a min/max peak envelope of a pattern's render (for waveform previews).
pub fn waveform_peaks(cfg: &TrackConfig, buckets: usize) -> Result<Vec<(f32, f32)>, String> {
    let p = render_planar_track(cfg)?;
    Ok(peaks_of(&p, buckets))
}

fn peaks_of(p: &PlanarRender, buckets: usize) -> Vec<(f32, f32)> {
    let n = buckets.clamp(16, 1024);
    let frames = p.frames;
    let per = (frames / n).max(1);
    let mut peaks = Vec::with_capacity(n);
    for b in 0..n {
        let start = b * per;
        let end = ((b + 1) * per).min(frames);
        let mut mn = f32::MAX;
        let mut mx = f32::MIN;
        for ch in &p.channels {
            for i in start..end.min(ch.len()) {
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
    peaks
}

/// Render a standalone pattern (no automation).
pub fn render_planar_track(cfg: &TrackConfig) -> Result<PlanarRender, String> {
    render_pattern(cfg, 0.0, &[], 0x51D_0001)
}

/// Validate and render a single pattern to WAV, applying the loudness target
/// from `cfg.fx.loudness` when set.
pub fn render_track(cfg: &TrackConfig) -> Result<Rendered, String> {
    let mut p = render_planar_track(cfg)?;
    normalize(&mut p, &cfg.fx);
    let report = loudness::measure(&p.channels, p.sample_rate as f64);
    let wav = encode_wav(&p.channels, p.sample_rate);
    Ok(Rendered {
        sample_rate: p.sample_rate,
        channels: 2,
        duration_ms: p.duration_ms,
        wav,
        lufs: report.integrated_lufs,
        peak: report.peak,
    })
}

/// Apply the requested loudness target: measure, trim and re-limit.
fn normalize(p: &mut PlanarRender, fx: &HashMap<String, f64>) {
    let target = fx_val(fx, "loudness", 0.0);
    if target <= -60.0 || target == 0.0 {
        return;
    }
    let report = loudness::measure(&p.channels, p.sample_rate as f64);
    if !report.integrated_lufs.is_finite() || report.integrated_lufs < -69.0 {
        return;
    }
    let gain = loudness::normalization_gain(report.integrated_lufs, target).clamp(0.05, 8.0) as f32;
    if (gain - 1.0).abs() < 0.01 {
        return;
    }
    for ch in p.channels.iter_mut() {
        for s in ch.iter_mut() {
            *s = (*s * gain).clamp(-1.0, 1.0);
        }
    }
}

/* ═══════════════════════════════════════════════════════════════
   Arrangement
   ═══════════════════════════════════════════════════════════════ */

/// One arrangement lane (a horizontal row in the DAW).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ArrangementTrack {
    pub id: String,
    #[serde(default = "default_track_name")]
    pub name: String,
    pub color: u32,
    pub mute: bool,
    /// Solo is honoured by the renderer as well as the UI.
    pub solo: bool,
    #[serde(default = "default_track_level")]
    pub level: f32,
    pub pan: f32,
    /// Per-track automation envelopes.
    #[serde(default)]
    pub automation: TrackAutomation,
}

impl Default for ArrangementTrack {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: "Track".into(),
            color: 0,
            mute: false,
            solo: false,
            level: 0.8,
            pan: 0.0,
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
    /// Target path: `track.level` | `track.pan` (post, per-sample) or a
    /// device path — `voice.<i>.<param>` | `voice.<i>.level` | `voice.<i>.pan`
    /// | `voice.<i>.fx.<j>.<key>` | `master.<key>`.
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
    /// Clip length in beats; `None` uses the pattern's own length.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length_beats: Option<f64>,
    /// Gain in dB applied to this clip (clip gain).
    #[serde(default)]
    pub gain_db: f64,
    pub pattern: TrackConfig,
}

impl Default for ArrangementClip {
    fn default() -> Self {
        Self { track: String::new(), start: 0.0, length_beats: None, gain_db: 0.0, pattern: TrackConfig::default() }
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
    /// Master bus options (loudness target, glue, ceiling, width).
    #[serde(default)]
    pub fx: HashMap<String, f64>,
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
            tracks: Vec::new(),
            clips: Vec::new(),
        }
    }
}

/// Parse an arrangement from JSON.
pub fn parse_arrangement(value: &serde_json::Value) -> Result<Arrangement, String> {
    let mut a: Arrangement =
        serde_json::from_value(value.clone()).map_err(|e| format!("invalid arrangement: {e}"))?;
    a.bpm = a.bpm.clamp(40.0, 240.0);
    a.length_beats = a.length_beats.clamp(4.0, 512.0);
    // `envelope_at` assumes breakpoints are ordered by beat — the model
    // sometimes emits them out of order, which silently breaks interpolation.
    for track in &mut a.tracks {
        for lane in &mut track.automation.lanes {
            lane.points
                .sort_by(|x, y| x.beat.partial_cmp(&y.beat).unwrap_or(std::cmp::Ordering::Equal));
        }
    }
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
    if a.tracks.len() > 32 {
        return Err("too many tracks (max 32)".into());
    }
    if a.clips.len() > 256 {
        return Err("too many clips (max 256)".into());
    }

    let bpm = a.bpm.clamp(40.0, 240.0);
    let length_beats = a.length_beats.clamp(4.0, 512.0);
    let total = (length_beats * 60.0 / bpm * SAMPLE_RATE).ceil() as usize + (TAIL_SECS * SAMPLE_RATE) as usize;
    let any_solo = a.tracks.iter().any(|t| t.solo);

    // Build the list of audible clips with their track automation.
    struct Job<'a> {
        clip: &'a ArrangementClip,
        lanes: &'a [AutomationLane],
        seed: u64,
    }
    let mut jobs: Vec<Job> = Vec::new();
    for (ci, clip) in a.clips.iter().enumerate() {
        let Some(track) = a.tracks.iter().find(|t| t.id == clip.track) else {
            return Err(format!("clip references unknown track `{}`", clip.track));
        };
        if track.mute || (any_solo && !track.solo) {
            continue;
        }
        jobs.push(Job { clip, lanes: &track.automation.lanes, seed: 0x51D_1000 + ci as u64 * 7919 });
    }

    // Render clips — in parallel across cores. Each clip owns its buffers, so
    // the workers touch disjoint memory and the result order stays stable.
    let workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).min(8);
    let mut rendered: Vec<Result<(Vec<f32>, Vec<f32>), String>> =
        (0..jobs.len()).map(|_| Err("clip not rendered".to_string())).collect();
    if jobs.len() <= 1 || workers <= 1 {
        for (j, slot) in jobs.iter().zip(rendered.iter_mut()) {
            *slot = render_clip_with_lanes(j.clip, bpm, j.lanes, j.seed);
        }
    } else {
        let chunk = (jobs.len() + workers - 1) / workers;
        std::thread::scope(|scope| {
            for (job_chunk, out_chunk) in jobs.chunks(chunk).zip(rendered.chunks_mut(chunk)) {
                scope.spawn(move || {
                    for (j, slot) in job_chunk.iter().zip(out_chunk.iter_mut()) {
                        *slot = render_clip_with_lanes(j.clip, bpm, j.lanes, j.seed);
                    }
                });
            }
        });
    }

    let mut mix_l = vec![0.0f32; total];
    let mut mix_r = vec![0.0f32; total];
    let samples_per_beat = SAMPLE_RATE * 60.0 / bpm;

    for (j, res) in jobs.iter().zip(rendered.into_iter()) {
        let (cl, cr) = res?;
        let track = a.tracks.iter().find(|t| t.id == j.clip.track).expect("checked above");
        let (_, level_lane, pan_lane) = split_lanes(j.lanes);
        let offset = (j.clip.start * samples_per_beat).round() as i64;
        let base_level = track.level.clamp(0.0, 2.0) as f64;
        let base_pan = track.pan.clamp(-1.0, 1.0) as f64;
        for i in 0..cl.len() {
            let pos = offset + i as i64;
            if pos < 0 || pos as usize >= total {
                continue;
            }
            let beat = pos as f64 / samples_per_beat;
            let level = level_lane
                .map(|l| envelope_at(&l.points, beat, base_level))
                .unwrap_or(base_level)
                .clamp(0.0, 2.0);
            let pan = pan_lane
                .map(|l| envelope_at(&l.points, beat, base_pan))
                .unwrap_or(base_pan)
                .clamp(-1.0, 1.0);
            let (gl, gr) = pan_gains(pan);
            let idx = pos as usize;
            mix_l[idx] += cl[i] * level as f32 * gl;
            mix_r[idx] += cr[i] * level as f32 * gr;
        }
    }

    // Global master chain: bus gain/width/glue, then the brickwall limiter.
    let mut fx = a.fx.clone();
    fx.insert("master_gain".into(), a.master as f64);
    let mut master = MasterChain::new(&fx);
    let mut out_l = vec![0.0f32; total];
    let mut out_r = vec![0.0f32; total];
    for i in 0..total {
        let (l, r) = master.tick(mix_l[i], mix_r[i]);
        out_l[i] = l;
        out_r[i] = r;
    }

    let mut p = PlanarRender {
        sample_rate: SAMPLE_RATE as u32,
        duration_ms: (total as f64 / SAMPLE_RATE * 1000.0).round() as u32,
        frames: total,
        channels: vec![out_l, out_r],
    };
    normalize(&mut p, &fx);
    let report = loudness::measure(&p.channels, p.sample_rate as f64);
    let wav = encode_wav(&p.channels, p.sample_rate);
    Ok(Rendered {
        sample_rate: p.sample_rate,
        channels: 2,
        duration_ms: p.duration_ms,
        wav,
        lufs: report.integrated_lufs,
        peak: report.peak,
    })
}

fn render_clip_with_lanes(clip: &ArrangementClip, bpm: f64, lanes: &[AutomationLane], seed: u64) -> Result<(Vec<f32>, Vec<f32>), String> {
    let mut cfg = clip.pattern.clone();
    cfg.bpm = bpm;
    let p = render_pattern(&cfg, clip.start, lanes, seed)?;
    let mut l = p.channels.first().cloned().unwrap_or_default();
    let mut r = p.channels.get(1).cloned().unwrap_or_else(|| l.clone());
    if clip.gain_db != 0.0 {
        let g = crate::dsp::util::db_to_gain(clip.gain_db) as f32;
        for s in l.iter_mut() {
            *s *= g;
        }
        for s in r.iter_mut() {
            *s *= g;
        }
    }
    Ok((l, r))
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

fn analyze_planar(p: &PlanarRender) -> Analysis {
    let report = loudness::measure(&p.channels, p.sample_rate as f64);
    let bands = band_energy(&p.channels, p.sample_rate as f64);
    Analysis {
        duration_ms: p.duration_ms,
        lufs: report.integrated_lufs,
        peak: report.peak,
        rms_db: report.rms_db,
        crest_db: report.crest_db,
        clipped: report.clipped,
        range_lu: report.range_lu,
        bands,
        channels: p.channels.len() as u32,
    }
}

/// Coarse 5-band energy split using cascaded one-pole filters — enough for an
/// LLM to reason about "too much sub" or "too bright".
fn band_energy(channels: &[Vec<f32>], sr: f64) -> [f64; 5] {
    const SPLITS: [f64; 4] = [120.0, 500.0, 2000.0, 6000.0];
    let mut energy = [0.0f64; 5];
    for ch in channels {
        let coefs: Vec<f64> =
            SPLITS.iter().map(|f| 1.0 - (-2.0 * std::f64::consts::PI * f / sr).exp()).collect();
        let mut lp = [0.0f64; 4];
        for &sample in ch {
            let x = sample as f64;
            for i in 0..4 {
                lp[i] += (x - lp[i]) * coefs[i];
            }
            // Band i is the difference between neighbouring low-pass taps;
            // the top band is whatever is left above the last split.
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
    use serde_json::json;

    fn kit(steps: u32) -> TrackConfig {
        TrackConfig { steps, ..TrackConfig::default() }
    }

    #[test]
    fn renders_a_kit() {
        let cfg = TrackConfig {
            bpm: 120.0,
            steps: 16,
            voices: vec![
                VoiceConfig { kind: "kick".into(), rhythm: "e4,0".into(), ..VoiceConfig::default() },
                VoiceConfig { kind: "hat".into(), rhythm: "e8,2".into(), ..VoiceConfig::default() },
                VoiceConfig { kind: "snare".into(), rhythm: "e4,8".into(), ..VoiceConfig::default() },
                VoiceConfig {
                    kind: "bass".into(),
                    rhythm: "x.x.x.x".into(),
                    degree: 0,
                    octave: 2,
                    wave: Some("triangle".into()),
                    ..VoiceConfig::default()
                },
            ],
            ..TrackConfig::default()
        };
        let out = render_track(&cfg).expect("render should succeed");
        assert_eq!(out.channels, 2);
        assert!(out.duration_ms > 500);
        assert_eq!(&out.wav[0..4], b"RIFF");
        assert!(out.peak > 0.01, "kit should be audible, peak = {}", out.peak);
        assert!(out.lufs.is_finite());
    }

    fn energy_of(out: &Rendered) -> i64 {
        out.wav[44..]
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]) as i64)
            .map(|s| s * s)
            .sum()
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
        assert!(energy_of(&out) > 0);
    }

    #[test]
    fn every_kind_renders_audible_audio() {
        for kind in voices::KINDS {
            if *kind == "grid" {
                continue; // covered separately
            }
            let cfg = TrackConfig {
                steps: 8,
                voices: vec![VoiceConfig {
                    kind: (*kind).into(),
                    rhythm: "x.x.x.x.".into(),
                    degree: 0,
                    octave: 3,
                    ..VoiceConfig::default()
                }],
                ..TrackConfig::default()
            };
            let out = render_track(&cfg).unwrap_or_else(|e| panic!("{kind} failed: {e}"));
            assert!(out.peak > 0.002, "{kind} rendered silence (peak {})", out.peak);
        }
    }

    #[test]
    fn polyphony_produces_chords() {
        // Three simultaneous notes must be louder than one.
        let one = TrackConfig {
            steps: 8,
            voices: vec![VoiceConfig {
                kind: "pad".into(),
                rhythm: "".into(),
                notes: vec![NoteOverride { step: 0, length: 4, degree: 0, octave: 3, velocity: None }],
                ..VoiceConfig::default()
            }],
            ..TrackConfig::default()
        };
        let three = TrackConfig {
            steps: 8,
            voices: vec![VoiceConfig {
                kind: "pad".into(),
                rhythm: "".into(),
                notes: vec![
                    NoteOverride { step: 0, length: 4, degree: 0, octave: 3, velocity: None },
                    NoteOverride { step: 0, length: 4, degree: 2, octave: 3, velocity: None },
                    NoteOverride { step: 0, length: 4, degree: 4, octave: 3, velocity: None },
                ],
                ..VoiceConfig::default()
            }],
            ..TrackConfig::default()
        };
        let a = render_planar_track(&one).unwrap();
        let b = render_planar_track(&three).unwrap();
        let rms = |p: &PlanarRender| -> f64 {
            let mut sum = 0.0;
            let mut n = 0;
            for ch in &p.channels {
                for s in ch {
                    sum += (*s as f64) * (*s as f64);
                    n += 1;
                }
            }
            (sum / n.max(1) as f64).sqrt()
        };
        assert!(rms(&b) > rms(&a) * 1.15, "chord should be fuller: {} vs {}", rms(&b), rms(&a));
    }

    #[test]
    fn kick_has_more_low_end_than_hat() {
        let k = analyze(&TrackConfig {
            steps: 4,
            voices: vec![VoiceConfig { kind: "kick".into(), rhythm: "x...".into(), ..VoiceConfig::default() }],
            ..TrackConfig::default()
        })
        .unwrap();
        let h = analyze(&TrackConfig {
            steps: 4,
            voices: vec![VoiceConfig { kind: "hat".into(), rhythm: "xxxx".into(), ..VoiceConfig::default() }],
            ..TrackConfig::default()
        })
        .unwrap();
        assert!(k.bands[0] > h.bands[0], "kick should be sub-heavier: {:?} vs {:?}", k.bands, h.bands);
        assert!(h.bands[4] > k.bands[4], "hat should be airier: {:?} vs {:?}", h.bands, k.bands);
    }

    #[test]
    fn grid_patch_renders_with_modulation() {
        use crate::grid::{GridCable, GridModule, GridPatch};
        let p = |_k: &str| -> HashMap<String, f64> { HashMap::new() };
        let mut patch = GridPatch::default();
        patch.modules = vec![
            GridModule { id: "o".into(), kind: "osc".into(), params: p("o") },
            GridModule { id: "f".into(), kind: "filter".into(), params: p("f") },
            GridModule { id: "e".into(), kind: "env".into(), params: p("e") },
            GridModule {
                id: "l".into(),
                kind: "lfo".into(),
                params: [("rate".to_string(), 2.0), ("depth".to_string(), 0.5)].into_iter().collect(),
            },
            GridModule { id: "out".into(), kind: "out".into(), params: p("out") },
        ];
        patch.cables = vec![
            GridCable { from: ("o".into(), "out".into()), to: ("f".into(), "in".into()), amount: None },
            GridCable { from: ("f".into(), "out".into()), to: ("e".into(), "in".into()), amount: None },
            GridCable { from: ("e".into(), "out".into()), to: ("out".into(), "in".into()), amount: None },
            GridCable { from: ("l".into(), "ctrl".into()), to: ("f".into(), "mod".into()), amount: Some(0.5) },
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
        assert!(energy_of(&out) > 0, "grid patch should render non-zero audio");
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
            voices: vec![VoiceConfig {
                kind: "bass".into(),
                rhythm: "x.x.x.x".into(),
                degree: 0,
                octave: 2,
                ..VoiceConfig::default()
            }],
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
                ArrangementClip { track: "t0".into(), start: 0.0, pattern: kit, ..ArrangementClip::default() },
                ArrangementClip { track: "t1".into(), start: 0.0, pattern: bass, ..ArrangementClip::default() },
            ],
            ..Arrangement::default()
        };
        let out = render_arrangement(&a).expect("arrangement should render");
        assert_eq!(&out.wav[0..4], b"RIFF");
        assert!(out.duration_ms >= 8000, "16 beats @120bpm = 8s, got {}", out.duration_ms);
    }

    #[test]
    fn arrangement_honours_solo_and_mute() {
        let pattern = TrackConfig {
            steps: 8,
            voices: vec![VoiceConfig { kind: "kick".into(), rhythm: "e4,0".into(), ..VoiceConfig::default() }],
            ..TrackConfig::default()
        };
        let mk = |mute: bool, solo: bool| Arrangement {
            bpm: 120.0,
            length_beats: 8.0,
            tracks: vec![ArrangementTrack {
                id: "t0".into(),
                name: "Drums".into(),
                mute,
                solo,
                ..ArrangementTrack::default()
            }],
            clips: vec![ArrangementClip { track: "t0".into(), start: 0.0, pattern: pattern.clone(), ..ArrangementClip::default() }],
            ..Arrangement::default()
        };
        let muted = render_arrangement(&mk(true, false)).unwrap();
        let audible = render_arrangement(&mk(false, false)).unwrap();
        assert!(energy_of(&muted) < energy_of(&audible) / 100, "mute must silence the track");
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
            clips: vec![ArrangementClip { track: "t0".into(), start: 0.0, pattern: kit, ..ArrangementClip::default() }],
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
        let mut synth = HashMap::new();
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
            clips: vec![ArrangementClip { track: "t0".into(), start: 0.0, pattern: bass, ..ArrangementClip::default() }],
            ..Arrangement::default()
        };
        let out = render_arrangement(&a).expect("device automation should render");
        assert!(energy_of(&out) > 0);
    }

    #[test]
    fn arrangement_rejects_bad_clip_track() {
        let a = Arrangement {
            tracks: vec![ArrangementTrack { id: "t0".into(), ..ArrangementTrack::default() }],
            clips: vec![ArrangementClip {
                track: "missing".into(),
                start: 0.0,
                pattern: TrackConfig::default(),
                ..ArrangementClip::default()
            }],
            ..Arrangement::default()
        };
        assert!(render_arrangement(&a).is_err());
    }

    #[test]
    fn euclidean_hits_count() {
        assert_eq!(rhythm_hits("e5,0", 16).len(), 5);
        assert_eq!(rhythm_hits("x.x", 16), vec![0, 2]);
        for h in 1..=16u32 {
            assert_eq!(euclidean(h, 16).iter().filter(|b| **b).count(), h as usize, "h={h}");
        }
    }

    #[test]
    fn note_duration_renders() {
        let cfg = TrackConfig {
            steps: 16,
            voices: vec![VoiceConfig {
                kind: "lead".into(),
                rhythm: "".into(),
                notes: vec![NoteOverride { step: 0, length: 8, degree: 4, octave: 3, velocity: None }],
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

    #[test]
    fn swing_shifts_odd_steps_only() {
        let mk = |rhythm: &str, swing: f64| TrackConfig {
            steps: 4,
            swing,
            voices: vec![VoiceConfig { kind: "hat".into(), rhythm: rhythm.into(), ..VoiceConfig::default() }],
            ..TrackConfig::default()
        };
        let straight_odd = render_track(&mk(".x..", 0.0)).unwrap();
        let swung_odd = render_track(&mk(".x..", 1.0)).unwrap();
        assert_ne!(straight_odd.wav, swung_odd.wav, "odd-step hit must shift with swing");
        let straight_even = render_track(&mk("x...", 0.0)).unwrap();
        let swung_even = render_track(&mk("x...", 1.0)).unwrap();
        assert_eq!(straight_even.wav, swung_even.wav, "even-step hit must not move");
    }

    #[test]
    fn accent_changes_velocity_loudness() {
        let mk = |accent: Option<f64>| TrackConfig {
            steps: 8,
            voices: vec![VoiceConfig {
                kind: "hat".into(),
                rhythm: "x.x.x.x.".into(),
                accent,
                ..VoiceConfig::default()
            }],
            ..TrackConfig::default()
        };
        let flat = render_track(&mk(None)).unwrap();
        let accented = render_track(&mk(Some(0.5))).unwrap();
        assert_ne!(flat.wav, accented.wav, "accent must change the render");
    }

    #[test]
    fn swing_parses_from_json() {
        let cfg = parse_config(&json!({ "steps": 8, "swing": 2.0 })).unwrap();
        assert_eq!(cfg.swing, 1.0, "swing clamps to 1");
        let cfg = parse_config(&json!({ "steps": 8 })).unwrap();
        assert_eq!(cfg.swing, 0.0, "swing defaults to straight");
    }

    #[test]
    fn macros_reach_the_engine() {
        // A macro on cutoff must change the render even with no UI involved.
        let macros = vec![MacroConfig {
            value: 1.0,
            entries: vec![MacroAssignment { path: "cutoff".into(), amount: 1.0, base: 300.0 }],
        }];
        let mk = |macros: Vec<MacroConfig>| TrackConfig {
            steps: 8,
            voices: vec![VoiceConfig {
                kind: "lead".into(),
                rhythm: "x.x.x.x.".into(),
                octave: 3,
                macros,
                ..VoiceConfig::default()
            }],
            ..TrackConfig::default()
        };
        let off = render_track(&mk(Vec::new())).unwrap();
        let on = render_track(&mk(macros)).unwrap();
        assert_ne!(off.wav, on.wav, "macro must affect the render");
    }

    #[test]
    fn loudness_target_is_honoured() {
        let mut fx = HashMap::new();
        fx.insert("loudness".to_string(), -18.0);
        let cfg = TrackConfig {
            steps: 16,
            voices: vec![VoiceConfig { kind: "kick".into(), rhythm: "e4,0".into(), ..VoiceConfig::default() }],
            fx,
            ..TrackConfig::default()
        };
        let out = render_track(&cfg).unwrap();
        assert!((out.lufs - (-18.0)).abs() < 2.5, "expected ≈ -18 LUFS, got {}", out.lufs);
    }

    #[test]
    fn tuning_systems_resolve_sensibly() {
        // An octave up is always exactly double in every supported tuning when
        // the degree count spans one octave.
        for (tuning, oct) in [("edo12", 12), ("edo19", 19), ("edo24", 24), ("edo31", 31)] {
            let a = resolve_frequency(0, 0, tuning, 440.0);
            let b = resolve_frequency(oct, 0, tuning, 440.0);
            assert!((b / a - 2.0).abs() < 1e-9, "{tuning}: {a} -> {b}");
        }
        assert!((resolve_frequency(0, 1, "edo12", 440.0) - 880.0).abs() < 1e-9);
    }

    #[test]
    fn performance_budget() {
        // A busy 16-step pattern must render far faster than realtime.
        let cfg = kit(16);
        let start = std::time::Instant::now();
        let _ = render_planar_track(&cfg).unwrap();
        let elapsed = start.elapsed();
        assert!(elapsed.as_secs_f64() < 2.0, "kit render took {elapsed:?}");
    }
}
