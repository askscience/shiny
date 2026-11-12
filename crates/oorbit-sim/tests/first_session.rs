//! The first session must show the game moving: a curious player who looks
//! in every few minutes sees discoveries and buildings finish within the
//! first hour, not the next day.
mod common;

use common::{play_session, HOUR, T0};
use oorbit_sim::{Catalog, Planet};

const MINUTE: i64 = 60_000;

/// Minutes from founding to each chronicle entry with `key`.
fn first_minutes(planet: &Planet, key: &str) -> Vec<i64> {
    planet
        .chronicle
        .iter()
        .filter(|e| e.key == key)
        .map(|e| (e.step - planet.created_step) * 10)
        .collect()
}

#[test]
fn the_first_hour_already_shows_progress() {
    let catalog = Catalog::builtin();
    for climate in ["temperate", "arid", "frozen", "oceanic"] {
        let mut planet = Planet::new(3, climate, T0, &catalog).unwrap();
        let mut now = T0;
        while now <= T0 + 2 * HOUR {
            play_session(&mut planet, &catalog, now);
            now += 5 * MINUTE;
        }
        let techs = first_minutes(&planet, "tech_done");
        let buildings = first_minutes(&planet, "building_done");
        println!("{climate:9} discoveries at {techs:?} min, buildings at {buildings:?} min");
        assert!(
            techs.first().is_some_and(|&m| m <= 20),
            "{climate}: first discovery within 20 minutes"
        );
        assert!(
            techs.iter().filter(|&&m| m <= 60).count() >= 2,
            "{climate}: two discoveries in the first hour"
        );
        assert!(
            buildings.first().is_some_and(|&m| m <= 90),
            "{climate}: a first building within 90 minutes"
        );
    }
}
