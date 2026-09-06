//! The Grid — a modular patch that compiles to a trem graph.
//!
//! A [`GridPatch`] is a list of modules (oscillator, noise, filter, drive,
//! gain, mixer, envelope, LFO, output) connected by patch cords. Audio cables
//! wire trem nodes together; control cables (from `env.ctrl` / `lfo.ctrl` into
//! a module's `mod` port) modulate a parameter over time and are rendered with
//! per-step segment re-rendering.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use trem::dsp::{
    Adsr, BiquadFilter, Distortion, FilterType, MonoCrossfade, MonoGain, Noise, Oscillator, Waveform,
};
use trem::graph::{Graph, NodeId};

use crate::voices::{def, ParamDef};

/// One module instance in a patch.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GridModule {
    pub id: String,
    #[serde(default = "default_grid_module_kind")]
    pub kind: String,
    pub params: HashMap<String, f64>,
}

fn default_grid_module_kind() -> String {
    "osc".into()
}

impl Default for GridModule {
    fn default() -> Self {
        Self { id: String::new(), kind: "osc".into(), params: HashMap::new() }
    }
}

/// A patch cord: from (module id, port name) to (module id, port name).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GridCable {
    pub from: (String, String),
    pub to: (String, String),
}

/// A complete modular patch.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct GridPatch {
    pub modules: Vec<GridModule>,
    pub cables: Vec<GridCable>,
}

/// Module kinds the Grid understands.
pub const GRID_MODULES: &[&str] = &["osc", "noise", "filter", "drive", "gain", "mixer", "env", "lfo", "out"];

/// Parameter catalog for a module kind (palette + inspector).
const OSC_PARAMS: &[ParamDef] = &[
    def("wave", "Wave", 0.0, 3.0, 1.0, 2.0),
    def("detune", "Detune", -24.0, 24.0, 0.1, 0.0),
];
const FILTER_PARAMS: &[ParamDef] = &[
    def("type", "Type", 0.0, 2.0, 1.0, 0.0),
    def("cutoff", "Cutoff", 20.0, 20000.0, 10.0, 2000.0),
    def("res", "Res", 0.1, 20.0, 0.1, 1.0),
];
const DRIVE_PARAMS: &[ParamDef] = &[def("drive", "Drive", 0.25, 24.0, 0.05, 2.0)];
const GAIN_PARAMS: &[ParamDef] = &[def("level", "Level", 0.0, 2.0, 0.01, 0.8)];
const MIXER_PARAMS: &[ParamDef] = &[def("balance", "Balance", 0.0, 1.0, 0.05, 0.5)];
const ENV_PARAMS: &[ParamDef] = &[
    def("attack", "Attack", 0.001, 5.0, 0.005, 0.005),
    def("decay", "Decay", 0.001, 5.0, 0.01, 0.2),
    def("sustain", "Sustain", 0.0, 1.0, 0.05, 0.6),
    def("release", "Release", 0.001, 5.0, 0.01, 0.3),
];
const LFO_PARAMS: &[ParamDef] = &[
    def("rate", "Rate", 0.05, 20.0, 0.01, 1.0),
    def("depth", "Depth", 0.0, 1.0, 0.01, 0.5),
    def("wave", "Wave", 0.0, 2.0, 1.0, 0.0),
];

pub fn module_params(kind: &str) -> &'static [ParamDef] {
    match kind {
        "osc" => OSC_PARAMS,
        "filter" => FILTER_PARAMS,
        "drive" => DRIVE_PARAMS,
        "gain" => GAIN_PARAMS,
        "mixer" => MIXER_PARAMS,
        "env" => ENV_PARAMS,
        "lfo" => LFO_PARAMS,
        _ => &[],
    }
}

fn param(m: &GridModule, key: &str, default: f64) -> f64 {
    m.params.get(key).copied().unwrap_or(default)
}

fn wave_from_idx(i: f64) -> Waveform {
    match i as i64 {
        0 => Waveform::Sine,
        1 => Waveform::Triangle,
        2 => Waveform::Saw,
        _ => Waveform::Square,
    }
}

/// A control-rate modulation source (computed analytically per segment).
#[derive(Clone, Copy)]
pub enum ModSource {
    Lfo { rate: f64, depth: f64, wave: i64 },
    Env { attack: f64, decay: f64, sustain: f64, release: f64 },
}

/// A compiled modulation binding: a source driving a target param.
pub struct ModBinding {
    pub target_node: NodeId,
    pub param_id: u32,
    pub base: f64,
    pub lo: f64,
    pub hi: f64,
    pub source: ModSource,
}

