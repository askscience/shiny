//! Time single-voice renders per kind (development aid).
use serde_json::json;
fn main() {
    let only = std::env::args().nth(1);
    for (kind, synth) in [
        ("kick", json!({})), ("hat", json!({})), ("bass", json!({})),
        ("lead", json!({})), ("pad", json!({})), ("pad", json!({"unison": 5})),
    ] {
        if only.as_deref().is_some_and(|o| o != kind) { continue; }
        let cfg = shiny_studio_plugin::engine::parse_config(&json!({"bpm":120,"steps":64,
            "voices":[{"kind":kind,"rhythm":"x...x...x...x...x...x...x...x...x...x...x...x...x...x...x...x...","octave":-1,"synth":synth}]})).unwrap();
        let t = std::time::Instant::now();
        let p = shiny_studio_plugin::engine::render_planar_track(&cfg).unwrap();
        println!("{kind:6} {synth}: {:.2}s audio in {:.3}s", p.frames as f64 / 44100.0, t.elapsed().as_secs_f64());
    }
}
