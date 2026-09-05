//! Instrument builders: map a voice `kind` to a mono trem graph chain, and
//! declare the synth parameters each kind exposes.
//!
//! Every builder returns the node id of the chain's final **mono** node plus
//! the list of tweakable parameters (key → node/param_id), so the engine can
//! apply the shared config's per-voice `synth` map and the UI can render
//! matching sliders.

use trem::dsp::{
    analog_voice, lead_voice, Adsr, BiquadFilter, Distortion, FilterType, HatSynth, KickSynth,
    MonoCrossfade, Oscillator, SnareSynth, Waveform,
};
use trem::graph::{Graph, NodeId};

/// Voice kinds the Studio engine understands, in UI order.
pub const KINDS: &[&str] = &[
    "kick", "snare", "hat", "clap", "tom", "perc", "bass", "pluck", "lead", "pad", "sub", "drumkit",
];

/// True when `kind` names a supported instrument.
pub fn is_kind(kind: &str) -> bool {
    KINDS.contains(&kind)
}

/// A self-describing synth parameter (mirrors trem's `ParamDescriptor`).
#[derive(Debug, Clone, Copy)]
pub struct ParamDef {
    pub key: &'static str,
    pub label: &'static str,
    pub min: f64,
    pub max: f64,
    pub step: f64,
    pub default: f64,
}

pub const fn def(key: &'static str, label: &'static str, min: f64, max: f64, step: f64, default: f64) -> ParamDef {
    ParamDef { key, label, min, max, step, default }
}

const KICK_PARAMS: &[ParamDef] = &[
    def("pitch", "Pitch", 20.0, 200.0, 1.0, 50.0),
    def("decay", "Decay", 2.0, 30.0, 0.1, 8.0),
    def("sweep", "Sweep", 5.0, 80.0, 0.5, 30.0),
];
const SNARE_PARAMS: &[ParamDef] = &[
    def("tone", "Tone", 80.0, 400.0, 1.0, 200.0),
    def("body", "Body", 5.0, 60.0, 0.1, 25.0),
    def("noise", "Noise", 5.0, 40.0, 0.1, 15.0),
];
const HAT_PARAMS: &[ParamDef] = &[
    def("decay", "Decay", 10.0, 100.0, 0.5, 40.0),
];
const BASS_PARAMS: &[ParamDef] = &[
    def("detune", "Detune", -24.0, 24.0, 0.1, 0.0),
    def("cutoff", "Cutoff", 20.0, 20000.0, 10.0, 700.0),
    def("resonance", "Resonance", 0.1, 20.0, 0.1, 0.9),
    def("attack", "Attack", 0.001, 5.0, 0.005, 0.004),
    def("decay", "Decay", 0.001, 5.0, 0.01, 0.12),
    def("sustain", "Sustain", 0.0, 1.0, 0.05, 0.5),
    def("release", "Release", 0.001, 5.0, 0.01, 0.12),
];
const PLUCK_PARAMS: &[ParamDef] = &[
    def("detune", "Detune", -24.0, 24.0, 0.1, 0.1),
    def("osc_mix", "Osc Mix", 0.0, 1.0, 0.05, 0.5),
    def("cutoff", "Cutoff", 20.0, 20000.0, 10.0, 2000.0),
    def("resonance", "Resonance", 0.1, 20.0, 0.1, 1.5),
    def("attack", "Attack", 0.001, 5.0, 0.005, 0.005),
    def("decay", "Decay", 0.001, 5.0, 0.01, 0.2),
    def("sustain", "Sustain", 0.0, 1.0, 0.05, 0.6),
    def("release", "Release", 0.001, 5.0, 0.01, 0.3),
];
const LEAD_PARAMS: &[ParamDef] = &[
    def("detune", "Detune", -24.0, 24.0, 0.1, 0.1),
    def("osc_mix", "Osc Mix", 0.0, 1.0, 0.05, 0.52),
    def("wt_mix", "WT Mix", 0.0, 1.0, 0.05, 0.88),
    def("wt_shape", "WT Shape", 0.0, 3.0, 0.05, 1.4),
    def("cutoff", "Cutoff", 20.0, 20000.0, 10.0, 2800.0),
    def("resonance", "Resonance", 0.1, 20.0, 0.1, 1.65),
    def("lfo_rate", "LFO Rate", 0.01, 50.0, 0.01, 0.28),
    def("lfo_depth", "LFO Depth", 0.0, 2000.0, 1.0, 520.0),
    def("attack", "Attack", 0.001, 5.0, 0.005, 0.004),
    def("decay", "Decay", 0.001, 5.0, 0.01, 0.18),
    def("sustain", "Sustain", 0.0, 1.0, 0.05, 0.55),
    def("release", "Release", 0.001, 5.0, 0.01, 0.22),
];
const PAD_PARAMS: &[ParamDef] = &[
    def("detune", "Detune", -24.0, 24.0, 0.1, 0.12),
    def("cutoff", "Cutoff", 20.0, 20000.0, 10.0, 1600.0),
    def("resonance", "Resonance", 0.1, 20.0, 0.1, 0.7),
    def("attack", "Attack", 0.001, 5.0, 0.005, 0.4),
    def("decay", "Decay", 0.001, 5.0, 0.01, 0.5),
    def("sustain", "Sustain", 0.0, 1.0, 0.05, 0.8),
    def("release", "Release", 0.001, 5.0, 0.01, 0.9),
];
const SUB_PARAMS: &[ParamDef] = &[
    def("drive", "Drive", 0.25, 24.0, 0.05, 4.0),
    def("attack", "Attack", 0.001, 5.0, 0.005, 0.002),
    def("decay", "Decay", 0.001, 5.0, 0.01, 0.12),
    def("sustain", "Sustain", 0.0, 1.0, 0.05, 0.6),
    def("release", "Release", 0.001, 5.0, 0.01, 0.12),
];
const CLAP_PARAMS: &[ParamDef] = &[
    def("tone", "Tone", 80.0, 400.0, 1.0, 180.0),
    def("body", "Body", 5.0, 60.0, 0.1, 10.0),
    def("noise", "Noise", 5.0, 40.0, 0.1, 35.0),
];
const TOM_PARAMS: &[ParamDef] = &[
    def("pitch", "Pitch", 20.0, 400.0, 1.0, 150.0),
    def("decay", "Decay", 2.0, 40.0, 0.1, 20.0),
    def("sweep", "Sweep", 5.0, 100.0, 0.5, 50.0),
];
const PERC_PARAMS: &[ParamDef] = &[
    def("decay", "Decay", 10.0, 200.0, 0.5, 60.0),
];

