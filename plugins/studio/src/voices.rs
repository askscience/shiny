//! Instrument catalog: the kinds the engine understands, their default
//! parameters, and the self-describing parameter lists the UI and the AI both
//! read.
//!
//! A "kind" is just a preset for the shared voice engine ([`crate::dsp::synth`])
//! or a drum model ([`crate::dsp::drums`]). Keeping the catalog declarative
//! means the REST/`studio_catalog` surface and the Studio device panels are
//! generated from exactly the values the renderer uses — no more drift between
//! the JS tables and the Rust defaults.

use std::collections::HashMap;

use crate::dsp::synth::SynthParams;

/// Voice kinds the Studio engine understands, in UI order.
pub const KINDS: &[&str] = &[
    // Drums
    "kick", "snare", "hat", "clap", "tom", "perc", "rim", "cowbell", "shaker", "crash", "ride",
    // Pitched
    "bass", "sub", "pluck", "lead", "pad", "organ", "ep", "bell", "strings", "brass", "synthme", "fm",
    // Collections
    "drumkit", "grid",
];

/// Drum-machine kinds (one-shot models).
pub const DRUM_KINDS: &[&str] = &[
    "kick", "snare", "hat", "clap", "tom", "perc", "rim", "cowbell", "shaker", "crash", "ride",
];

/// Melodic kinds (the shared subtractive/FM voice engine).
pub const MELODIC_KINDS: &[&str] = &[
    "bass", "sub", "pluck", "lead", "pad", "organ", "ep", "bell", "strings", "brass", "synthme", "fm",
];

/// Maximum voices in one pattern — polyphony multiplies CPU, so this is a
/// deliberate ceiling rather than an arbitrary one.
pub const MAX_VOICES: usize = 16;

pub fn is_kind(kind: &str) -> bool {
    KINDS.contains(&kind)
}

pub fn is_drum(kind: &str) -> bool {
    DRUM_KINDS.contains(&kind)
}

pub fn is_melodic(kind: &str) -> bool {
    MELODIC_KINDS.contains(&kind)
}

/// A self-describing parameter (mirrors the JSON `synth` map).
#[derive(Debug, Clone, Copy)]
pub struct ParamDef {
    pub key: &'static str,
    pub label: &'static str,
    pub group: &'static str,
    pub min: f64,
    pub max: f64,
    pub step: f64,
    pub default: f64,
    /// Hz-scale parameters are shown logarithmically by the UI.
    pub log: bool,
    /// Discrete parameters render as a dropdown of these labels (if any).
    pub choices: &'static [&'static str],
}

pub const fn def(
    key: &'static str,
    label: &'static str,
    group: &'static str,
    min: f64,
    max: f64,
    step: f64,
    default: f64,
) -> ParamDef {
    ParamDef { key, label, group, min, max, step, default, log: false, choices: &[] }
}

pub const fn logdef(
    key: &'static str,
    label: &'static str,
    group: &'static str,
    min: f64,
    max: f64,
    step: f64,
    default: f64,
) -> ParamDef {
    ParamDef { key, label, group, min, max, step, default, log: true, choices: &[] }
}

pub const fn choice(
    key: &'static str,
    label: &'static str,
    group: &'static str,
    default: f64,
    choices: &'static [&'static str],
) -> ParamDef {
    ParamDef {
        key,
        label,
        group,
        min: 0.0,
        max: (choices.len().saturating_sub(1)) as f64,
        step: 1.0,
        default,
        log: false,
        choices,
    }
}

const WAVES: &[&str] = &["Sine", "Triangle", "Saw", "Square", "Pulse", "Noise", "Organ"];
const FILTER_TYPES: &[&str] = &["LP", "HP", "BP", "Notch", "Peak"];
const LFO_WAVES: &[&str] = &["Sine", "Triangle", "Saw", "Ramp", "Square", "S&H", "Random"];

/* ── Drum parameter catalogs ─────────────────────────────────── */

