//! The self-describing catalog: every instrument kind, effect, Grid module and
//! tuning the engine supports, with ranges and defaults.
//!
//! This exists so **the AI never has to guess**. `studio_catalog` (tool) and
//! `GET /api/studio/catalog` (route) return the same JSON, and the Studio UI
//! can build its device panels from it too — which is what keeps the frontend
//! and the renderer from drifting apart.

use serde_json::{json, Value};

use crate::engine::TUNINGS;
use crate::fx;
use crate::grid;
use crate::voices::{self, ParamDef};

fn param_json(d: &ParamDef) -> Value {
    json!({
        "key": d.key,
        "label": d.label,
        "group": d.group,
        "min": d.min,
        "max": d.max,
        "step": d.step,
        "default": d.default,
        "log": d.log,
        "choices": d.choices,
    })
}

fn params_json(defs: &[ParamDef]) -> Value {
    Value::Array(defs.iter().map(param_json).collect())
}

/// The full catalog as JSON.
pub fn catalog_json() -> Value {
    let kinds: Vec<Value> = voices::KINDS
        .iter()
        .map(|kind| {
            let category = if voices::is_drum(kind) {
                "drum"
            } else if voices::is_melodic(kind) {
                "synth"
            } else {
                "special"
            };
            json!({
                "kind": kind,
                "category": category,
                "default_level": voices::default_level(kind),
                "default_pan": voices::default_pan(kind),
                "params": params_json(voices::param_defs_for(kind)),
                "defaults": voices::kind_params(kind),
            })
        })
        .collect();

    let effects: Vec<Value> = fx::EFFECT_KINDS
        .iter()
        .map(|kind| {
            json!({
                "kind": kind,
                "label": fx::label(kind),
                "params": params_json(fx::param_defs(kind)),
                "defaults": fx::defaults(kind),
            })
        })
        .collect();

    let grid_modules: Vec<Value> = grid::MODULE_KINDS
        .iter()
        .map(|kind| {
            json!({
                "kind": kind,
                "params": params_json(grid::module_params(kind)),
                "inputs": grid::module_inputs(kind),
                "outputs": grid::module_outputs(kind),
                "has_mod": grid::module_has_mod(kind),
            })
        })
        .collect();

    let midi_fx: Vec<Value> = voices::MIDI_FX_KINDS
        .iter()
        .map(|kind| {
            json!({
                "kind": kind,
                "params": params_json(voices::midi_fx_defs(kind)),
            })
        })
        .collect();

    json!({
        "kinds": kinds,
        "effects": effects,
        "grid_modules": grid_modules,
        "midi_fx": midi_fx,
        "tunings": TUNINGS,
        "master_fx": [
            {"key": "delay_mix", "min": 0.0, "max": 1.0, "default": 0.0, "help": "Master delay send"},
            {"key": "delay_time", "min": 1.0, "max": 4000.0, "default": 250.0, "help": "Master delay time (ms)"},
            {"key": "feedback", "min": 0.0, "max": 0.95, "default": 0.4},
            {"key": "delay_ping_pong", "min": 0.0, "max": 1.0, "default": 0.45},
            {"key": "delay_damp", "min": 0.0, "max": 1.0, "default": 0.4},
            {"key": "reverb_mix", "min": 0.0, "max": 1.0, "default": 0.0},
            {"key": "reverb_size", "min": 0.05, "max": 1.6, "default": 0.5},
            {"key": "reverb_damp", "min": 0.0, "max": 1.0, "default": 0.5},
            {"key": "reverb_predelay", "min": 0.0, "max": 250.0, "default": 12.0},
            {"key": "reverb_width", "min": 0.0, "max": 2.0, "default": 1.0},
            {"key": "master_gain", "min": 0.0, "max": 2.0, "default": 1.0},
            {"key": "master_drive", "min": 0.0, "max": 1.0, "default": 0.0, "help": "Bus saturation"},
            {"key": "master_width", "min": 0.0, "max": 2.0, "default": 1.0},
            {"key": "glue", "min": 0.0, "max": 1.0, "default": 0.0, "help": "Parallel bus compression"},
            {"key": "ceiling", "min": -6.0, "max": 0.0, "default": -0.4, "help": "Limiter ceiling (dBFS)"},
            {"key": "loudness", "min": -30.0, "max": 0.0, "default": 0.0, "help": "Target integrated LUFS (-14 is streaming-normal); 0 disables"},
        ],
        "limits": {
            "max_voices": voices::MAX_VOICES,
            "max_effects_per_voice": 8,
            "max_midi_fx_per_voice": 8,
            "max_tracks": 32,
            "max_clips": 256,
            "steps": [4, 8, 16, 32, 64],
            "sample_rate": crate::engine::SAMPLE_RATE as u32,
        },
    })
}

/// A compact, human/LLM-readable description of one instrument kind.
pub fn describe_kind(kind: &str) -> Option<Value> {
    if !voices::is_kind(kind) {
        return None;
    }
    let category = if voices::is_drum(kind) {
        "drum"
    } else if voices::is_melodic(kind) {
        "synth"
    } else {
        "special"
    };
    Some(json!({
        "kind": kind,
        "category": category,
        "params": params_json(voices::param_defs_for(kind)),
        "defaults": voices::kind_params(kind),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_covers_every_kind_and_effect() {
        let c = catalog_json();
        let kinds = c["kinds"].as_array().unwrap();
        assert_eq!(kinds.len(), voices::KINDS.len());
        let effects = c["effects"].as_array().unwrap();
        assert_eq!(effects.len(), fx::EFFECT_KINDS.len());
        let modules = c["grid_modules"].as_array().unwrap();
        assert_eq!(modules.len(), grid::MODULE_KINDS.len());
        // Every parametric kind/effect/module must expose at least one param.
        for k in kinds {
            let name = k["kind"].as_str().unwrap();
            if matches!(name, "drumkit" | "grid") {
                continue;
            }
            assert!(!k["params"].as_array().unwrap().is_empty(), "{name} has no params");
        }
        for e in effects {
            assert!(!e["params"].as_array().unwrap().is_empty());
        }
        assert!(c["master_fx"].as_array().unwrap().len() > 10);
    }
}