/// The synth parameter catalog for a voice kind (UI + docs).
pub fn param_defs_for(kind: &str) -> &'static [ParamDef] {
    match kind {
        "kick" => KICK_PARAMS,
        "snare" => SNARE_PARAMS,
        "hat" => HAT_PARAMS,
        "bass" => BASS_PARAMS,
        "pluck" => PLUCK_PARAMS,
        "lead" => LEAD_PARAMS,
        "pad" => PAD_PARAMS,
        "sub" => SUB_PARAMS,
        "clap" => CLAP_PARAMS,
        "tom" => TOM_PARAMS,
        "perc" => PERC_PARAMS,
        "drumkit" => &[],
        _ => &[],
    }
}

/// Default mix level per kind (used when a voice omits `level`).
pub fn default_level(kind: &str) -> f32 {
    match kind {
        "kick" => 0.9,
        "snare" => 0.75,
        "hat" => 0.45,
        "bass" => 0.7,
        "pluck" => 0.5,
        "lead" => 0.55,
        "pad" => 0.5,
        "sub" => 0.7,
        "clap" => 0.5,
        "tom" => 0.7,
        "perc" => 0.4,
        "drumkit" => 0.7,
        _ => 0.6,
    }
}

/// Default pan per kind (used when a voice omits `pan`).
pub fn default_pan(kind: &str) -> f32 {
    match kind {
        "hat" => 0.35,
        "lead" => -0.25,
        "snare" => 0.05,
        "kick" => -0.05,
        _ => 0.0,
    }
}

/// Resolve a wave name to a trem [`Waveform`] (bass voice).
pub fn waveform(name: Option<&str>) -> Waveform {
    match name.unwrap_or("sine") {
        "saw" => Waveform::Saw,
        "square" => Waveform::Square,
        "triangle" | "tri" => Waveform::Triangle,
        _ => Waveform::Sine,
    }
}

/// A built voice: its mono output node plus each exposed synth parameter
/// (def, node id, param id).
pub struct BuiltVoice {
    pub out: NodeId,
    pub params: Vec<(ParamDef, NodeId, u32)>,
}

