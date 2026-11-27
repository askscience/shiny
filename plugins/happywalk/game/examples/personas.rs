//! Print, as JSON, who the language model plays in a case: the client and
//! every suspect, as the game writes them (for trying a model by hand).
//!
//!     cargo run --example personas -- [city name] [case number] [level]

use happywalk_game::case;
use happywalk_game::city::seed_for;
use happywalk_game::mystery::{client_persona, suspect_persona};

fn main() {
    let mut args = std::env::args().skip(1);
    let city = args.next().unwrap_or_else(|| "Borgo Pastello".into());
    let number: u32 = args.next().and_then(|a| a.parse().ok()).unwrap_or(0);
    let level: u32 = args.next().and_then(|a| a.parse().ok()).unwrap_or(1);
    let case = case::case(seed_for(&city), number, level);
    let suspects: Vec<serde_json::Value> = (0..case.suspects.len())
        .map(|s| {
            serde_json::json!({
                "name": case.suspects[s].name,
                "culprit": s == case.culprit,
                "persona": suspect_persona(&case, s, &city, true),
            })
        })
        .collect();
    let out = serde_json::json!({
        "what": case.crime.what,
        "client": { "name": case.client.0, "persona": client_persona(&case, &city, "Italian") },
        "suspects": suspects,
    });
    println!("{}", serde_json::to_string_pretty(&out).unwrap());
}