static KICK: &[ParamDef] = &[
    def("pitch", "Pitch", "Drum", 20.0, 200.0, 1.0, 50.0),
    def("decay", "Decay", "Drum", 0.5, 40.0, 0.1, 8.0),
    def("sweep", "Sweep", "Drum", 0.5, 120.0, 0.5, 30.0),
    def("click", "Click", "Drum", 0.0, 1.0, 0.01, 0.22),
    def("drive", "Drive", "Drum", 0.0, 1.0, 0.01, 0.3),
    def("hold", "Hold", "Drum", 0.0, 0.2, 0.001, 0.004),
];
static SNARE: &[ParamDef] = &[
    def("tone", "Tone", "Drum", 80.0, 500.0, 1.0, 200.0),
    def("body", "Body", "Drum", 1.0, 80.0, 0.1, 25.0),
    def("noise", "Noise", "Drum", 1.0, 80.0, 0.1, 15.0),
    def("snap", "Snap", "Drum", 0.0, 1.0, 0.01, 0.5),
    def("buzz", "Buzz", "Drum", 0.0, 1.0, 0.01, 0.15),
    def("drive", "Drive", "Drum", 0.0, 1.0, 0.01, 0.2),
];
static HAT: &[ParamDef] = &[
    def("decay", "Decay", "Drum", 2.0, 200.0, 0.5, 40.0),
    logdef("tone", "Tone", "Drum", 2000.0, 14000.0, 10.0, 9000.0),
    def("metal", "Metal", "Drum", 0.0, 1.0, 0.01, 0.8),
    def("drive", "Drive", "Drum", 0.0, 1.0, 0.01, 0.1),
];
static CLAP: &[ParamDef] = &[
    logdef("tone", "Tone", "Drum", 400.0, 4000.0, 10.0, 1400.0),
    def("body", "Body", "Drum", 1.0, 60.0, 0.1, 10.0),
    def("noise", "Noise", "Drum", 1.0, 60.0, 0.1, 35.0),
    def("spread", "Spread", "Drum", 0.0, 1.0, 0.01, 0.5),
];
static TOM: &[ParamDef] = &[
    def("pitch", "Pitch", "Drum", 40.0, 500.0, 1.0, 150.0),
    def("decay", "Decay", "Drum", 0.5, 60.0, 0.1, 20.0),
    def("sweep", "Sweep", "Drum", 0.5, 150.0, 0.5, 50.0),
    def("noise", "Noise", "Drum", 0.0, 1.0, 0.01, 0.3),
];
static PERC: &[ParamDef] = &[
    def("decay", "Decay", "Drum", 4.0, 300.0, 0.5, 60.0),
    logdef("tone", "Tone", "Drum", 200.0, 8000.0, 10.0, 2500.0),
    def("metal", "Metal", "Drum", 0.0, 1.0, 0.01, 0.5),
];
static RIM: &[ParamDef] = &[
    def("decay", "Decay", "Drum", 4.0, 200.0, 0.5, 90.0),
    logdef("tone", "Tone", "Drum", 200.0, 6000.0, 10.0, 1700.0),
];
static COWBELL: &[ParamDef] = &[
    def("decay", "Decay", "Drum", 4.0, 200.0, 0.5, 30.0),
    logdef("tone", "Tone", "Drum", 200.0, 3000.0, 10.0, 540.0),
];
static SHAKER: &[ParamDef] = &[
    def("decay", "Decay", "Drum", 4.0, 300.0, 0.5, 40.0),
    logdef("tone", "Tone", "Drum", 2000.0, 14000.0, 10.0, 6000.0),
];
static CRASH: &[ParamDef] = &[
    def("decay", "Decay", "Drum", 0.5, 30.0, 0.01, 1.6),
    logdef("tone", "Tone", "Drum", 2000.0, 14000.0, 10.0, 5000.0),
];
static RIDE: &[ParamDef] = &[
    def("decay", "Decay", "Drum", 0.5, 30.0, 0.01, 2.2),
    logdef("tone", "Tone", "Drum", 2000.0, 14000.0, 10.0, 4500.0),
    def("bell", "Bell", "Drum", 0.0, 1.0, 0.01, 0.4),
];

/* ── Shared synth parameter catalog ──────────────────────────── */

