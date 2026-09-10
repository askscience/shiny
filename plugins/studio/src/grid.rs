//! The Grid (WaveMe) — a modular patch that runs on the plugin's own DSP.
//!
//! A patch is a list of modules connected by cables. Audio cables carry a mono
//! signal per module; control cables (to a module's `mod` port) modulate one
//! parameter at control rate, ramped across each block so the modulation is
//! smooth instead of stepped.
//!
//! Unlike the previous implementation — which re-rendered the whole pattern
//! once per step to fake modulation — modules run continuously and keep their
//! state, so a filter sweep is a real filter sweep rather than sixteen
//! crossfaded renders.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::dsp::delay::{Chorus, StereoDelay};
use super::dsp::env::Adsr;
use super::dsp::filter::{Filter, FilterKind};
use super::dsp::fx::Distortion;
use super::dsp::lfo::{Lfo, LfoShape};
use super::dsp::noise::{Noise, NoiseKind};
use super::dsp::osc::{Osc, Wave};
use super::dsp::reverb::Reverb;
use super::dsp::util::ramp;
use super::dsp::BLOCK;

/* ── the JSON contract ─────────────────────────────────────────── */

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

/// A patch cord. `amount` scales control connections (default 1) and is
/// ignored for audio connections.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GridCable {
    pub from: (String, String),
    pub to: (String, String),
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<f64>,
}

/// A complete modular patch.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct GridPatch {
    pub modules: Vec<GridModule>,
    pub cables: Vec<GridCable>,
}

/// Module kinds the Grid understands, in palette order.
pub const MODULE_KINDS: &[&str] = &[
    "osc", "noise", "filter", "drive", "gain", "mixer", "env", "lfo", "delay", "reverb", "chorus", "out",
];

/* ── module catalog (palette, inspector, AI docs) ───────────────── */

use crate::voices::{choice as pchoice, def as pdef, logdef as plogdef, ParamDef};

const WAVES: &[&str] = &["Sine", "Triangle", "Saw", "Square", "Pulse", "Noise", "Organ"];
const FILTER_TYPES: &[&str] = &["LP", "HP", "BP", "Notch", "Peak"];
const LFO_WAVES: &[&str] = &["Sine", "Triangle", "Saw", "Ramp", "Square", "S&H", "Random"];
const NOISE_KINDS: &[&str] = &["White", "Pink", "Brown", "Bright"];

