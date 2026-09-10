//! Effect catalog: the insert effects the engine can build, their parameters
//! and their defaults. Mirrors [`crate::dsp::fx::EffectKind`].

use std::collections::HashMap;

use crate::voices::{choice, def, logdef, ParamDef};

/// Effect kinds, in UI order.
pub const EFFECT_KINDS: &[&str] =
    &["distortion", "filter", "eq", "compressor", "delay", "reverb", "chorus", "phaser", "bitcrush"];

pub fn is_effect(kind: &str) -> bool {
    EFFECT_KINDS.contains(&kind)
}

const DIST_MODES: &[&str] = &["Soft", "Hard", "Fold", "Tube", "Fuzz"];
const FILTER_TYPES: &[&str] = &["LP", "HP", "BP", "Notch", "Peak"];

static DISTORTION: &[ParamDef] = &[
    choice("mode", "Mode", "Distortion", 0.0, DIST_MODES),
    def("drive", "Drive", "Distortion", 0.25, 24.0, 0.05, 2.0),
    def("mix", "Mix", "Distortion", 0.0, 1.0, 0.01, 0.6),
    logdef("tone", "Tone", "Distortion", 800.0, 20000.0, 10.0, 12000.0),
    def("out", "Out", "Distortion", 0.1, 3.0, 0.05, 1.0),
];
static FILTER: &[ParamDef] = &[
    choice("type", "Type", "Filter", 0.0, FILTER_TYPES),
    logdef("cutoff", "Cutoff", "Filter", 20.0, 20000.0, 10.0, 2000.0),
    def("resonance", "Resonance", "Filter", 0.4, 20.0, 0.1, 1.0),
    def("drive", "Drive", "Filter", 1.0, 24.0, 0.05, 1.0),
    def("poles", "Slope", "Filter", 1.0, 2.0, 1.0, 1.0),
];
static EQ: &[ParamDef] = &[
    def("low_gain", "Low Gain", "EQ", -24.0, 24.0, 0.5, 0.0),
    def("mid_gain", "Mid Gain", "EQ", -24.0, 24.0, 0.5, 0.0),
    def("hi_gain", "High Gain", "EQ", -24.0, 24.0, 0.5, 0.0),
];
static COMPRESSOR: &[ParamDef] = &[
    def("threshold", "Threshold", "Dynamics", -60.0, 0.0, 0.5, -18.0),
    def("ratio", "Ratio", "Dynamics", 1.0, 20.0, 0.1, 4.0),
    def("attack", "Attack", "Dynamics", 0.1, 200.0, 0.5, 10.0),
    def("release", "Release", "Dynamics", 1.0, 2000.0, 1.0, 150.0),
    def("knee", "Knee", "Dynamics", 0.0, 24.0, 0.5, 6.0),
    def("makeup", "Makeup", "Dynamics", 0.0, 30.0, 0.5, 0.0),
    def("mix", "Mix", "Dynamics", 0.0, 1.0, 0.01, 1.0),
];
static DELAY: &[ParamDef] = &[
    def("time", "Time", "Delay", 1.0, 4000.0, 1.0, 250.0),
    def("feedback", "Feedback", "Delay", 0.0, 0.95, 0.01, 0.4),
    def("mix", "Mix", "Delay", 0.0, 1.0, 0.01, 0.3),
    def("ping_pong", "Ping-Pong", "Delay", 0.0, 1.0, 0.01, 0.45),
    def("damp", "Damping", "Delay", 0.0, 1.0, 0.01, 0.35),
    def("mod", "Mod", "Delay", 0.0, 1.0, 0.01, 0.15),
    def("offset", "Offset", "Delay", 0.0, 500.0, 1.0, 0.0),
];
static REVERB: &[ParamDef] = &[
    def("size", "Size", "Reverb", 0.05, 1.6, 0.01, 0.5),
    def("damping", "Damping", "Reverb", 0.0, 1.0, 0.01, 0.5),
    def("mix", "Mix", "Reverb", 0.0, 1.0, 0.01, 0.2),
    def("predelay", "Pre-Delay", "Reverb", 0.0, 250.0, 1.0, 12.0),
    def("width", "Width", "Reverb", 0.0, 2.0, 0.01, 1.0),
    def("mod", "Mod", "Reverb", 0.0, 1.0, 0.01, 0.6),
];
static CHORUS: &[ParamDef] = &[
    def("rate", "Rate", "Chorus", 0.01, 10.0, 0.01, 0.6),
    def("depth", "Depth", "Chorus", 0.0, 1.0, 0.01, 0.5),
    def("mix", "Mix", "Chorus", 0.0, 1.0, 0.01, 0.4),
    def("spread", "Spread", "Chorus", 0.0, 1.0, 0.01, 0.6),
];
static PHASER: &[ParamDef] = &[
    def("rate", "Rate", "Phaser", 0.01, 8.0, 0.01, 0.3),
    def("depth", "Depth", "Phaser", 0.0, 1.0, 0.01, 0.7),
    def("mix", "Mix", "Phaser", 0.0, 1.0, 0.01, 0.5),
    def("feedback", "Feedback", "Phaser", 0.0, 0.95, 0.01, 0.4),
    def("stages", "Stages", "Phaser", 1.0, 6.0, 1.0, 4.0),
];
static BITCRUSH: &[ParamDef] = &[
    def("bits", "Bits", "Bitcrush", 1.0, 16.0, 1.0, 8.0),
    def("downsample", "Downsample", "Bitcrush", 1.0, 64.0, 1.0, 1.0),
    def("mix", "Mix", "Bitcrush", 0.0, 1.0, 0.01, 1.0),
];

/// Parameter catalog for an effect kind.
pub fn param_defs(kind: &str) -> &'static [ParamDef] {
    match kind {
        "distortion" => DISTORTION,
        "filter" => FILTER,
        "eq" => EQ,
        "compressor" => COMPRESSOR,
        "delay" => DELAY,
        "reverb" => REVERB,
        "chorus" => CHORUS,
        "phaser" => PHASER,
        "bitcrush" => BITCRUSH,
        _ => &[],
    }
}

/// Default parameter map for an effect kind.
pub fn defaults(kind: &str) -> HashMap<String, f64> {
    param_defs(kind).iter().map(|d| (d.key.to_string(), d.default)).collect()
}

/// Display label for an effect kind.
pub fn label(kind: &str) -> &'static str {
    match kind {
        "distortion" => "Distortion",
        "filter" => "Filter",
        "eq" => "EQ",
        "compressor" => "Compressor",
        "delay" => "Delay",
        "reverb" => "Reverb",
        "chorus" => "Chorus",
        "phaser" => "Phaser",
        "bitcrush" => "Bitcrush",
        _ => "Effect",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_effect_has_params_and_defaults() {
        for kind in EFFECT_KINDS {
            assert!(!param_defs(kind).is_empty(), "{kind} has no catalog");
            let d = defaults(kind);
            for def in param_defs(kind) {
                let v = d[def.key];
                assert!(v >= def.min - 1e-9 && v <= def.max + 1e-9, "{kind}.{} default {v} out of range", def.key);
            }
        }
    }
}