/// The parameter list every melodic kind shares. Kinds differ only in their
/// *defaults*, so one editor (and one set of automation targets) serves them
/// all — and the AI can drive any instrument through the same keys.
static MELODIC_PARAMS: &[ParamDef] = &[
    // Oscillator
    choice("o1w", "Osc 1", "Oscillator", 2.0, WAVES),
    choice("o2w", "Osc 2", "Oscillator", 3.0, WAVES),
    def("a_level", "Osc 1 Lvl", "Oscillator", 0.0, 1.5, 0.01, 0.7),
    def("b_level", "Osc 2 Lvl", "Oscillator", 0.0, 1.5, 0.01, 0.0),
    def("b_semi", "Detune", "Oscillator", -24.0, 24.0, 0.1, 0.0),
    def("b_octave", "Octave", "Oscillator", -3.0, 3.0, 1.0, 0.0),
    def("pw", "Pulse Width", "Oscillator", 0.05, 0.95, 0.01, 0.5),
    def("unison", "Unison", "Oscillator", 1.0, 8.0, 1.0, 1.0),
    def("spread", "Uni Spread", "Oscillator", 0.0, 1.0, 0.01, 0.5),
    def("sub", "Sub", "Oscillator", 0.0, 1.0, 0.01, 0.0),
    def("noise", "Noise", "Oscillator", 0.0, 1.0, 0.01, 0.0),
    def("ring", "Ring Mod", "Oscillator", 0.0, 1.0, 0.01, 0.0),
    def("fm_ratio", "FM Ratio", "Oscillator", 0.25, 12.0, 0.01, 2.0),
    def("fm_index", "FM Index", "Oscillator", 0.0, 10.0, 0.01, 0.0),
    // Filter
    choice("ftype", "Type", "Filter", 0.0, FILTER_TYPES),
    logdef("cutoff", "Cutoff", "Filter", 20.0, 20000.0, 10.0, 3000.0),
    def("res", "Resonance", "Filter", 0.4, 20.0, 0.1, 1.0),
    def("drive", "Drive", "Filter", 0.25, 24.0, 0.05, 1.0),
    def("poles", "Slope", "Filter", 1.0, 2.0, 1.0, 1.0),
    def("fenv", "Env Amount", "Filter", -6.0, 6.0, 0.05, 0.0),
    def("keytrack", "Key Track", "Filter", 0.0, 1.0, 0.01, 0.35),
    def("fattack", "F. Attack", "Filter", 0.001, 5.0, 0.005, 0.005),
    def("fdecay", "F. Decay", "Filter", 0.001, 5.0, 0.01, 0.3),
    def("fsustain", "F. Sustain", "Filter", 0.0, 1.0, 0.01, 0.4),
    def("frelease", "F. Release", "Filter", 0.001, 5.0, 0.01, 0.3),
    // Envelope
    def("attack", "Attack", "Envelope", 0.001, 5.0, 0.005, 0.005),
    def("decay", "Decay", "Envelope", 0.001, 5.0, 0.01, 0.2),
    def("sustain", "Sustain", "Envelope", 0.0, 1.0, 0.01, 0.7),
    def("release", "Release", "Envelope", 0.001, 5.0, 0.01, 0.2),
    def("env_shape", "Curve", "Envelope", 0.0, 1.0, 0.01, 0.0),
    def("vel_amp", "Vel to Amp", "Envelope", 0.0, 1.0, 0.01, 0.7),
    def("vel_filter", "Vel to Filter", "Envelope", 0.0, 1.0, 0.01, 0.4),
    // LFO
    def("lfo_rate", "Rate", "LFO", 0.01, 40.0, 0.01, 3.0),
    choice("lfo_wave", "Shape", "LFO", 0.0, LFO_WAVES),
    def("lfo_pitch", "to Pitch", "LFO", -12.0, 12.0, 0.01, 0.0),
    def("lfo_depth", "to Cutoff", "LFO", -6.0, 6.0, 0.01, 0.0),
    def("lfo_amp", "to Amp", "LFO", 0.0, 1.0, 0.01, 0.0),
    def("lfo_pan", "to Pan", "LFO", 0.0, 1.0, 0.01, 0.0),
    // Voice
    def("glide", "Glide", "Voice", 0.0, 2.0, 0.005, 0.0),
    def("poly", "Polyphony", "Voice", 1.0, 16.0, 1.0, 8.0),
    def("pan_spread", "Pan Spread", "Voice", 0.0, 1.0, 0.01, 0.0),
    def("analog", "Analog Drift", "Voice", 0.0, 1.0, 0.01, 0.15),
];