static OSC_PARAMS: &[ParamDef] = &[
    pchoice("wave", "Wave", "Osc", 2.0, WAVES),
    plogdef("level", "Level", "Osc", 0.0, 2.0, 0.01, 1.0),
    pdef("octave", "Octave", "Osc", -3.0, 3.0, 1.0, 0.0),
    pdef("semi", "Semi", "Osc", -24.0, 24.0, 1.0, 0.0),
    pdef("detune", "Detune", "Osc", -48.0, 48.0, 0.1, 0.0),
    pdef("pw", "Pulse Width", "Osc", 0.05, 0.95, 0.01, 0.5),
];
static NOISE_PARAMS: &[ParamDef] = &[
    pchoice("kind", "Kind", "Noise", 0.0, NOISE_KINDS),
    plogdef("level", "Level", "Noise", 0.0, 2.0, 0.01, 1.0),
];
static FILTER_PARAMS: &[ParamDef] = &[
    pchoice("type", "Type", "Filter", 0.0, FILTER_TYPES),
    plogdef("cutoff", "Cutoff", "Filter", 20.0, 20000.0, 10.0, 2000.0),
    pdef("res", "Resonance", "Filter", 0.4, 20.0, 0.1, 1.0),
    pdef("drive", "Drive", "Filter", 1.0, 24.0, 0.05, 1.0),
    pdef("poles", "Slope", "Filter", 1.0, 2.0, 1.0, 1.0),
    pdef("env", "Env Follow", "Filter", -4.0, 4.0, 0.05, 0.0),
];
static DRIVE_PARAMS: &[ParamDef] = &[
    pdef("drive", "Drive", "Drive", 0.25, 24.0, 0.05, 2.0),
    pdef("mix", "Mix", "Drive", 0.0, 1.0, 0.01, 0.6),
    plogdef("tone", "Tone", "Drive", 800.0, 20000.0, 10.0, 12000.0),
    pdef("out", "Out", "Drive", 0.1, 3.0, 0.05, 1.0),
    pdef("mode", "Mode", "Drive", 0.0, 4.0, 1.0, 0.0),
];
static GAIN_PARAMS: &[ParamDef] = &[pdef("level", "Level", "Gain", 0.0, 2.0, 0.01, 0.8)];
static MIXER_PARAMS: &[ParamDef] = &[pdef("balance", "Balance", "Mixer", 0.0, 1.0, 0.01, 0.5)];
static ENV_PARAMS: &[ParamDef] = &[
    pdef("attack", "Attack", "Envelope", 0.001, 5.0, 0.005, 0.005),
    pdef("decay", "Decay", "Envelope", 0.001, 5.0, 0.01, 0.2),
    pdef("sustain", "Sustain", "Envelope", 0.0, 1.0, 0.01, 0.6),
    pdef("release", "Release", "Envelope", 0.001, 5.0, 0.01, 0.3),
    pdef("depth", "Depth", "Envelope", 0.0, 4.0, 0.01, 1.0),
];
static LFO_PARAMS: &[ParamDef] = &[
    plogdef("rate", "Rate", "LFO", 0.01, 40.0, 0.01, 1.0),
    pchoice("wave", "Shape", "LFO", 0.0, LFO_WAVES),
    pdef("depth", "Depth", "LFO", 0.0, 4.0, 0.01, 0.5),
];
static DELAY_PARAMS: &[ParamDef] = &[
    pdef("time", "Time", "Delay", 1.0, 4000.0, 1.0, 250.0),
    pdef("feedback", "Feedback", "Delay", 0.0, 0.95, 0.01, 0.4),
    pdef("mix", "Mix", "Delay", 0.0, 1.0, 0.01, 0.35),
    pdef("ping_pong", "Ping-Pong", "Delay", 0.0, 1.0, 0.01, 0.4),
    pdef("damp", "Damping", "Delay", 0.0, 1.0, 0.01, 0.4),
];
static REVERB_PARAMS: &[ParamDef] = &[
    pdef("size", "Size", "Reverb", 0.05, 1.6, 0.01, 0.5),
    pdef("damping", "Damping", "Reverb", 0.0, 1.0, 0.01, 0.5),
    pdef("mix", "Mix", "Reverb", 0.0, 1.0, 0.01, 0.3),
    pdef("predelay", "Pre-Delay", "Reverb", 0.0, 250.0, 1.0, 12.0),
    pdef("width", "Width", "Reverb", 0.0, 2.0, 0.01, 1.0),
];
static CHORUS_PARAMS: &[ParamDef] = &[
    plogdef("rate", "Rate", "Chorus", 0.01, 10.0, 0.01, 0.6),
    pdef("depth", "Depth", "Chorus", 0.0, 1.0, 0.01, 0.5),
    pdef("mix", "Mix", "Chorus", 0.0, 1.0, 0.01, 0.4),
    pdef("spread", "Spread", "Chorus", 0.0, 1.0, 0.01, 0.6),
];
static OUT_PARAMS: &[ParamDef] = &[pdef("level", "Level", "Out", 0.0, 2.0, 0.01, 0.8)];

/// Parameter catalog for a Grid module kind.
pub fn module_params(kind: &str) -> &'static [ParamDef] {
    match kind {
        "osc" => OSC_PARAMS,
        "noise" => NOISE_PARAMS,
        "filter" => FILTER_PARAMS,
        "drive" => DRIVE_PARAMS,
        "gain" => GAIN_PARAMS,
        "mixer" => MIXER_PARAMS,
        "env" => ENV_PARAMS,
        "lfo" => LFO_PARAMS,
        "delay" => DELAY_PARAMS,
        "reverb" => REVERB_PARAMS,
        "chorus" => CHORUS_PARAMS,
        "out" => OUT_PARAMS,
        _ => &[],
    }
}

