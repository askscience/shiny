//! Pacing: a steady player (two visits a day, waking hours only) must move
//! through the eras within the design's windows. Run with `--nocapture` to
//! see the timeline.
mod common;

use common::{play_session, DAY, HOUR, T0};
use oorbit_sim::{Catalog, Planet};

/// Day each era was reached (index 0 = era 2), then the day the last era
/// was completed; `None` when not reached within `max_days`.
fn timeline(
    seed: u64,
    climate: &str,
    visits_per_day: i64,
    max_days: i64,
) -> (Vec<Option<f64>>, Option<f64>) {
    let catalog = Catalog::builtin();
    let mut planet = Planet::new(seed, climate, T0, &catalog).unwrap();
    let mut reached = vec![None; catalog.eras.len() - 1];
    let mut done = None;
    let gap = 24 * HOUR / visits_per_day;
    let mut now = T0;
    while now < T0 + max_days * DAY {
        let hour = (now / HOUR) % 24;
        if (7..23).contains(&hour) {
            play_session(&mut planet, &catalog, now);
        }
        let days = (now - T0) as f64 / DAY as f64;
        for era in 2..=planet.era {
            let slot = &mut reached[era as usize - 2];
            if slot.is_none() {
                *slot = Some(days);
            }
        }
        if planet.eras_done {
            done = Some(days);
            break;
        }
        now += gap;
    }
    (reached, done)
}

fn show(t: &[Option<f64>]) -> String {
    t.iter()
        .map(|d| d.map_or("-".into(), |d| format!("{d:.1}")))
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn a_steady_player_paces_through_every_era() {
    let mut failures = Vec::new();
    for climate in ["temperate", "arid", "frozen", "oceanic"] {
        for seed in [1u64, 2, 3] {
            let (reached, done) = timeline(seed, climate, 2, 90);
            println!(
                "{climate:9} seed {seed}: eras 2..10 on days {} | done {:?}",
                show(&reached),
                done
            );
            let day = |era: usize| reached[era - 2].unwrap_or(f64::INFINITY);
            for (what, value, range) in [
                ("era 2", day(2), 1.5..=5.0),
                ("era 3", day(3), 5.0..=12.0),
                ("era 4", day(4), 10.0..=22.0),
                ("era 7", day(7), 30.0..=52.0),
                ("era 10", day(10), 45.0..=72.0),
                (
                    "the last era done",
                    done.unwrap_or(f64::INFINITY),
                    50.0..=85.0,
                ),
            ] {
                if !range.contains(&value) {
                    failures.push(format!(
                        "{climate} seed {seed}: {what} on day {value:.1}, expected {range:?}"
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
    let (reached, _) = timeline(4, "temperate", 1, 90);
    println!("one visit a day: eras 2..10 on days {}", show(&reached));
    assert!(
        reached[5].is_some(),
        "era 7 is reached with one visit a day"
    );
}
