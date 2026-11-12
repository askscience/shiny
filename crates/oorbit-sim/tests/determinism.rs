//! The core promise of the simulation: catching up in one go gives exactly
//! the same planet as catching up in pieces, and long absences never destroy.
mod common;

use common::{play_session, DAY, HOUR, T0};
use oorbit_sim::rng::Rng;
use oorbit_sim::{advance, Catalog, Planet};

fn json(planet: &Planet) -> String {
    serde_json::to_string(planet).unwrap()
}

#[test]
fn one_long_advance_equals_many_short_ones() {
    let catalog = Catalog::builtin();
    for seed in [1u64, 7, 42, 2026] {
        let mut whole = Planet::new(seed, "temperate", T0, &catalog).unwrap();
        let mut pieces = whole.clone();
        let end = T0 + 10 * DAY + 17 * 60_000;
        advance(&mut whole, &catalog, end);

        let mut rng = Rng::new(seed);
        let mut now = T0;
        while now < end {
            now = (now + 1 + (rng.next_u64() % (9 * HOUR as u64)) as i64).min(end);
            advance(&mut pieces, &catalog, now);
        }
        assert_eq!(json(&whole), json(&pieces), "seed {seed}");
    }
}

#[test]
fn played_sessions_replay_identically() {
    let catalog = Catalog::builtin();
    let visits: Vec<i64> = (0..40)
        .map(|i| T0 + i * 11 * HOUR + (i % 3) * 7 * 60_000)
        .collect();
    let run = |split: bool| {
        let mut planet = Planet::new(99, "oceanic", T0, &catalog).unwrap();
        for &visit in &visits {
            if split {
                // The machine was on: the planet had already advanced to just
                // before the visit, in small steps.
                let mut t = planet.step * oorbit_sim::STEP_MS;
                while t + HOUR < visit {
                    t += HOUR;
                    advance(&mut planet, &catalog, t);
                }
            }
            play_session(&mut planet, &catalog, visit);
        }
        json(&planet)
    };
    assert_eq!(run(false), run(true));
}

#[test]
fn going_backwards_in_time_changes_nothing() {
    let catalog = Catalog::builtin();
    let mut planet = Planet::new(5, "arid", T0, &catalog).unwrap();
    advance(&mut planet, &catalog, T0 + DAY);
    let before = json(&planet);
    advance(&mut planet, &catalog, T0);
    assert_eq!(before, json(&planet));
}

#[test]
fn a_long_absence_never_destroys() {
    let catalog = Catalog::builtin();
    for climate in ["temperate", "arid", "frozen", "oceanic"] {
        let mut planet = Planet::new(3, climate, T0, &catalog).unwrap();
        for i in 0..12 {
            play_session(&mut planet, &catalog, T0 + i * 12 * HOUR);
        }
        let built: Vec<Option<String>> = planet.tiles.iter().map(|t| t.building.clone()).collect();
        let known = planet.known.clone();
        let era = planet.era;
        // Sixty days away, nobody answering anything.
        advance(&mut planet, &catalog, T0 + 66 * DAY);
        // Nothing that stood is gone (construction may have finished meanwhile).
        for (tile, before) in planet.tiles.iter().zip(&built) {
            if before.is_some() {
                assert_eq!(&tile.building, before, "{climate}");
            }
        }
        assert!(known.is_subset(&planet.known), "{climate}");
        assert!(planet.era >= era, "{climate}");
        assert!(
            planet.population >= catalog.tuning.min_population,
            "{climate}"
        );
        assert!(
            planet
                .resources
                .values()
                .all(|v| v.is_finite() && *v >= 0.0),
            "{climate}"
        );
        assert!((0.0..=1.0).contains(&planet.mood), "{climate}");
        assert!(
            planet.pending_event.is_none()
                || planet.chronicle.iter().any(|e| e.key == "event_auto")
        );
    }
}

#[test]
fn every_climate_gives_a_playable_planet() {
    let catalog = Catalog::builtin();
    for climate in &catalog.climates {
        for seed in 0..50u64 {
            let planet = Planet::new(seed, &climate.id, T0, &catalog).unwrap();
            for biome in [
                oorbit_sim::catalog::Biome::Plains,
                oorbit_sim::catalog::Biome::Forest,
                oorbit_sim::catalog::Biome::Mountain,
            ] {
                assert!(
                    planet.tiles.iter().filter(|t| t.biome == biome).count() >= 4,
                    "{} {seed}",
                    climate.id
                );
            }
        }
    }
}

#[test]
fn the_state_survives_a_json_round_trip() {
    let catalog = Catalog::builtin();
    let mut planet = Planet::new(8, "frozen", T0, &catalog).unwrap();
    for i in 0..10 {
        play_session(&mut planet, &catalog, T0 + i * 9 * HOUR);
    }
    let text = json(&planet);
    let back: Planet = serde_json::from_str(&text).unwrap();
    assert_eq!(back, planet);
}