/// Parameter group order the UI renders devices in.
pub const GROUP_ORDER: &[&str] = &["Oscillator", "Filter", "Envelope", "LFO", "Voice", "Drum", "MIDI"];

/// The synth parameter catalog for a voice kind (UI + docs).
pub fn param_defs_for(kind: &str) -> &'static [ParamDef] {
    match kind {
        "kick" => KICK,
        "snare" => SNARE,
        "hat" => HAT,
        "clap" => CLAP,
        "tom" => TOM,
        "perc" => PERC,
        "rim" => RIM,
        "cowbell" => COWBELL,
        "shaker" => SHAKER,
        "crash" => CRASH,
        "ride" => RIDE,
        k if is_melodic(k) => MELODIC_PARAMS,
        _ => &[],
    }
}

/// Every parameter definition across all kinds — used for `param_range`.
fn all_defs() -> impl Iterator<Item = &'static ParamDef> {
    MELODIC_PARAMS
        .iter()
        .chain(KICK.iter())
        .chain(SNARE.iter())
        .chain(HAT.iter())
        .chain(CLAP.iter())
        .chain(TOM.iter())
        .chain(PERC.iter())
        .chain(RIM.iter())
        .chain(COWBELL.iter())
        .chain(SHAKER.iter())
        .chain(CRASH.iter())
        .chain(RIDE.iter())
}

/// The value span of a parameter (for macro depths). Falls back to 1.
pub fn param_range(key: &str) -> Option<f64> {
    all_defs().find(|d| d.key == key).map(|d| d.max - d.min)
}

/// Look up a single parameter definition by key.
pub fn param_def(key: &str) -> Option<&'static ParamDef> {
    all_defs().find(|d| d.key == key)
}

/* ── Per-kind default parameters ─────────────────────────────── */