/// A compiled patch: the audio output node + its modulation bindings.
pub struct CompiledGrid {
    pub out: NodeId,
    pub modulations: Vec<ModBinding>,
}

/// Build the audio graph for a patch (base params; modulation applied later).
pub fn compile_grid(g: &mut Graph, patch: &GridPatch, voice_id: u32) -> Result<CompiledGrid, String> {
    let get = |id: &str| -> Result<&GridModule, String> {
        patch.modules.iter().find(|m| m.id == id).ok_or_else(|| format!("grid: unknown module `{id}`"))
    };
    let mut nodes: HashMap<String, NodeId> = HashMap::new();
    // (module id, port name, node, param id, base, lo, hi) — "mod" targets.
    let mut mod_targets: Vec<(String, NodeId, u32, f64, f64, f64)> = Vec::new();

    for m in &patch.modules {
        match m.kind.as_str() {
            "osc" => {
                let mut osc = Oscillator::new(wave_from_idx(param(m, "wave", 2.0))).with_voice(voice_id);
                osc.detune = param(m, "detune", 0.0);
                nodes.insert(m.id.clone(), g.add_node(Box::new(osc)));
            }
            "noise" => {
                nodes.insert(m.id.clone(), g.add_node(Box::new(Noise::new())));
            }
            "filter" => {
                let ftype = match param(m, "type", 0.0) as i64 {
                    1 => FilterType::HighPass,
                    2 => FilterType::BandPass,
                    _ => FilterType::LowPass,
                };
                let n = g.add_node(Box::new(BiquadFilter::new(ftype, param(m, "cutoff", 2000.0), param(m, "res", 1.0))));
                nodes.insert(m.id.clone(), n);
                mod_targets.push((m.id.clone(), n, 0, param(m, "cutoff", 2000.0), 20.0, 20000.0));
            }
            "drive" => {
                let n = g.add_node(Box::new(Distortion::new()));
                g.set_node_param(n, 1, param(m, "drive", 2.0));
                nodes.insert(m.id.clone(), n);
                mod_targets.push((m.id.clone(), n, 1, param(m, "drive", 2.0), 0.25, 24.0));
            }
            "gain" => {
                let n = g.add_node(Box::new(MonoGain::new(param(m, "level", 0.8) as f32)));
                nodes.insert(m.id.clone(), n);
                mod_targets.push((m.id.clone(), n, 0, param(m, "level", 0.8), 0.0, 2.0));
            }
            "mixer" => {
                let n = g.add_node(Box::new(MonoCrossfade::new(param(m, "balance", 0.5).clamp(0.0, 1.0) as f32)));
                nodes.insert(m.id.clone(), n);
            }
            "env" => {
                let n = g.add_node(Box::new(Adsr::new(
                    param(m, "attack", 0.005),
                    param(m, "decay", 0.2),
                    param(m, "sustain", 0.6),
                    param(m, "release", 0.3),
                ).with_voice(voice_id)));
                nodes.insert(m.id.clone(), n);
            }
            "lfo" => { /* analytic — no node */ }
            "out" => { /* terminal */ }
            other => return Err(format!("grid: unknown module kind `{other}`")),
        }
    }

    // Wire audio cables. Input port index per kind: audio inputs are 0/1.
    for c in &patch.cables {
        let from_node = nodes.get(&c.from.0).copied();
        let to_node = nodes.get(&c.to.0).copied();
        let (Some(fn0), Some(tn0)) = (from_node, to_node) else { continue };
        // Skip "mod" ports — those become modulations, not audio edges.
        if c.to.1 == "mod" {
            continue;
        }
        let to_port: u16 = match (get(&c.to.0)?.kind.as_str(), c.to.1.as_str()) {
            ("mixer", "a") => 0,
            ("mixer", "b") => 1,
            _ => 0,
        };
        g.connect(fn0, 0, tn0, to_port);
    }

    // Resolve the output: the module feeding `out.in`.
    let mut out_node: Option<NodeId> = None;
    for c in &patch.cables {
        if c.to.0 == "out" {
            if let Some(n) = nodes.get(&c.from.0).copied() {
                out_node = Some(n);
            }
        }
    }
    if out_node.is_none() {
        out_node = nodes.values().copied().next();
    }
    let out = out_node.ok_or_else(|| "grid: patch has no output".to_string())?;

    // Resolve modulation cables → ModBinding (source + target).
    let mut modulations = Vec::new();
    for c in &patch.cables {
        if c.to.1 != "mod" {
            continue;
        }
        let Some((_, tn, pid, base, lo, hi)) = mod_targets.iter().find(|(mid, _, _, _, _, _)| *mid == c.to.0).cloned() else {
            continue;
        };
        let Some(src_mod) = patch.modules.iter().find(|m| m.id == c.from.0) else { continue };
        let source = match src_mod.kind.as_str() {
            "lfo" => ModSource::Lfo {
                rate: param(src_mod, "rate", 1.0),
                depth: param(src_mod, "depth", 0.5),
                wave: param(src_mod, "wave", 0.0) as i64,
            },
            "env" => ModSource::Env {
                attack: param(src_mod, "attack", 0.005),
                decay: param(src_mod, "decay", 0.2),
                sustain: param(src_mod, "sustain", 0.6),
                release: param(src_mod, "release", 0.3),
            },
            _ => continue,
        };
        modulations.push(ModBinding { target_node: tn, param_id: pid, base, lo, hi, source });
    }

    Ok(CompiledGrid { out, modulations })
}

