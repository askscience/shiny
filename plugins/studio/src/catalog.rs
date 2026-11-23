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

/// Ready-made sounds per voice kind (`{ kind: [{ name, params }] }`), the
/// window's Presets list and something the AI can start from: `params`
/// is what `applyPreset` merges into a voice (`synth`, `fx`, `level`, …).
pub fn factory_presets() -> Value {
    serde_json::from_str(include_str!("factory_presets.json")).unwrap_or_else(|_| json!({}))
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
        "soundfonts": crate::dsp::sampler::list(),
        "tunings": TUNINGS,
        "factory_presets": factory_presets(),
        "master_fx": [
            {"key": "delay_mix", "min": 0.0, "max": 1.0, "default": 0.0, "help": "Pattern: send into the shared delay return (arrangements: use track sends)"},
            {"key": "delay_time", "min": 1.0, "max": 4000.0, "default": 250.0, "help": "Master delay time (ms)"},
            {"key": "feedback", "min": 0.0, "max": 0.95, "default": 0.4},
            {"key": "delay_ping_pong", "min": 0.0, "max": 1.0, "default": 0.45},
            {"key": "delay_damp", "min": 0.0, "max": 1.0, "default": 0.4},
            {"key": "reverb_mix", "min": 0.0, "max": 1.0, "default": 0.0, "help": "Pattern: send into the shared reverb return (arrangements: use track sends)"},
            {"key": "reverb_size", "min": 0.05, "max": 1.6, "default": 0.5},
            {"key": "reverb_damp", "min": 0.0, "max": 1.0, "default": 0.5},
            {"key": "reverb_predelay", "min": 0.0, "max": 250.0, "default": 12.0},
            {"key": "reverb_width", "min": 0.0, "max": 2.0, "default": 1.0},
            {"key": "master_gain", "min": 0.0, "max": 2.0, "default": 1.0},
            {"key": "master_drive", "min": 0.0, "max": 1.0, "default": 0.0, "help": "Bus saturation"},
            {"key": "master_width", "min": 0.0, "max": 2.0, "default": 1.0},
            {"key": "glue", "min": 0.0, "max": 1.0, "default": 0.0, "help": "Parallel bus compression"},
            {"key": "ceiling", "min": -6.0, "max": 0.0, "default": -1.0, "help": "Look-ahead limiter ceiling (dBFS)"},
            {"key": "loudness", "min": -30.0, "max": 0.0, "default": 0.0, "help": "Target integrated LUFS (-14 is streaming-normal); 0 disables"},
            {"key": "sample_rate", "min": 8000.0, "max": 192000.0, "default": 44100.0, "help": "Output sample rate — the engine renders natively at this rate (default 44100)"},
            {"key": "wav_bits", "min": 16.0, "max": 24.0, "default": 16.0, "help": "WAV bit depth: 16 (default) or 24"},
            {"key": "master_oversample", "min": 1.0, "max": 8.0, "default": 2.0, "help": "Oversampling factor for the master bus saturation: 1 (default), 2, 4 or 8"},
        ],
        "returns": {
            "reverb": crate::fx::param_defs("reverb").iter().filter(|d| d.key != "mix").map(|d| json!({"key": d.key, "min": d.min, "max": d.max, "default": d.default})).collect::<Vec<_>>(),
            "delay": [
                {"key": "sync", "min": 0.0625, "max": 4.0, "default": 0.75, "help": "Delay time in beats (0.75 = dotted 8th); overrides time"},
                {"key": "time", "min": 1.0, "max": 4000.0, "default": 375.0, "help": "Delay time in ms when sync is unset"},
                {"key": "feedback", "min": 0.0, "max": 0.95, "default": 0.35},
                {"key": "damp", "min": 0.0, "max": 1.0, "default": 0.4},
                {"key": "ping_pong", "min": 0.0, "max": 1.0, "default": 0.5},
            ],
        },
        "track_automation": ["track.level", "track.pan", "send.reverb", "send.delay", "fx.<j>.<key>", "voice.<i>.<param>", "voice.<i>.fx.<j>.<key>", "master.master_gain", "master.master_width"],
        "limits": {
            "max_voices": voices::MAX_VOICES,
            "max_effects_per_voice": 8,
            "max_midi_fx_per_voice": 8,
            "max_tracks": 32,
            "max_clips": 256,
            "max_effects_per_track": 8,
            "max_master_effects": 8,
            "max_length_beats": crate::engine::MAX_BEATS,
            "steps": [4, 8, 16, 32, 64],
            "sample_rate": crate::engine::SAMPLE_RATE as u32,
        },
    })
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

    #[test]
    fn factory_presets_only_set_parameters_their_kind_has() {
        let c = catalog_json();
        let presets = c["factory_presets"].as_object().expect("factory_presets is a map");
        assert!(!presets.is_empty());
        let mut wrong = Vec::new();
        for (kind, list) in presets {
            let kind_entry = c["kinds"].as_array().unwrap().iter().find(|k| k["kind"] == kind.as_str());
            let kind_entry = kind_entry.unwrap_or_else(|| panic!("preset kind {kind} is not a voice kind"));
            let params = kind_entry["params"].as_array().unwrap();
            let keys: Vec<&str> = params.iter().filter_map(|p| p["key"].as_str()).collect();
            for preset in list.as_array().unwrap() {
                let name = preset["name"].as_str().unwrap_or("?");
                if let Some(synth) = preset["params"]["synth"].as_object() {
                    for key in synth.keys() {
                        let Some(def) = params.iter().find(|p| p["key"] == key.as_str()) else {
                            wrong.push(format!("{kind}/{name}: `{key}` is not a {kind} parameter"));
                            continue;
                        };
                        let v = synth[key].as_f64().unwrap_or(f64::NAN);
                        let (lo, hi) = (def["min"].as_f64().unwrap(), def["max"].as_f64().unwrap());
                        if !(lo..=hi).contains(&v) {
                            wrong.push(format!("{kind}/{name}: `{key}` = {v} is outside {lo}..{hi}"));
                        }
                    }
                }
            }
        }
        assert!(wrong.is_empty(), "presets set parameters their kind lacks:\n{}", wrong.join("\n"));
    }
}
