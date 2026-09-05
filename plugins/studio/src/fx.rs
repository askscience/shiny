//! Insert effects: catalog + trem graph builders for a per-voice device chain.
//!
//! Mono effects (distortion, filter) sit between the instrument and the
//! level/pan gain; stereo effects (EQ, compressor, delay, reverb) sit after
//! the gain, in the order given.

use std::collections::HashMap;

use trem::dsp::{
    BiquadFilter, Compressor, Distortion, FilterType, ParametricEq, PlateReverb, StereoDelay,
};
use trem::graph::{Graph, NodeId};

use crate::voices::{def, ParamDef};

pub const EFFECT_KINDS: &[&str] = &["distortion", "filter", "eq", "compressor", "delay", "reverb"];

pub fn is_effect(kind: &str) -> bool {
    EFFECT_KINDS.contains(&kind)
}

/// True when the effect processes a stereo (2-in 2-out) signal.
pub fn is_stereo(kind: &str) -> bool {
    matches!(kind, "eq" | "compressor" | "delay" | "reverb")
}

static DISTORTION: &[ParamDef] = &[
    def("mode", "Mode", 0.0, 4.0, 1.0, 0.0),
    def("drive", "Drive", 0.25, 24.0, 0.05, 2.0),
    def("mix", "Mix", 0.0, 1.0, 0.01, 0.5),
    def("out", "Out", 0.1, 3.0, 0.05, 1.0),
];
static FILTER: &[ParamDef] = &[
    def("type", "Type", 0.0, 2.0, 1.0, 0.0), // 0 LP, 1 HP, 2 BP
    def("cutoff", "Cutoff", 20.0, 20000.0, 10.0, 2000.0),
    def("resonance", "Res", 0.1, 20.0, 0.1, 1.0),
];
static EQ: &[ParamDef] = &[
    def("low_gain", "Lo Gain", -24.0, 24.0, 0.5, 0.0),
    def("mid_freq", "Mid Freq", 20.0, 20000.0, 10.0, 1000.0),
    def("mid_gain", "Mid Gain", -24.0, 24.0, 0.5, 0.0),
    def("hi_gain", "Hi Gain", -24.0, 24.0, 0.5, 0.0),
];
static COMPRESSOR: &[ParamDef] = &[
    def("threshold", "Threshold", -60.0, 0.0, 0.5, -18.0),
    def("ratio", "Ratio", 1.0, 20.0, 0.1, 4.0),
    def("attack", "Attack", 0.1, 200.0, 0.5, 10.0),
    def("release", "Release", 1.0, 2000.0, 1.0, 150.0),
    def("makeup", "Makeup", 0.0, 30.0, 0.5, 0.0),
];
static DELAY: &[ParamDef] = &[
    def("time", "Time", 1.0, 2000.0, 1.0, 250.0),
    def("feedback", "Feedback", 0.0, 0.95, 0.01, 0.4),
    def("mix", "Mix", 0.0, 1.0, 0.01, 0.3),
];
static REVERB: &[ParamDef] = &[
    def("size", "Size", 0.0, 1.0, 0.01, 0.5),
    def("damping", "Damping", 0.0, 1.0, 0.01, 0.5),
    def("mix", "Mix", 0.0, 1.0, 0.01, 0.2),
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
        _ => &[],
    }
}

/// Build an effect node and apply its params. Returns the node id.
pub fn build_effect(g: &mut Graph, kind: &str, params: &HashMap<String, f64>) -> Result<NodeId, String> {
    let val = |key: &str, d: f64| params.get(key).copied().unwrap_or(d);
    match kind {
        "distortion" => {
            let n = g.add_node(Box::new(Distortion::new()));
            g.set_node_param(n, 0, val("mode", 0.0));
            g.set_node_param(n, 1, val("drive", 2.0));
            g.set_node_param(n, 2, val("mix", 0.5));
            g.set_node_param(n, 3, val("out", 1.0));
            Ok(n)
        }
        "filter" => {
            let ftype = match val("type", 0.0) as i64 {
                1 => FilterType::HighPass,
                2 => FilterType::BandPass,
                _ => FilterType::LowPass,
            };
            Ok(g.add_node(Box::new(BiquadFilter::new(ftype, val("cutoff", 2000.0), val("resonance", 1.0)))))
        }
        "eq" => {
            let n = g.add_node(Box::new(ParametricEq::new()));
            g.set_node_param(n, 1, val("low_gain", 0.0));
            g.set_node_param(n, 3, val("mid_freq", 1000.0));
            g.set_node_param(n, 4, val("mid_gain", 0.0));
            g.set_node_param(n, 7, val("hi_gain", 0.0));
            Ok(n)
        }
        "compressor" => {
            let n = g.add_node(Box::new(Compressor::new(
                val("threshold", -18.0),
                val("ratio", 4.0),
                val("attack", 10.0),
                val("release", 150.0),
            )));
            g.set_node_param(n, 4, val("makeup", 0.0));
            Ok(n)
        }
        "delay" => Ok(g.add_node(Box::new(StereoDelay::new(
            val("time", 250.0),
            val("feedback", 0.4),
            val("mix", 0.3),
        )))),
        "reverb" => Ok(g.add_node(Box::new(PlateReverb::new(
            val("size", 0.5),
            val("damping", 0.5),
            val("mix", 0.2),
        )))),
        _ => Err(format!("unknown effect `{kind}`")),
    }
}
