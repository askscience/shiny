//! Print a case as the generator writes it.
//!
//!     cargo run --example case -- [city name] [case number] [level]

use happywalk_game::case::{self, features, Clue};
use happywalk_game::city::seed_for;

fn main() {
    let mut args = std::env::args().skip(1);
    let city = args.next().unwrap_or_else(|| "Borgo Pastello".into());
    let number: u32 = args.next().and_then(|a| a.parse().ok()).unwrap_or(0);
    let level: u32 = args.next().and_then(|a| a.parse().ok()).unwrap_or(1);
    let case = case::case(seed_for(&city), number, level);
    println!("{city}, case {number}, level {level}");
    println!(
        "What happened: {} (at {}).",
        case.crime.what,
        case.crime.at.name()
    );
    println!("Reported by {}, {}.", case.client.0, case.client.1);
    println!("At the scene: {}.", case.crime.trace.at_scene());
    println!("\nSuspects:");
    for (i, s) in case.suspects.iter().enumerate() {
        let looks: Vec<String> = features(&s.look).iter().map(|f| f.words()).collect();
        let traces: Vec<&str> = s.traces.iter().map(|t| t.on_someone()).collect();
        println!(
            "  {i}. {}, {} ({}), usually at {}",
            s.name,
            s.role,
            s.manner,
            s.hangs_out.name()
        );
        println!("     looks: {}", looks.join(", "));
        println!(
            "     carries: {}",
            if traces.is_empty() {
                "nothing".into()
            } else {
                traces.join(", ")
            }
        );
        println!("     secret: {}", s.secret);
        if let Some(a) = &s.alibi {
            println!(
                "     says they were at {} with {} ({})",
                a.place.name(),
                case.suspects[a.with].name,
                if a.true_ { "true" } else { "a lie" }
            );
        }
    }
    println!("\nClues:");
    for clue in &case.clues {
        match clue {
            Clue::Trace(t) => println!("  - at the scene: {}", t.at_scene()),
            Clue::Sighting { feature, by } => println!(
                "  - {} saw that the culprit {}",
                by.map(|b| case.suspects[b].name).unwrap_or(case.client.0),
                feature.words()
            ),
            Clue::AlibiCheck { of, by } => println!(
                "  - ask {} about {}'s alibi",
                case.suspects[*by].name, case.suspects[*of].name
            ),
        }
    }
    let culprit = &case.suspects[case.culprit];
    println!(
        "\nIt was {} the {}, who {}. Fair: {}",
        culprit.name,
        culprit.role,
        case.motive,
        case.is_fair()
    );
}