/// Build the mono instrument chain for `kind`, listening on `voice_id`.
pub fn build_instrument(g: &mut Graph, kind: &str, voice_id: u32, wave: Option<&str>) -> BuiltVoice {
    match kind {
        "snare" => {
            let n = g.add_node(Box::new(SnareSynth::new(voice_id)));
            BuiltVoice {
                out: n,
                params: SNARE_PARAMS.iter().enumerate().map(|(i, d)| (*d, n, i as u32)).collect(),
            }
        }
        "hat" => {
            let n = g.add_node(Box::new(HatSynth::new(voice_id)));
            BuiltVoice {
                out: n,
                params: HAT_PARAMS.iter().enumerate().map(|(i, d)| (*d, n, i as u32)).collect(),
            }
        }
        "bass" => {
            let osc = g.add_node(Box::new(Oscillator::new(waveform(wave)).with_voice(voice_id)));
            let filt = g.add_node(Box::new(BiquadFilter::new(FilterType::LowPass, 700.0, 0.9)));
            let env = g.add_node(Box::new(Adsr::new(0.004, 0.12, 0.5, 0.12).with_voice(voice_id)));
            g.connect(osc, 0, filt, 0);
            g.connect(filt, 0, env, 0);
            BuiltVoice {
                out: env,
                params: vec![
                    (BASS_PARAMS[0], osc, 0),   // detune
                    (BASS_PARAMS[1], filt, 0),  // cutoff
                    (BASS_PARAMS[2], filt, 1),  // resonance
                    (BASS_PARAMS[3], env, 0),   // attack
                    (BASS_PARAMS[4], env, 1),   // decay
                    (BASS_PARAMS[5], env, 2),   // sustain
                    (BASS_PARAMS[6], env, 3),   // release
                ],
            }
        }
        "pluck" => {
            let n = g.add_node(Box::new(analog_voice(voice_id, 512)));
            BuiltVoice {
                out: n,
                // analog_voice exposes ids 0..8 (detune..level); skip id 8 (level)
                params: PLUCK_PARAMS.iter().enumerate().map(|(i, d)| (*d, n, i as u32)).collect(),
            }
        }
        "lead" => {
            let n = g.add_node(Box::new(lead_voice(voice_id, 512)));
            BuiltVoice {
                out: n,
                // lead_voice exposes ids 0..12 (detune..level); skip id 12 (level)
                params: LEAD_PARAMS.iter().enumerate().map(|(i, d)| (*d, n, i as u32)).collect(),
            }
        }
        "pad" => {
            let osc1 = g.add_node(Box::new(Oscillator::new(Waveform::Saw).with_voice(voice_id)));
            let mut osc2p = Oscillator::new(Waveform::Saw).with_voice(voice_id);
            osc2p.detune = 0.12;
            let osc2 = g.add_node(Box::new(osc2p));
            let xfade = g.add_node(Box::new(MonoCrossfade::new(0.5)));
            let filt = g.add_node(Box::new(BiquadFilter::new(FilterType::LowPass, 1600.0, 0.7)));
            let env = g.add_node(Box::new(Adsr::new(0.4, 0.5, 0.8, 0.9).with_voice(voice_id)));
            g.connect(osc1, 0, xfade, 0);
            g.connect(osc2, 0, xfade, 1);
            g.connect(xfade, 0, filt, 0);
            g.connect(filt, 0, env, 0);
            BuiltVoice {
                out: env,
                params: vec![
                    (PAD_PARAMS[0], osc2, 0), // detune
                    (PAD_PARAMS[1], filt, 0), // cutoff
                    (PAD_PARAMS[2], filt, 1), // resonance
                    (PAD_PARAMS[3], env, 0),  // attack
                    (PAD_PARAMS[4], env, 1),  // decay
                    (PAD_PARAMS[5], env, 2),  // sustain
                    (PAD_PARAMS[6], env, 3),  // release
                ],
            }
        }
        "sub" => {
            let osc = g.add_node(Box::new(Oscillator::new(Waveform::Sine).with_voice(voice_id)));
            let dist = g.add_node(Box::new(Distortion::new()));
            let env = g.add_node(Box::new(Adsr::new(0.002, 0.12, 0.6, 0.12).with_voice(voice_id)));
            g.connect(osc, 0, dist, 0);
            g.connect(dist, 0, env, 0);
            BuiltVoice {
                out: env,
                params: vec![
                    (SUB_PARAMS[0], dist, 1), // drive
                    (SUB_PARAMS[1], env, 0),  // attack
                    (SUB_PARAMS[2], env, 1),  // decay
                    (SUB_PARAMS[3], env, 2),  // sustain
                    (SUB_PARAMS[4], env, 3),  // release
                ],
            }
        }
        "clap" => {
            let n = g.add_node(Box::new(SnareSynth::new(voice_id)));
            g.set_node_param(n, 0, 180.0); // tone
            g.set_node_param(n, 1, 10.0);  // body
            g.set_node_param(n, 2, 35.0);  // noise
            BuiltVoice {
                out: n,
                params: CLAP_PARAMS.iter().enumerate().map(|(i, d)| (*d, n, i as u32)).collect(),
            }
        }
        "tom" => {
            let n = g.add_node(Box::new(KickSynth::new(voice_id)));
            g.set_node_param(n, 0, 150.0); // pitch
            g.set_node_param(n, 1, 20.0);  // decay
            g.set_node_param(n, 2, 50.0);  // sweep
            BuiltVoice {
                out: n,
                params: TOM_PARAMS.iter().enumerate().map(|(i, d)| (*d, n, i as u32)).collect(),
            }
        }
        "perc" => {
            let n = g.add_node(Box::new(HatSynth::new(voice_id)));
            g.set_node_param(n, 0, 60.0); // decay
            BuiltVoice {
                out: n,
                params: PERC_PARAMS.iter().enumerate().map(|(i, d)| (*d, n, i as u32)).collect(),
            }
        }
        _ => {
            // "kick" and unknown fall back to a kick
            let n = g.add_node(Box::new(KickSynth::new(voice_id)));
            BuiltVoice {
                out: n,
                params: KICK_PARAMS.iter().enumerate().map(|(i, d)| (*d, n, i as u32)).collect(),
            }
        }
    }
}