/// Audio input ports for a module kind.
pub fn module_inputs(kind: &str) -> &'static [&'static str] {
    match kind {
        "osc" | "noise" | "lfo" => &[],
        "mixer" => &["a", "b"],
        "out" => &["in"],
        _ => &["in"],
    }
}

/// Audio output ports for a module kind.
pub fn module_outputs(kind: &str) -> &'static [&'static str] {
    match kind {
        "lfo" | "env" => &["out", "ctrl"],
        "out" => &[],
        _ => &["out"],
    }
}

/// Whether a module's `mod` port accepts a control cable.
pub fn module_has_mod(kind: &str) -> bool {
    matches!(kind, "filter" | "drive" | "gain" | "mixer" | "osc")
}

/* ── compiled nodes ────────────────────────────────────────────── */

enum Node {
    Osc(Box<OscNode>),
    Noise(Box<NoiseNode>),
    Filter(Box<FilterNode>),
    Drive(Box<DriveNode>),
    Gain(GainNode),
    Mixer(MixerNode),
    Env(Box<EnvNode>),
    Lfo(Box<LfoNode>),
    Delay(Box<DelayNode>),
    Reverb(Box<ReverbNode>),
    Chorus(Box<ChorusNode>),
    Out(OutNode),
}

struct OscNode {
    osc: Osc,
    wave: Wave,
    level: f64,
    octave: f64,
    semi: f64,
    detune: f64,
    pw: f64,
    freq: f64,
}
struct NoiseNode {
    noise: Noise,
    kind: NoiseKind,
    level: f64,
}
struct FilterNode {
    f: Filter,
    /// Unmodulated cutoff in Hz — modulation is applied relative to this.
    base_cutoff: f64,
    /// Envelope-follow amount (0 = none).
    env_amount: f64,
    env: f64,
}
struct DriveNode {
    d: Distortion,
}
struct GainNode {
    level: f64,
}
struct MixerNode {
    balance: f64,
}
struct EnvNode {
    env: Adsr,
    depth: f64,
}
struct LfoNode {
    lfo: Lfo,
    rate: f64,
    depth: f64,
    value: f64,
}
struct DelayNode {
    d: StereoDelay,
    mix: f64,
}
struct ReverbNode {
    r: Reverb,
    mix: f64,
}
struct ChorusNode {
    c: Chorus,
    mix: f64,
}
struct OutNode {
    level: f64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CtrlTarget {
    Cutoff,
    Drive,
    Gain,
    Balance,
    Detune,
}

/// A resolved control connection.
struct Ctrl {
    to_module: usize,
    target: CtrlTarget,
    from_module: usize,
    amount: f64,
    /// The target's value with no modulation.
    base: f64,
}

fn p(m: &GridModule, key: &str, default: f64) -> f64 {
    m.params.get(key).copied().unwrap_or(default)
}

/// The compiled, running patch.
pub struct GridEngine {
    nodes: Vec<Node>,
    ids: HashMap<String, usize>,
    /// Execution order (topological over audio cables).
    order: Vec<usize>,
    /// Audio inputs per module: (input port, source module).
    inputs: Vec<Vec<(usize, usize)>>,
    ctrls: Vec<Ctrl>,
    /// Per-module scratch buffers.
    bufs: Vec<[f32; BLOCK]>,
    out: Option<usize>,
    has_env: bool,
    freq: f64,
    gate: bool,
    velocity: f64,
    sr: f64,
}

impl GridEngine {
    pub fn compile(patch: &GridPatch, seed: u64, sr: f64) -> Result<GridEngine, String> {
        if patch.modules.is_empty() {
            return Err("grid: patch has no modules".into());
        }
        if patch.modules.len() > 64 {
            return Err("grid: too many modules (max 64)".into());
        }
        let mut ids: HashMap<String, usize> = HashMap::new();
        for (i, m) in patch.modules.iter().enumerate() {
            ids.insert(m.id.clone(), i);
        }

        let mut nodes: Vec<Node> = Vec::with_capacity(patch.modules.len());
        for (i, m) in patch.modules.iter().enumerate() {
            let s = seed ^ ((i as u64 + 1).wrapping_mul(0x9E37_79B9));
            let node = match m.kind.as_str() {
                "osc" => {
                    let wave = Wave::from_index(p(m, "wave", 2.0));
                    Node::Osc(Box::new(OscNode {
                        osc: Osc::new(wave).with_seed(s),
                        wave,
                        level: p(m, "level", 1.0),
                        octave: p(m, "octave", 0.0),
                        semi: p(m, "semi", 0.0),
                        detune: p(m, "detune", 0.0),
                        pw: p(m, "pw", 0.5),
                        freq: 440.0,
                    }))
                }
                "noise" => {
                    let kind = NoiseKind::from_index(p(m, "kind", 0.0));
                    let mut noise = Noise::new(s);
                    noise.set_kind(kind);
                    Node::Noise(Box::new(NoiseNode { noise, kind, level: p(m, "level", 1.0) }))
                }
                "filter" => {
                    let kind = FilterKind::from_index(p(m, "type", 0.0));
                    let cutoff = p(m, "cutoff", 2000.0).clamp(20.0, 20000.0);
                    let mut f = Filter::new(kind, cutoff, p(m, "res", 1.0));
                    f.set_drive(p(m, "drive", 1.0));
                    f.set_poles(p(m, "poles", 1.0).round().clamp(1.0, 2.0) as u8);
                    Node::Filter(Box::new(FilterNode { f, base_cutoff: cutoff, env_amount: p(m, "env", 0.0), env: 0.0 }))
                }
                "drive" => Node::Drive(Box::new(DriveNode { d: Distortion::new(&m.params) })),
                "gain" => Node::Gain(GainNode { level: p(m, "level", 0.8) }),
                "mixer" => Node::Mixer(MixerNode { balance: p(m, "balance", 0.5) }),
                "env" => {
                    let mut env = Adsr::new(
                        p(m, "attack", 0.005),
                        p(m, "decay", 0.2),
                        p(m, "sustain", 0.6),
                        p(m, "release", 0.3),
                    );
                    env.set_sample_rate(sr);
                    Node::Env(Box::new(EnvNode { env, depth: p(m, "depth", 1.0) }))
                }
                "lfo" => Node::Lfo(Box::new(LfoNode {
                    lfo: Lfo::new(LfoShape::from_index(p(m, "wave", 0.0)), s),
                    rate: p(m, "rate", 1.0),
                    depth: p(m, "depth", 0.5),
                    value: 0.0,
                })),
                "delay" => {
                    let mix = p(m, "mix", 0.35);
                    let mut d = StereoDelay::new(p(m, "time", 250.0), p(m, "feedback", 0.4), mix);
                    d.set_sample_rate(sr);
                    d.ping_pong = p(m, "ping_pong", 0.4);
                    d.damp = p(m, "damp", 0.4);
                    d.update_damping();
                    Node::Delay(Box::new(DelayNode { d, mix }))
                }
                "reverb" => {
                    let mix = p(m, "mix", 0.3);
                    let mut r = Reverb::new(p(m, "size", 0.5), p(m, "damping", 0.5), mix);
                    r.set_sample_rate(sr);
                    r.predelay_ms = p(m, "predelay", 12.0);
                    r.width = p(m, "width", 1.0);
                    r.update_damping();
                    Node::Reverb(Box::new(ReverbNode { r, mix }))
                }
                "chorus" => {
                    let mix = p(m, "mix", 0.4);
                    let mut c = Chorus::new(p(m, "rate", 0.6), p(m, "depth", 0.5), mix);
                    c.set_sample_rate(sr);
                    c.spread = p(m, "spread", 0.6);
                    Node::Chorus(Box::new(ChorusNode { c, mix }))
                }
                "out" => Node::Out(OutNode { level: p(m, "level", 0.8) }),
                other => return Err(format!("grid: unknown module kind `{other}`")),
            };
            nodes.push(node);
        }

        // Resolve cables into audio inputs and control bindings.
        let mut inputs: Vec<Vec<(usize, usize)>> = vec![Vec::new(); nodes.len()];
        let mut ctrls: Vec<Ctrl> = Vec::new();
        let mut edges: Vec<(usize, usize)> = Vec::new();
        for c in &patch.cables {
            let (Some(&from), Some(&to)) = (ids.get(&c.from.0), ids.get(&c.to.0)) else { continue };
            let amount = c.amount.unwrap_or(1.0).clamp(-4.0, 4.0);
            if c.to.1 == "mod" {
                let target = match &nodes[to] {
                    Node::Filter(_) => CtrlTarget::Cutoff,
                    Node::Drive(_) => CtrlTarget::Drive,
                    Node::Gain(_) => CtrlTarget::Gain,
                    Node::Mixer(_) => CtrlTarget::Balance,
                    Node::Osc(_) => CtrlTarget::Detune,
                    _ => continue,
                };
                let base = match (&nodes[to], target) {
                    (Node::Filter(f), CtrlTarget::Cutoff) => f.base_cutoff.log2(),
                    (Node::Gain(g), CtrlTarget::Gain) => g.level,
                    (Node::Mixer(m), CtrlTarget::Balance) => m.balance,
                    (Node::Osc(o), CtrlTarget::Detune) => o.detune,
                    (Node::Drive(_), CtrlTarget::Drive) => 2.0,
                    _ => 0.0,
                };
                ctrls.push(Ctrl { to_module: to, target, from_module: from, amount, base });
                continue;
            }
            let port = match (&nodes[to], c.to.1.as_str()) {
                (Node::Mixer(_), "b") => 1,
                _ => 0,
            };
            inputs[to].push((port, from));
            edges.push((from, to));
        }

        // Kahn topological sort; any module left in a cycle is appended in
        // declaration order (a feedback patch still runs, just one block late).
        let n = nodes.len();
        let mut indeg = vec![0usize; n];
        let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
        for (a, b) in &edges {
            adj[*a].push(*b);
            indeg[*b] += 1;
        }
        let mut queue: Vec<usize> = (0..n).filter(|i| indeg[*i] == 0).collect();
        let mut order = Vec::with_capacity(n);
        while let Some(i) = queue.pop() {
            order.push(i);
            for &j in &adj[i] {
                indeg[j] -= 1;
                if indeg[j] == 0 {
                    queue.push(j);
                }
            }
        }
        if order.len() != n {
            for i in 0..n {
                if !order.contains(&i) {
                    order.push(i);
                }
            }
        }

        let mut out = None;
        for c in &patch.cables {
            if c.to.0 == "out" {
                if let Some(&from) = ids.get(&c.from.0) {
                    out = Some(from);
                }
            }
        }
        let out = out.or_else(|| (0..n).rev().find(|i| !matches!(nodes[*i], Node::Out(_))));
        let has_env = nodes.iter().any(|nd| matches!(nd, Node::Env(_)));
        let bufs: Vec<[f32; BLOCK]> = (0..nodes.len()).map(|_| [0.0f32; BLOCK]).collect();

        Ok(GridEngine {
            nodes,
            ids,
            order,
            inputs,
            ctrls,
            bufs,
            out,
            has_env,
            freq: 440.0,
            gate: false,
            velocity: 1.0,
            sr,
        })
    }

