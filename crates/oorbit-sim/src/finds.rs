//! Things that turn up on the land for a while: berries, lost goats, a crate
//! washed ashore, a fallen meteorite, wild horses, a wreck, old ruins. The
//! player taps one and the people bring it home at once; left alone, it is
//! gone after a few hours. They give a reason to turn the planet around.
//!
//! Finds draw from their own random stream (`Rng::for_step` with a salt),
//! so they never shift the draws of events or neighbours.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::catalog::{Biome, Catalog};
use crate::planet::Planet;
use crate::rng::Rng;
use crate::{hours_to_steps, STEP_HOURS};

/// Chance per hour that something turns up, at most [`MAX_FINDS`] at once.
pub const FINDS_PER_HOUR: f64 = 0.3;
pub const MAX_FINDS: usize = 3;
/// How long a find waits, in hours.
pub const FIND_HOURS: f64 = 8.0;
const SALT: u64 = 0xF1_4D5;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Find {
    pub id: u64,
    pub kind: String,
    pub tile: usize,
    pub until_step: i64,
}

/// Every kind of find: where it turns up and what it brings (before the
/// era's growth).
pub const KINDS: [(&str, &[Biome]); 7] = [
    ("berries", &[Biome::Plains, Biome::Forest]),
    ("goats", &[Biome::Plains, Biome::Mountain]),
    ("crate", &[Biome::Coast]),
    ("meteorite", &[Biome::Desert, Biome::Mountain, Biome::Ice, Biome::Plains]),
    ("horses", &[Biome::Plains, Biome::Desert]),
    ("wreck", &[Biome::Coast]),
    ("ruins", &[Biome::Desert, Biome::Forest, Biome::Mountain]),
];

/// What a find brings at the planet's era: resources (more in later eras).
pub fn reward(planet: &Planet, catalog: &Catalog, kind: &str) -> BTreeMap<String, f64> {
    let grow = 1.0 + 0.6 * (planet.era as f64 - 1.0);
    let base: &[(&str, f64)] = match kind {
        "berries" => &[("food", 18.0)],
        "goats" => &[("food", 14.0), ("culture", 4.0)],
        "crate" => &[("wood", 14.0), ("food", 6.0)],
        "meteorite" => &[("stone", 12.0), ("knowledge", 10.0)],
        "horses" => &[("culture", 10.0), ("food", 6.0)],
        "wreck" => &[("wood", 24.0)],
        "ruins" => &[("knowledge", 18.0), ("culture", 6.0)],
        _ => &[],
    };
    let mut out: BTreeMap<String, f64> = base.iter().map(|(r, a)| (r.to_string(), (a * grow).round())).collect();
    // Later eras find something of their own: the newest gatherable
    // material of the era (bronze, iron...) in small amounts.
    if planet.era >= 3 && kind == "wreck" {
        if let Some(r) = catalog
            .resources
            .iter()
            .filter(|r| catalog.resource_era(&r.id) == planet.era && !r.uncapped)
            .next_back()
        {
            *out.entry(r.id.clone()).or_insert(0.0) += (5.0 * grow).round();
        }
    }
    out
}

/// Cells where finds may turn up: in reach of the people, free, not worked.
fn spots(planet: &Planet, biomes: &[Biome]) -> Vec<usize> {
    let reach = crate::orders::in_reach(planet);
    (0..planet.tiles.len())
        .filter(|&i| reach[i] && biomes.contains(&planet.tiles[i].biome))
        .filter(|&i| planet.tiles[i].building.is_none())
        .filter(|&i| !planet.construction.iter().any(|c| c.tile == i))
        .filter(|&i| !planet.orders.iter().any(|o| o.tile == i))
        .filter(|&i| !planet.finds.iter().any(|f| f.tile == i))
        .collect()
}

/// One step: old finds go, something new may turn up.
pub fn run_finds(planet: &mut Planet, catalog: &Catalog, step: i64) {
    let _ = catalog;
    planet.finds.retain(|f| f.until_step > step);
    let mut rng = Rng::for_step(planet.seed ^ SALT, step);
    let roll = rng.unit();
    let kind_pick = rng.below(KINDS.len());
    let spot_pick = rng.next_u64();
    if planet.finds.len() >= MAX_FINDS || roll >= FINDS_PER_HOUR * STEP_HOURS {
        return;
    }
    let (kind, biomes) = KINDS[kind_pick];
    let cells = spots(planet, biomes);
    if cells.is_empty() {
        return;
    }
    let tile = cells[(spot_pick % cells.len() as u64) as usize];
    planet.next_id += 1;
    planet.finds.push(Find {
        id: planet.next_id,
        kind: kind.to_string(),
        tile,
        until_step: step + hours_to_steps(FIND_HOURS),
    });
}

/// Bring a find home: its reward now. False when there is no such find.
pub fn collect(planet: &mut Planet, catalog: &Catalog, step: i64, id: u64) -> bool {
    let Some(k) = planet.finds.iter().position(|f| f.id == id) else {
        return false;
    };
    let find = planet.finds.remove(k);
    let reward = reward(planet, catalog, &find.kind);
    for (r, a) in &reward {
        planet.add(r, *a);
    }
    planet.finds_collected += 1;
    let tile = find.tile.to_string();
    planet.log(catalog, step, "find_collected", &[("find", &find.kind), ("tile", &tile)]);
    true
}
