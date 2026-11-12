//! Not a check: prints a trace of a steady player, for balancing.
//! `SEED=1 CLIMATE=temperate FROM=40 cargo test -p oorbit-sim --test trace -- --ignored --nocapture`
mod common;
use common::{play_session, DAY, HOUR, T0};
use oorbit_sim::{rates, Catalog, Planet};

#[test]
#[ignore]
fn trace() {
    let env = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
    let catalog = Catalog::builtin();
    let mut planet = Planet::new(
        env("SEED", "1").parse().unwrap(),
        &env("CLIMATE", "temperate"),
        T0,
        &catalog,
    )
    .unwrap();
    let from: i64 = env("FROM", "0").parse().unwrap();
    let to: i64 = env("TO", "60").parse().unwrap();
    let mut now = T0;
    while now < T0 + to * DAY {
        let hour = (now / HOUR) % 24;
        if (7..23).contains(&hour) {
            play_session(&mut planet, &catalog, now);
        }
        let day = (now - T0) / DAY;
        if day >= from
            && (now - T0) % (2 * DAY) < 24 * HOUR / env("VISITS", "2").parse::<i64>().unwrap()
        {
            let r = rates(&planet, &catalog);
            let era = catalog.era(planet.era).unwrap();
            let monument = catalog.building(&era.monument).unwrap();
            let need: Vec<String> = monument
                .cost
                .iter()
                .map(|(k, v)| {
                    format!(
                        "{k} {:.0}/{v:.0} ({:+.1}/h)",
                        planet.amount(k),
                        r.per_hour.get(k).unwrap_or(&0.0)
                    )
                })
                .collect();
            let missing: Vec<&String> = era
                .requires
                .iter()
                .filter(|t| !planet.known.contains(*t))
                .collect();
            println!(
                "day {day:2} era {} pop {:.0}/{:.0} storage {:.0} research {:?} missing {:?} free {} building {:?}\n   monument {}: {}",
                planet.era, planet.population, r.housing, r.storage, planet.research, missing,
                oorbit_sim::step::free_tiles(&planet, &[]).len(),
                planet.construction.iter().map(|c| c.building.as_str()).collect::<Vec<_>>(),
                monument.id, need.join(", ")
            );
            if let Ok(probe) = std::env::var("PROBE") {
                let mountains: Vec<String> = planet
                    .tiles
                    .iter()
                    .filter(|t| t.biome == oorbit_sim::catalog::Biome::Mountain)
                    .map(|t| t.building.clone().unwrap_or("-".into()))
                    .collect();
                println!(
                    "   probe {probe}: {:?}; wood {:.0} stone {:.0} iron {:.0}; mountains {:?}",
                    oorbit_sim::actions::can_build(&planet, &catalog, &probe),
                    planet.amount("wood"),
                    planet.amount("stone"),
                    planet.amount("iron"),
                    mountains
                );
            }
        }
        now += 24 * HOUR / env("VISITS", "2").parse::<i64>().unwrap();
    }
}
