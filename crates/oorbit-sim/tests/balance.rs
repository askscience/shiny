//! Pacing: a steady player (two visits a day) must reach each era within the
//! design's window. Run with `--nocapture` to see the timeline.
mod common;

use common::{play_session, DAY, HOUR, T0};
use oorbit_sim::{Catalog, Planet};

/// Days to reach era 2, era 3, and to finish era 3, for one planet.
fn timeline(seed: u64, climate: &str, visits_per_day: i64) -> [Option<f64>; 3] {
    let catalog = Catalog::builtin();
    let mut planet = Planet::new(seed, climate, T0, &catalog).unwrap();
    let mut reached = [None; 3];
    let gap = 24 * HOUR / visits_per_day;
    let mut now = T0;
    while now < T0 + 60 * DAY {
        // Visits in waking hours only: skip 23:00 to 07:00 (UTC as local).
        let hour = (now / HOUR) % 24;
        if (7..23).contains(&hour) {
            play_session(&mut planet, &catalog, now);
        }
        let days = (now - T0) as f64 / DAY as f64;
        if planet.era >= 2 && reached[0].is_none() {
            reached[0] = Some(days);
        }
        if planet.era >= 3 && reached[1].is_none() {
            reached[1] = Some(days);
        }
        if planet.eras_done && reached[2].is_none() {
            reached[2] = Some(days);
            break;
        }
        now += gap;
    }
    reached
}

#[test]
fn a_steady_player_paces_through_the_first_three_eras() {
    let mut failures = Vec::new();
    for climate in ["temperate", "arid", "frozen", "oceanic"] {
        for seed in [1u64, 2, 3, 4, 5] {
            let t = timeline(seed, climate, 2);
            println!(
                "{climate:9} seed {seed}: era2 {:?} era3 {:?} done {:?}",
                t[0], t[1], t[2]
            );
            let [e2, e3, done] = t.map(|d| d.unwrap_or(f64::INFINITY));
            for (what, day, range) in [
                ("era 2", e2, 2.0..=8.0),
                ("era 3", e3, 6.0..=16.0),
                ("era 3 done", done, 12.0..=27.0),
            ] {
                if !range.contains(&day) {
                    failures.push(format!(
                        "{climate} seed {seed}: {what} on day {day:.1}, expected {range:?}"
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn an_occasional_player_still_progresses() {
    // One visit a day: slower, never stuck.
    let t = timeline(4, "temperate", 1);
    println!("one visit a day: {t:?}");
    assert!(t[1].is_some(), "era 3 is reached with one visit a day");
}