impl SynthParams {
    /// Parameters a kind starts from (before user overrides).
    pub fn kind_defaults(kind: &str) -> HashMap<String, f64> {
        let mut m: HashMap<String, f64> = HashMap::new();
        match kind {
            "bass" => {
                m.insert("o1w".into(), 2.0);
                m.insert("o2w".into(), 3.0);
                m.insert("a_level".into(), 0.8);
                m.insert("b_level".into(), 0.3);
                m.insert("b_octave".into(), -1.0);
                m.insert("sub".into(), 0.35);
                m.insert("cutoff".into(), 700.0);
                m.insert("res".into(), 1.2);
                m.insert("fenv".into(), 1.6);
                m.insert("fdecay".into(), 0.12);
                m.insert("fsustain".into(), 0.15);
                m.insert("poles".into(), 2.0);
                m.insert("drive".into(), 1.4);
                m.insert("attack".into(), 0.004);
                m.insert("decay".into(), 0.12);
                m.insert("sustain".into(), 0.5);
                m.insert("release".into(), 0.12);
                m.insert("poly".into(), 1.0);
                m.insert("glide".into(), 0.02);
            }
            "sub" => {
                m.insert("o1w".into(), 0.0);
                m.insert("a_level".into(), 0.95);
                m.insert("sub".into(), 0.2);
                m.insert("cutoff".into(), 260.0);
                m.insert("res".into(), 0.6);
                m.insert("poles".into(), 1.0);
                m.insert("drive".into(), 1.6);
                m.insert("attack".into(), 0.002);
                m.insert("decay".into(), 0.12);
                m.insert("sustain".into(), 0.65);
                m.insert("release".into(), 0.12);
                m.insert("poly".into(), 1.0);
                m.insert("glide".into(), 0.03);
            }
            "pluck" => {
                m.insert("o1w".into(), 2.0);
                m.insert("o2w".into(), 3.0);
                m.insert("a_level".into(), 0.7);
                m.insert("b_level".into(), 0.3);
                m.insert("b_semi".into(), 0.1);
                m.insert("cutoff".into(), 2200.0);
                m.insert("res".into(), 1.6);
                m.insert("fenv".into(), 2.6);
                m.insert("fdecay".into(), 0.18);
                m.insert("fsustain".into(), 0.0);
                m.insert("poles".into(), 2.0);
                m.insert("attack".into(), 0.003);
                m.insert("decay".into(), 0.22);
                m.insert("sustain".into(), 0.05);
                m.insert("release".into(), 0.28);
                m.insert("poly".into(), 8.0);
            }
            "lead" => {
                m.insert("o1w".into(), 2.0);
                m.insert("o2w".into(), 2.0);
                m.insert("a_level".into(), 0.55);
                m.insert("b_level".into(), 0.45);
                m.insert("b_semi".into(), -0.12);
                m.insert("unison".into(), 3.0);
                m.insert("spread".into(), 0.6);
                m.insert("cutoff".into(), 2800.0);
                m.insert("res".into(), 1.6);
                m.insert("fenv".into(), 1.8);
                m.insert("lfo_rate".into(), 5.0);
                m.insert("lfo_pitch".into(), 0.05);
                m.insert("attack".into(), 0.006);
                m.insert("decay".into(), 0.2);
                m.insert("sustain".into(), 0.6);
                m.insert("release".into(), 0.22);
                m.insert("poly".into(), 8.0);
                m.insert("pan_spread".into(), 0.25);
            }
            "pad" => {
                m.insert("o1w".into(), 2.0);
                m.insert("o2w".into(), 2.0);
                m.insert("a_level".into(), 0.5);
                m.insert("b_level".into(), 0.5);
                m.insert("b_semi".into(), 0.07);
                m.insert("unison".into(), 5.0);
                m.insert("spread".into(), 0.85);
                m.insert("cutoff".into(), 1200.0);
                m.insert("res".into(), 0.8);
                m.insert("fenv".into(), 1.2);
                m.insert("fdecay".into(), 1.2);
                m.insert("fsustain".into(), 0.6);
                m.insert("lfo_rate".into(), 0.22);
                m.insert("lfo_depth".into(), 0.35);
                m.insert("attack".into(), 0.5);
                m.insert("decay".into(), 0.6);
                m.insert("sustain".into(), 0.8);
                m.insert("release".into(), 1.1);
                m.insert("poly".into(), 12.0);
                m.insert("pan_spread".into(), 0.5);
                m.insert("analog".into(), 0.3);
            }
            "organ" => {
                m.insert("o1w".into(), 6.0);
                m.insert("o2w".into(), 0.0);
                m.insert("a_level".into(), 0.6);
                m.insert("b_level".into(), 0.35);
                m.insert("b_semi".into(), 12.0);
                m.insert("cutoff".into(), 7000.0);
                m.insert("res".into(), 0.6);
                m.insert("attack".into(), 0.01);
                m.insert("decay".into(), 0.1);
                m.insert("sustain".into(), 1.0);
                m.insert("release".into(), 0.15);
                m.insert("poly".into(), 10.0);
            }
            "ep" => {
                m.insert("o1w".into(), 1.0);
                m.insert("o2w".into(), 0.0);
                m.insert("a_level".into(), 0.7);
                m.insert("b_level".into(), 0.35);
                m.insert("b_semi".into(), 12.0);
                m.insert("cutoff".into(), 3200.0);
                m.insert("res".into(), 1.0);
                m.insert("fenv".into(), 1.6);
                m.insert("fdecay".into(), 0.55);
                m.insert("fsustain".into(), 0.1);
                m.insert("attack".into(), 0.002);
                m.insert("decay".into(), 0.45);
                m.insert("sustain".into(), 0.08);
                m.insert("release".into(), 0.35);
                m.insert("poly".into(), 10.0);
                m.insert("vel_filter".into(), 0.6);
            }
            "bell" => {
                m.insert("o1w".into(), 0.0);
                m.insert("a_level".into(), 0.7);
                m.insert("fm_ratio".into(), 3.5);
                m.insert("fm_index".into(), 1.4);
                m.insert("fm_decay".into(), 1.0);
                m.insert("cutoff".into(), 12000.0);
                m.insert("res".into(), 0.6);
                m.insert("attack".into(), 0.001);
                m.insert("decay".into(), 1.4);
                m.insert("sustain".into(), 0.0);
                m.insert("release".into(), 1.4);
                m.insert("poly".into(), 10.0);
            }
            "strings" => {
                m.insert("o1w".into(), 2.0);
                m.insert("o2w".into(), 2.0);
                m.insert("a_level".into(), 0.5);
                m.insert("b_level".into(), 0.5);
                m.insert("b_semi".into(), 0.14);
                m.insert("unison".into(), 4.0);
                m.insert("spread".into(), 0.75);
                m.insert("cutoff".into(), 1900.0);
                m.insert("res".into(), 0.7);
                m.insert("fenv".into(), 1.0);
                m.insert("lfo_rate".into(), 4.6);
                m.insert("lfo_pitch".into(), 0.04);
                m.insert("attack".into(), 0.7);
                m.insert("decay".into(), 0.4);
                m.insert("sustain".into(), 0.8);
                m.insert("release".into(), 1.2);
                m.insert("poly".into(), 12.0);
                m.insert("pan_spread".into(), 0.4);
                m.insert("analog".into(), 0.3);
            }
            "brass" => {
                m.insert("o1w".into(), 2.0);
                m.insert("o2w".into(), 2.0);
                m.insert("a_level".into(), 0.6);
                m.insert("b_level".into(), 0.4);
                m.insert("b_semi".into(), -0.06);
                m.insert("unison".into(), 2.0);
                m.insert("spread".into(), 0.4);
                m.insert("cutoff".into(), 1300.0);
                m.insert("res".into(), 2.0);
                m.insert("fenv".into(), 2.4);
                m.insert("fdecay".into(), 0.35);
                m.insert("fsustain".into(), 0.45);
                m.insert("attack".into(), 0.05);
                m.insert("decay".into(), 0.25);
                m.insert("sustain".into(), 0.75);
                m.insert("release".into(), 0.3);
                m.insert("poly".into(), 8.0);
                m.insert("vel_filter".into(), 0.55);
            }
            "synthme" => {
                m.insert("o1w".into(), 2.0);
                m.insert("o2w".into(), 3.0);
                m.insert("a_level".into(), 0.5);
                m.insert("b_level".into(), 0.5);
                m.insert("b_semi".into(), 0.1);
                m.insert("cutoff".into(), 2000.0);
                m.insert("res".into(), 1.0);
                m.insert("drive".into(), 2.0);
                m.insert("attack".into(), 0.005);
                m.insert("decay".into(), 0.2);
                m.insert("sustain".into(), 0.6);
                m.insert("release".into(), 0.3);
                m.insert("poly".into(), 8.0);
            }
            "fm" => {
                m.insert("o1w".into(), 0.0);
                m.insert("a_level".into(), 0.8);
                m.insert("fm_ratio".into(), 2.0);
                m.insert("fm_index".into(), 2.2);
                m.insert("fm_decay".into(), 0.4);
                m.insert("cutoff".into(), 14000.0);
                m.insert("res".into(), 0.6);
                m.insert("attack".into(), 0.002);
                m.insert("decay".into(), 0.35);
                m.insert("sustain".into(), 0.25);
                m.insert("release".into(), 0.4);
                m.insert("poly".into(), 8.0);
                m.insert("vel_amp".into(), 0.6);
            }
            _ => {}
        }
        // Anything not covered starts from the catalog defaults so the UI
        // sliders and the renderer agree out of the box.
        for d in param_defs_for(kind) {
            m.entry(d.key.to_string()).or_insert(d.default);
        }
        m
    }
}

