//! A steady player for the tests: checks in at fixed times and plays sensibly.
#![allow(dead_code)]

use oorbit_sim::actions::{can_build, can_research};
use oorbit_sim::step::free_tiles;
use oorbit_sim::{apply, Action, Catalog, Planet, Priorities};

pub const HOUR: i64 = 3_600_000;
pub const DAY: i64 = 24 * HOUR;
/// 2026-11-12T08:00:00Z
pub const T0: i64 = 1_794_470_400_000;

/// One visit: answer the event, keep the people fed and housed, research
/// and build what helps reach the next era.
pub fn play_session(planet: &mut Planet, catalog: &Catalog, now: i64) {
    oorbit_sim::advance(planet, catalog, now);
    if let Some(pending) = planet.pending_event.clone() {
        let choice = catalog.event(&pending.id).unwrap().choices[0].id.clone();
        apply(planet, catalog, now, &Action::ResolveEvent { choice }).unwrap();
    }
    let rates = oorbit_sim::rates(planet, catalog);
    let food_rate = rates.per_hour.get("food").copied().unwrap_or(0.0);
    let priorities = if food_rate < 0.2 || planet.amount("food") < 20.0 {
        Priorities {
            food: 4,
            materials: 2,
            knowledge: 2,
            culture: 1,
        }
    } else {
        Priorities {
            food: 2,
            materials: 3,
            knowledge: 3,
            culture: 1,
        }
    };
    apply(planet, catalog, now, &Action::SetPriorities { priorities }).unwrap();

    if planet.research.is_none() {
        let era = catalog.era(planet.era).unwrap();
        let mut options: Vec<_> = catalog
            .techs
            .iter()
            .filter(|t| can_research(planet, catalog, &t.id).is_ok())
            .collect();
        // Era requirements first, then the cheapest.
        options.sort_by(|a, b| {
            let ra = !era.requires.contains(&a.id);
            let rb = !era.requires.contains(&b.id);
            (ra, a.cost as i64).cmp(&(rb, b.cost as i64))
        });
        if let Some(t) = options.first() {
            let tech = t.id.clone();
            apply(planet, catalog, now, &Action::Research { tech }).unwrap();
        }
    }

    // Build: housing when crowded, the monument when ready, then one useful
    // building per visit.
    let mut wanted: Vec<String> = Vec::new();
    if planet.population > rates.housing - 2.0 {
        wanted.extend(["house", "hut"].map(String::from));
    }
    let era = catalog.era(planet.era).unwrap();
    let saving = era.requires.iter().all(|t| planet.known.contains(t))
        && !planet.has_built(&era.monument)
        && !planet
            .construction
            .iter()
            .any(|c| c.building == era.monument);
    if saving {
        // Save everything for the monument (housing only when crowded).
        wanted.insert(0, era.monument.clone());
        wanted.truncate(if planet.population > rates.housing - 1.0 {
            3
        } else {
            1
        });
        // ...but build what yields a monument input nobody produces yet.
        let monument = catalog.building(&era.monument).unwrap();
        for input in monument.cost.keys() {
            if rates.per_hour.get(input).copied().unwrap_or(0.0) > 0.0 {
                continue;
            }
            for b in &catalog.buildings {
                let converts = b.effects.iter().any(|e| matches!(e, oorbit_sim::catalog::Effect::Convert { output, .. } if output == input));
                let feeds_converter = catalog.buildings.iter().any(|c| c.effects.iter().any(|e| matches!(e, oorbit_sim::catalog::Effect::Convert { output, inputs, .. } if output == input && inputs.keys().any(|i| catalog.resource(i).and_then(|r| r.from_building.as_deref()) == Some(b.id.as_str())))));
                if (converts || feeds_converter) && !planet.has_built(&b.id) {
                    wanted.insert(0, b.id.clone());
                }
            }
        }
    }
    let count = |id: &str| {
        planet.built().filter(|b| *b == id).count()
            + planet
                .construction
                .iter()
                .filter(|c| c.building == id)
                .count()
    };
    if !saving {
        wanted.extend(
            [
                "mine",
                "foundry",
                "field",
                "storehouse",
                "hunting_camp",
                "fire_pit",
                "pasture",
                "scribe_house",
                "temple",
            ]
            .map(String::from),
        );
    }
    // Keep a few tiles for the monument until it stands.
    let monument = catalog.building(&era.monument).unwrap();
    let monument_room = free_tiles(planet, &monument.biomes).len();
    let monument_pending = !planet.has_built(&era.monument)
        && !planet
            .construction
            .iter()
            .any(|c| c.building == era.monument);
    for id in wanted {
        let def = catalog.building(&id).unwrap();
        if !def.monument
            && monument_pending
            && monument_room <= 2
            && def
                .biomes
                .iter()
                .any(|b| monument.biomes.contains(b) || def.biomes.is_empty())
        {
            continue;
        }
        let limit = match id.as_str() {
            "field" => 6,
            "mine" => 3,
            "house" | "hut" => 99,
            _ => 1,
        };
        if (def.monument || count(&id) < limit) && can_build(planet, catalog, &id).is_ok() {
            // Buildings that fit anywhere go on the least useful land.
            let mut tiles = free_tiles(planet, &def.biomes);
            use oorbit_sim::catalog::Biome::*;
            let rank = |b| {
                [Ice, Desert, Coast, Forest, Plains, Mountain]
                    .iter()
                    .position(|x| *x == b)
                    .unwrap()
            };
            if def.biomes.is_empty() {
                tiles.sort_by_key(|&i| rank(planet.tiles[i].biome));
            }
            if let Some(&tile) = tiles.first() {
                apply(planet, catalog, now, &Action::Build { building: id, tile }).unwrap();
                break;
            }
        }
    }
}