    pub fn note_on(&mut self, freq: f64, velocity: f64) {
        self.freq = freq;
        self.gate = true;
        self.velocity = velocity.clamp(0.05, 1.6);
        for node in self.nodes.iter_mut() {
            match node {
                Node::Osc(o) => {
                    o.osc.reset();
                    o.freq = freq;
                }
                Node::Env(e) => e.env.gate_on(),
                _ => {}
            }
        }
    }

    pub fn note_off(&mut self) {
        self.gate = false;
        for node in self.nodes.iter_mut() {
            if let Node::Env(e) = node {
                e.env.gate_off();
            }
        }
    }

    pub fn reset(&mut self) {
        for node in self.nodes.iter_mut() {
            match node {
                Node::Osc(o) => o.osc.reset(),
                Node::Noise(n) => n.noise.reset(),
                Node::Filter(f) => f.f.reset(),
                Node::Env(e) => e.env.reset(),
                Node::Lfo(l) => l.lfo.reset(),
                Node::Delay(d) => d.d.reset(),
                Node::Reverb(r) => r.r.reset(),
                Node::Chorus(c) => c.c.reset(),
                _ => {}
            }
        }
        self.gate = false;
    }

    /// Still producing sound? Patches with no envelope are always considered
    /// active so the render loop keeps feeding them.
    pub fn is_active(&self) -> bool {
        if self.gate || !self.has_env {
            return true;
        }
        self.nodes.iter().any(|n| matches!(n, Node::Env(e) if e.env.is_active()))
    }