/// Default parameters for a kind, as a `HashMap` for the engine.
pub fn kind_params(kind: &str) -> HashMap<String, f64> {
    SynthParams::kind_defaults(kind)
}

/// Default mix level per kind (used when a voice omits `level`).
pub fn default_level(kind: &str) -> f32 {
    match kind {
        "kick" => 0.9,
        "snare" => 0.75,
        "hat" => 0.45,
        "bass" => 0.7,
        "pluck" => 0.5,
        "lead" => 0.5,
        "pad" => 0.42,
        "sub" => 0.7,
        "clap" => 0.5,
        "tom" => 0.7,
        "perc" => 0.4,
        "rim" => 0.45,
        "cowbell" => 0.4,
        "shaker" => 0.35,
        "crash" => 0.35,
        "ride" => 0.35,
        "organ" => 0.5,
        "ep" => 0.5,
        "bell" => 0.45,
        "strings" => 0.5,
        "brass" => 0.55,
        "synthme" => 0.55,
        "fm" => 0.5,
        "drumkit" => 0.7,
        "grid" => 0.6,
        _ => 0.6,
    }
}

/// Default pan per kind (used when a voice omits `pan`).
pub fn default_pan(kind: &str) -> f32 {
    match kind {
        "hat" => 0.35,
        "lead" => -0.2,
        "snare" => 0.05,
        "kick" => -0.05,
        "organ" => 0.1,
        "ep" => -0.1,
        "strings" => 0.15,
        "brass" => 0.1,
        _ => 0.0,
    }
}

