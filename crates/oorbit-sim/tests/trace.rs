//! Not a check: prints a day-by-day trace of a steady player, for balancing.
//! `cargo test -p oorbit-sim --test trace -- --ignored --nocapture`
mod common;
use common::{play_session, DAY, HOUR, T0};
use oorbit_sim::{rates, Catalog, Planet};

#[test]
#[ignore]
fn trace() {
    let catalog = Catalog::builtin();
    let mut planet = Planet::new(
        std::env::var("SEED")
            .map(|s| s.parse().unwrap())
            .unwrap_or(1),
        &std::env::var("CLIMATE").unwrap_or("temperate".into()),
        T0,
        &catalog,
    )
    .unwrap();
    let mut now = T0;
    while now < T0 + 24 * DAY {
        let hour = (now / HOUR) % 24;
        if (7..23).contains(&hour) {
            play_session(&mut planet, &catalog, now);
        }
        if std::env::var("TRACE_ALL").is_ok() || (now - T0) % DAY == 0 {
            let r = rates(&planet, &catalog);
            let res: Vec<String> = planet
                .resources
                .iter()
                .map(|(k, v)| format!("{k}={v:.0}"))
                .collect();
            let cons: Vec<String> = planet
                .construction
                .iter()
                .map(|c| c.building.clone())
                .collect();
            let free = oorbit_sim::step::free_tiles(&planet, &[]).len();
            println!(
                "   building {:?} free {} storage {:.0} built {:?}",
                cons,
                free,
                r.storage,
                planet.built().collect::<Vec<_>>()
            );
            println!(
                "day {:2} era {} pop {:.1}/{:.0} mood {:.2} research {:?} known {} built {} | {} | food/h {:.2} know/h {:.2}",
                (now - T0) / DAY, planet.era, planet.population, r.housing, planet.mood, planet.research,
                planet.known.len(), planet.built().count(), res.join(" "),
                r.per_hour.get("food").unwrap_or(&0.0), r.per_hour.get("knowledge").unwrap_or(&0.0)
            );
        }
        now += 12 * HOUR;
    }
}