    /// Set a module parameter by module id and catalog key.
    pub fn set_param(&mut self, module_id: &str, key: &str, value: f64) {
        let Some(&i) = self.ids.get(module_id) else { return };
        self.set_param_index(i, key, value);
    }

    fn set_param_index(&mut self, i: usize, key: &str, value: f64) {
        match &mut self.nodes[i] {
            Node::Osc(o) => match key {
                "wave" => {
                    o.wave = Wave::from_index(value);
                    o.osc.wave = o.wave;
                }
                "level" => o.level = value,
                "octave" => o.octave = value,
                "semi" => o.semi = value,
                "detune" => o.detune = value,
                "pw" => {
                    o.pw = value;
                    o.osc.pulse_width = value;
                }
                _ => {}
            },
            Node::Noise(n) => match key {
                "level" => n.level = value,
                "kind" => {
                    n.kind = NoiseKind::from_index(value);
                    n.noise.set_kind(n.kind);
                }
                _ => {}
            },
            Node::Filter(f) => match key {
                "cutoff" => {
                    f.base_cutoff = value.clamp(20.0, 20000.0);
                    f.f.set_cutoff(f.base_cutoff);
                }
                "res" => f.f.set_q(value),
                "drive" => f.f.set_drive(value),
                "type" => f.f.kind = FilterKind::from_index(value),
                "poles" => f.f.set_poles(value.round().clamp(1.0, 2.0) as u8),
                "env" => f.env_amount = value,
                _ => {}
            },
            Node::Drive(d) => {
                if key == "drive" {
                    d.d.drive = value.clamp(0.25, 24.0);
                } else if key == "mix" {
                    d.d.mix = value.clamp(0.0, 1.0);
                } else if key == "mode" {
                    d.d.mode = value;
                } else if key == "out" {
                    d.d.out = value.clamp(0.05, 4.0);
                } else if key == "tone" {
                    d.d.tone = value.clamp(200.0, 20000.0);
                    d.d.retune();
                }
            }
            Node::Gain(g) => {
                if key == "level" {
                    g.level = value;
                }
            }
            Node::Mixer(m) => {
                if key == "balance" {
                    m.balance = value;
                }
            }
            Node::Env(e) => match key {
                "attack" | "decay" | "sustain" | "release" => {
                    let (a, d, s, r) = e.env.times();
                    let next = match key {
                        "attack" => (value, d, s, r),
                        "decay" => (a, value, s, r),
                        "sustain" => (a, d, value, r),
                        _ => (a, d, s, value),
                    };
                    e.env.set_times(next.0, next.1, next.2, next.3);
                }
                "depth" => e.depth = value,
                _ => {}
            },
            Node::Lfo(l) => match key {
                "rate" => l.rate = value,
                "depth" => l.depth = value,
                "wave" => l.lfo.shape = LfoShape::from_index(value),
                _ => {}
            },
            Node::Delay(d) => {
                match key {
                    "time" => d.d.time_ms = value.clamp(1.0, 4000.0),
                    "feedback" => d.d.feedback = value.clamp(0.0, 0.98),
                    "mix" => d.mix = value.clamp(0.0, 1.0),
                    "ping_pong" => d.d.ping_pong = value.clamp(0.0, 1.0),
                    "damp" => {
                        d.d.damp = value.clamp(0.0, 1.0);
                        d.d.update_damping();
                    }
                    "mod" => d.d.mod_depth = value,
                    "offset" => d.d.offset_ms = value,
                    _ => {}
                }
            }
            Node::Reverb(r) => {
                match key {
                    "size" => r.r.size = value,
                    "damping" | "damp" => {
                        r.r.damping = value;
                        r.r.update_damping();
                    }
                    "mix" => r.r.mix = value,
                    "predelay" => r.r.predelay_ms = value,
                    "width" => r.r.width = value,
                    "mod" => r.r.mod_amount = value,
                    _ => {}
                }
                if key == "mix" {
                    r.mix = value;
                }
            }
            Node::Chorus(c) => {
                match key {
                    "rate" => c.c.rate = value,
                    "depth" => c.c.depth = value,
                    "mix" => c.c.mix = value,
                    "spread" => c.c.spread = value,
                    _ => {}
                }
                if key == "mix" {
                    c.mix = value;
                }
            }
            Node::Out(o) => {
                if key == "level" {
                    o.level = value;
                }
            }
        }
    }