/// Value of a modulation source at time `t` (seconds).
/// `notes` is `(on_seconds, off_seconds)` for the gate of the env source.
pub fn source_value(src: &ModSource, notes: &[(f64, f64)], t: f64) -> f64 {
    match src {
        ModSource::Lfo { rate, depth, wave } => {
            let ph = t * rate * std::f64::consts::TAU;
            let v = match wave {
                1 => (2.0 / std::f64::consts::PI) * ph.sin().asin(),
                2 => if ph.sin() >= 0.0 { 1.0 } else { -1.0 },
                _ => ph.sin(),
            };
            v * depth
        }
        ModSource::Env { attack, decay, sustain, release } => {
            adsr_level(notes, *attack, *decay, *sustain, *release, t)
        }
    }
}

/// A resolved modulation binding (no graph nodes — used for segment rendering).
pub struct GridMod {
    pub module_id: String,
    pub param_key: &'static str,
    pub base: f64,
    pub lo: f64,
    pub hi: f64,
    pub source: ModSource,
}

/// Resolve every control cable into a `(target module, param, source)` binding.
pub fn grid_modulations(patch: &GridPatch) -> Vec<GridMod> {
    let mut out = Vec::new();
    for c in &patch.cables {
        if c.to.1 != "mod" {
            continue;
        }
        let target = patch.modules.iter().find(|m| m.id == c.to.0);
        let src = patch.modules.iter().find(|m| m.id == c.from.0);
        let (Some(t), Some(s)) = (target, src) else { continue };
        let (param_key, lo, hi) = match t.kind.as_str() {
            "filter" => ("cutoff", 20.0, 20000.0),
            "drive" => ("drive", 0.25, 24.0),
            "gain" => ("level", 0.0, 2.0),
            _ => continue,
        };
        let source = match s.kind.as_str() {
            "lfo" => ModSource::Lfo {
                rate: param(s, "rate", 1.0),
                depth: param(s, "depth", 0.5),
                wave: param(s, "wave", 0.0) as i64,
            },
            "env" => ModSource::Env {
                attack: param(s, "attack", 0.005),
                decay: param(s, "decay", 0.2),
                sustain: param(s, "sustain", 0.6),
                release: param(s, "release", 0.3),
            },
            _ => continue,
        };
        out.push(GridMod { module_id: t.id.clone(), param_key, base: param(t, param_key, 0.0), lo, hi, source });
    }
    out
}

/// Write a modulated value into a grid module's parameter.
pub fn apply_grid_mod(patch: &mut GridPatch, module_id: &str, param_key: &str, value: f64) {
    if let Some(m) = patch.modules.iter_mut().find(|m| m.id == module_id) {
        m.params.insert(param_key.to_string(), value);
    }
}

/// ADSR envelope level (0..1) at time `t`, gated by the most recent note.
pub fn adsr_level(notes: &[(f64, f64)], attack: f64, decay: f64, sustain: f64, release: f64, t: f64) -> f64 {
    let mut level = 0.0;
    for &(on, off) in notes {
        if t < on {
            continue;
        }
        if t <= off {
            let dt = t - on;
            if dt < attack {
                level = if attack > 0.0 { dt / attack } else { 1.0 };
            } else if dt < attack + decay {
                level = if decay > 0.0 { 1.0 - (1.0 - sustain) * ((dt - attack) / decay) } else { sustain };
            } else {
                level = sustain;
            }
        } else {
            let dt = t - off;
            level = if release > 0.0 { sustain * (1.0 - dt / release).max(0.0) } else { 0.0 };
        }
    }
    level
}