/* ── MIDI effects ────────────────────────────────────────────── */

/// MIDI (note-processing) effects applied before synthesis.
pub const MIDI_FX_KINDS: &[&str] = &["transpose", "velocity", "gate", "ratchet", "probability", "humanize"];
pub fn is_midi_fx(kind: &str) -> bool {
    MIDI_FX_KINDS.contains(&kind)
}
static TRANSPOSE_PARAMS: &[ParamDef] = &[def("steps", "Steps", "MIDI", -24.0, 24.0, 1.0, 0.0)];
static VELOCITY_PARAMS: &[ParamDef] = &[def("amount", "Amount", "MIDI", 0.0, 2.0, 0.01, 1.0)];
static GATE_PARAMS: &[ParamDef] = &[def("amount", "Amount", "MIDI", 0.1, 4.0, 0.01, 1.0)];
static RATCHET_PARAMS: &[ParamDef] = &[def("count", "Count", "MIDI", 2.0, 8.0, 1.0, 2.0)];
static PROBABILITY_PARAMS: &[ParamDef] = &[def("amount", "Chance", "MIDI", 0.0, 1.0, 0.01, 1.0)];
static HUMANIZE_PARAMS: &[ParamDef] = &[def("amount", "Amount", "MIDI", 0.0, 1.0, 0.01, 0.25)];

/// Parameter catalog for a MIDI effect kind.
pub fn midi_fx_defs(kind: &str) -> &'static [ParamDef] {
    match kind {
        "transpose" => TRANSPOSE_PARAMS,
        "velocity" => VELOCITY_PARAMS,
        "gate" => GATE_PARAMS,
        "ratchet" => RATCHET_PARAMS,
        "probability" => PROBABILITY_PARAMS,
        "humanize" => HUMANIZE_PARAMS,
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_has_params_or_is_special() {
        for kind in KINDS {
            if matches!(*kind, "drumkit" | "grid") {
                continue;
            }
            assert!(!param_defs_for(kind).is_empty(), "{kind} has no parameter catalog");
            assert!(!kind_params(kind).is_empty(), "{kind} has no defaults");
            assert!(default_level(kind) > 0.0, "{kind} has no default level");
        }
    }

    #[test]
    fn melodic_kinds_are_not_drums() {
        for k in MELODIC_KINDS {
            assert!(is_melodic(k));
            assert!(!is_drum(k));
        }
    }

    #[test]
    fn defaults_stay_inside_catalog_ranges() {
        for kind in MELODIC_KINDS.iter().chain(DRUM_KINDS) {
            let defaults = kind_params(kind);
            for def in param_defs_for(kind) {
                if let Some(v) = defaults.get(def.key) {
                    assert!(
                        *v >= def.min - 1e-9 && *v <= def.max + 1e-9,
                        "{kind}.{} default {v} outside [{}, {}]",
                        def.key,
                        def.min,
                        def.max
                    );
                }
            }
        }
    }
}