    /// Control source value (bipolar) for the given module.
    fn ctrl_value(&self, i: usize) -> f64 {
        match &self.nodes[i] {
            Node::Env(e) => e.env.level() * e.depth,
            Node::Lfo(l) => l.value * l.depth,
            _ => 0.0,
        }
    }

    /// Render one block, *adding* into the stereo buses.
    pub fn render(&mut self, out_l: &mut [f32], out_r: &mut [f32], frames: usize) {
        let frames = frames.min(BLOCK);
        // Resolve control values once per block (they move at control rate).
        let mods: Vec<(usize, CtrlTarget, f64)> = self
            .ctrls
            .iter()
            .map(|c| {
                let v = self.ctrl_value(c.from_module);
                (c.to_module, c.target, c.base + v * c.amount)
            })
            .collect();
        for &(module, target, value) in &mods {
            if target != CtrlTarget::Cutoff {
                self.apply_ctrl(module, target, value);
            }
        }

        let order = std::mem::take(&mut self.order);
        for &i in &order {
            // Sum audio inputs (mixer keeps the two ports separate).
            let mut a_buf = [0.0f32; BLOCK];
            let mut b_buf = [0.0f32; BLOCK];
            for (port, src) in self.inputs[i].iter() {
                let srcbuf = self.bufs[*src];
                let dst = if *port == 1 { &mut b_buf } else { &mut a_buf };
                for k in 0..frames {
                    dst[k] += srcbuf[k];
                }
            }
            let cutoff = mods.iter().find(|(m, t, _)| *m == i && *t == CtrlTarget::Cutoff).map(|(_, _, v)| *v);
            let mut out = [0.0f32; BLOCK];
            self.process_node(i, &a_buf, &b_buf, &mut out, frames, cutoff);
            self.bufs[i] = out;
        }
        self.order = order;

        if let Some(o) = self.out {
            let src = self.bufs[o];
            for k in 0..frames {
                out_l[k] += src[k];
                out_r[k] += src[k];
            }
        }
    }

