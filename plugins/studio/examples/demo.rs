//! Render a short demo arrangement with the Studio engine and write a WAV.
//!
//! ```sh
//! cargo run -p shiny-studio-plugin --example demo -- /tmp/studio-demo.wav
//! ```
//!
//! This is the fastest way to *hear* the engine end to end: drums, bass, a
//! supersaw lead and a pad, arranged over eight bars with master glue,
//! reverb and loudness normalisation — all through the same
//! `parse_arrangement` / `render_arrangement` path the plugin's REST routes
//! and agent tools use.

use serde_json::json;

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "studio-demo.wav".to_string());

    // ── drums: one pattern per kit piece, voiced from the drum models ──
    let kit = json!({
        "title": "Kit",
        "bpm": 124,
        "steps": 16,
        "swing": 0.18,
        "tuning": "edo12",
        "voices": [
            { "kind": "kick", "rhythm": "e4,0", "accent": 0.25,
              "synth": { "pitch": 48, "decay": 9, "sweep": 34, "click": 0.3, "drive": 0.35 },
              "notes": [ {"step": 14, "length": 1, "degree": 0, "octave": 0, "velocity": 0.6} ] },
            { "kind": "snare", "rhythm": "e4,8", "accent": 0.2,
              "synth": { "tone": 190, "body": 22, "noise": 18, "snap": 0.6, "drive": 0.25 } },
            { "kind": "clap", "rhythm": "e2,2", "level": 0.35,
              "synth": { "tone": 1500, "body": 9, "noise": 30, "spread": 0.6 } },
            { "kind": "hat", "rhythm": "e8,2", "accent": 0.3, "level": 0.32,
              "synth": { "decay": 26, "tone": 9500, "metal": 0.85, "drive": 0.12 } },
            { "kind": "shaker", "rhythm": "e16,5", "level": 0.2, "accent": 0.18,
              "synth": { "decay": 55, "tone": 7000 } }
        ],
        "fx": { "delay_mix": 0.0, "reverb_mix": 0.06, "reverb_size": 0.4 }
    });

    // ── bass: mono, glided, 24 dB filter with envelope ──
    let bass = json!({
        "title": "Bass",
        "bpm": 124,
        "steps": 16,
        "tuning": "edo12",
        "voices": [
            { "kind": "bass", "rhythm": "", "octave": 2,
              "notes": [
                {"step": 0, "length": 3, "degree": 0, "octave": 2},
                {"step": 4, "length": 2, "degree": 0, "octave": 2},
                {"step": 6, "length": 2, "degree": 3, "octave": 2},
                {"step": 8, "length": 3, "degree": 5, "octave": 2},
                {"step": 12, "length": 4, "degree": 4, "octave": 2}
              ],
              "synth": { "o1w": 2, "o2w": 3, "b_level": 0.3, "sub": 0.45,
                         "cutoff": 620, "res": 1.5, "poles": 2, "fenv": 1.8,
                         "fdecay": 0.14, "fsustain": 0.1, "drive": 1.8,
                         "glide": 0.03, "attack": 0.004, "release": 0.1 } }
        ],
        "fx": { "reverb_mix": 0.03 }
    });

    // ── lead: 3-voice supersaw with a touch of LFO on the filter ──
    let lead = json!({
        "title": "Lead",
        "bpm": 124,
        "steps": 16,
        "tuning": "edo12",
        "voices": [
            { "kind": "lead", "rhythm": "", "octave": 4,
              "notes": [
                {"step": 0, "length": 2, "degree": 4, "octave": 4},
                {"step": 2, "length": 2, "degree": 7, "octave": 4},
                {"step": 4, "length": 4, "degree": 9, "octave": 4},
                {"step": 8, "length": 2, "degree": 7, "octave": 4},
                {"step": 10, "length": 2, "degree": 4, "octave": 4},
                {"step": 12, "length": 4, "degree": 2, "octave": 4}
              ],
              "synth": { "unison": 3, "spread": 0.6, "cutoff": 2600, "res": 1.8, "fenv": 1.6,
                         "lfo_rate": 4.5, "lfo_pitch": 0.05, "attack": 0.006,
                         "decay": 0.22, "sustain": 0.55, "release": 0.3, "pan_spread": 0.3 },
              "fx": [ { "kind": "delay", "params": { "time": 362, "feedback": 0.35, "mix": 0.28, "ping_pong": 0.6, "damp": 0.45 } } ] }
        ],
        "fx": { "reverb_mix": 0.14, "reverb_size": 0.6, "reverb_damp": 0.5 }
    });

    // ── pad: five-voice unison chords, long attack ──
    let pad = json!({
        "title": "Pad",
        "bpm": 124,
        "steps": 16,
        "tuning": "edo12",
        "voices": [
            { "kind": "pad", "rhythm": "", "octave": 3, "level": 0.32,
              "notes": [
                {"step": 0, "length": 8, "degree": 0, "octave": 3},
                {"step": 0, "length": 8, "degree": 2, "octave": 3},
                {"step": 0, "length": 8, "degree": 4, "octave": 3},
                {"step": 8, "length": 8, "degree": 5, "octave": 3},
                {"step": 8, "length": 8, "degree": 7, "octave": 3},
                {"step": 8, "length": 8, "degree": 9, "octave": 3}
              ],
              "synth": { "unison": 5, "spread": 0.85, "cutoff": 1300, "res": 0.8,
                         "attack": 0.5, "decay": 0.6, "sustain": 0.8, "release": 1.1,
                         "lfo_rate": 0.22, "lfo_depth": 0.3, "pan_spread": 0.5 },
              "fx": [ { "kind": "chorus", "params": { "rate": 0.4, "depth": 0.6, "mix": 0.35, "spread": 0.7 } } ] }
        ],
        "fx": { "reverb_mix": 0.1 }
    });

    let arrangement = json!({
        "title": "Studio demo",
        "bpm": 124,
        "length_beats": 32,
        "master": 0.9,
        "fx": { "glue": 0.35, "ceiling": -0.8, "loudness": -14.0, "master_width": 1.1 },
        "tracks": [
            { "id": "drums", "name": "Drums", "color": 0, "level": 0.9, "pan": 0.0 },
            { "id": "bass",  "name": "Bass",  "color": 3, "level": 0.85, "pan": 0.0 },
            { "id": "lead",  "name": "Lead",  "color": 5, "level": 0.55, "pan": 0.15 },
            { "id": "pad",   "name": "Pad",   "color": 6, "level": 0.6, "pan": -0.1,
              "automation": { "lanes": [
                { "param": "track.level", "points": [
                    { "beat": 0, "value": 0.0 }, { "beat": 8, "value": 0.6 },
                    { "beat": 16, "value": 0.6 }, { "beat": 24, "value": 0.75 } ] } ] } }
        ],
        "clips": [
            { "track": "drums", "start": 0,  "pattern": kit },
            { "track": "drums", "start": 16, "pattern": kit },
            { "track": "bass",  "start": 0,  "pattern": bass },
            { "track": "bass",  "start": 16, "pattern": bass },
            { "track": "lead",  "start": 8,  "pattern": lead },
            { "track": "lead",  "start": 24, "pattern": lead },
            { "track": "pad",   "start": 0,  "pattern": pad },
            { "track": "pad",   "start": 16, "pattern": pad }
        ]
    });

    let started = std::time::Instant::now();
    let arr = match shiny_studio_plugin::engine::parse_arrangement(&arrangement) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("invalid arrangement: {e}");
            std::process::exit(1);
        }
    };
    let rendered = match shiny_studio_plugin::engine::render_arrangement(&arr) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("render failed: {e}");
            std::process::exit(1);
        }
    };
    if let Err(e) = std::fs::write(&out, &rendered.wav) {
        eprintln!("could not write {out}: {e}");
        std::process::exit(1);
    }
    let secs = rendered.duration_ms as f64 / 1000.0;
    println!("wrote {out}");
    println!(
        "  {:.2}s of audio in {:.2}s ({:.0}x realtime)",
        secs,
        started.elapsed().as_secs_f64(),
        secs / started.elapsed().as_secs_f64().max(1e-6)
    );
    println!("  {:.1} LUFS · peak {:.3} · {} Hz · {} ch", rendered.lufs, rendered.peak, rendered.sample_rate, rendered.channels);
}
