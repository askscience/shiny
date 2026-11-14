//! The first session played actively: while someone plays, an hour of the
//! planet lasts a real minute. A player who acts every real minute reaches
//! the Agricultural Age within a twenty-minute session, and the planet never
//! slows down for idleness while they watch.
mod common;

use common::{play_session, HOUR, T0};
use oorbit_sim::{Catalog, Planet};

/// Planet hours (= real minutes of play) until the first `key` entry.
fn hours_to(planet: &Planet, key: &str) -> Option<f64> {
    planet
        .chronicle
        .iter()
        .find(|e| e.key == key)
        .map(|e| (e.step - planet.created_step) as f64 / 6.0)
}

#[test]
fn an_active_first_session_reaches_the_second_era() {
    let catalog = Catalog::builtin();
    let mut worst: f64 = 0.0;
    for climate in ["temperate", "arid", "frozen", "oceanic"] {
        for seed in [1u64, 2, 3] {
            let mut planet = Planet::new(seed, climate, T0, &catalog).unwrap();
            let mut now = T0;
            while now <= T0 + 40 * HOUR && planet.era < 2 {
                // Watching counts as acting.
                planet.last_action_step = oorbit_sim::step_of(now);
                play_session(&mut planet, &catalog, now);
                now += HOUR;
            }
            let era2 = hours_to(&planet, "era_reached").unwrap_or(f64::INFINITY);
            let first = hours_to(&planet, "tech_done").unwrap_or(f64::INFINITY);
            if std::env::var("DEBUG").is_ok() {
                let r = oorbit_sim::rates(&planet, &catalog);
                println!("  res {:?}", planet.resources.iter().filter(|(_, v)| **v > 0.5).map(|(k, v)| format!("{k}={v:.0}({:+.1})", r.per_hour.get(k).unwrap_or(&0.0))).collect::<Vec<_>>());
                println!("  known {:?} building {:?} orders {:?} pop {:.1} eff {:.2}", planet.known, planet.construction.iter().map(|c| &c.building).collect::<Vec<_>>(), planet.orders.iter().map(|o| &o.resource).collect::<Vec<_>>(), planet.population, r.efficiency);
                println!("  built {:?}", planet.built().collect::<Vec<_>>());
            }
            println!("{climate:9} seed {seed}: first discovery at {first:.1} min, era 2 at {era2:.1} min");
            assert!(first <= 3.0, "{climate} {seed}: a discovery in the first minutes");
            worst = worst.max(era2);
        }
    }
    assert!(worst <= 22.0, "era 2 within a twenty-minute session (worst {worst:.1})");
}