    fn apply_ctrl(&mut self, module: usize, target: CtrlTarget, value: f64) {
        match target {
            CtrlTarget::Cutoff => {
                let hz = 2f64.powf(value.clamp(4.3, 14.3));
                if let Node::Filter(f) = &mut self.nodes[module] {
                    f.f.set_cutoff(hz);
                }
            }
            CtrlTarget::Drive => {
                if let Node::Drive(d) = &mut self.nodes[module] {
                    d.d.drive = value.clamp(0.25, 24.0);
                }
            }
            CtrlTarget::Gain => {
                if let Node::Gain(g) = &mut self.nodes[module] {
                    g.level = value.clamp(0.0, 2.0);
                }
            }
            CtrlTarget::Balance => {
                if let Node::Mixer(m) = &mut self.nodes[module] {
                    m.balance = value.clamp(0.0, 1.0);
                }
            }
            CtrlTarget::Detune => {
                if let Node::Osc(o) = &mut self.nodes[module] {
                    o.detune = value.clamp(-48.0, 48.0);
                }
            }
        }
    }

    fn process_node(
        &mut self,
        i: usize,
        a: &[f32; BLOCK],
        b: &[f32; BLOCK],
        out: &mut [f32; BLOCK],
        frames: usize,
        cutoff: Option<f64>,
    ) {
        let sr = self.sr;
        let freq = self.freq;
        let velocity = self.velocity;
        match &mut self.nodes[i] {
            Node::Osc(o) => {
                o.osc.pulse_width = o.pw;
                let f = freq * 2f64.powf(o.octave + o.semi / 12.0) * 2f64.powf(o.detune / 12.0);
                let lvl = o.level as f32;
                for k in 0..frames {
                    out[k] = o.osc.tick(f, sr) * lvl;
                }
            }
            Node::Noise(n) => {
                let lvl = n.level as f32;
                for k in 0..frames {
                    out[k] = n.noise.tick() * lvl;
                }
            }
            Node::Filter(f) => {
                // Modulated cutoff ramps across the block (exponential in Hz).
                let ramp_pair = cutoff.map(|target| {
                    let from = f.base_cutoff.log2();
                    (from, target)
                });
                let env_amount = f.env_amount;
                for k in 0..frames {
                    if let Some((from, to)) = ramp_pair {
                        let v = ramp(from as f32, to as f32, k, frames) as f64;
                        f.f.set_cutoff(2f64.powf(v.clamp(4.3, 14.3)));
                    }
                    // Envelope-follow: the input's own amplitude opens the
                    // filter (a simple, musical "auto-wah").
                    if env_amount != 0.0 {
                        let x = a[k].abs() as f64;
                        f.env += (x - f.env) * 0.002;
                        let target = (f.base_cutoff * 2f64.powf(f.env * env_amount * 4.0)).clamp(20.0, 20000.0);
                        f.f.set_cutoff(target);
                    }
                    out[k] = f.f.tick(a[k]);
                }
            }
            Node::Drive(d) => {
                for k in 0..frames {
                    let (l, _) = d.d.tick(a[k], a[k]);
                    out[k] = l;
                }
            }
            Node::Gain(g) => {
                let lvl = g.level as f32;
                for k in 0..frames {
                    out[k] = a[k] * lvl;
                }
            }
            Node::Mixer(m) => {
                let bal = m.balance.clamp(0.0, 1.0) as f32;
                for k in 0..frames {
                    out[k] = a[k] * (1.0 - bal) + b[k] * bal;
                }
            }
            Node::Env(e) => {
                let d = e.depth as f32;
                let v = velocity as f32;
                for k in 0..frames {
                    out[k] = e.env.tick() as f32 * d * v;
                }
            }
            Node::Lfo(l) => {
                let mut last = 0.0;
                for k in 0..frames {
                    last = l.lfo.tick(l.rate, sr) as f64 * l.depth;
                    out[k] = last as f32;
                }
                l.value = last / l.depth.max(1e-9);
            }
            Node::Delay(d) => {
                let m = d.mix as f32;
                for k in 0..frames {
                    let (l, _) = d.d.tick(a[k], a[k]);
                    out[k] = a[k] * (1.0 - m) + l * m;
                }
            }
            Node::Reverb(r) => {
                let m = r.mix as f32;
                for k in 0..frames {
                    let (l, _) = r.r.tick(a[k], a[k]);
                    out[k] = a[k] * (1.0 - m) + l * m;
                }
            }
            Node::Chorus(c) => {
                let m = c.mix as f32;
                for k in 0..frames {
                    let (l, _) = c.c.tick(a[k], a[k]);
                    out[k] = a[k] * (1.0 - m) + l * m;
                }
            }
            Node::Out(o) => {
                let lvl = o.level as f32;
                for k in 0..frames {
                    out[k] = a[k] * lvl;
                }
            }
        }
    }
}
