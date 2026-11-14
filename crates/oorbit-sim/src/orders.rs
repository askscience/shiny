//! Where the people work. The player points at a cell (a forest, a hill, a
//! meadow, a shallow sea) and a crew goes there for one of its resources;
//! the crew stays until told otherwise and moves on by itself when the cell
//! is spent. Buildings that extract something (mines) keep a crew of their
//! own. Whoever has no order thinks and makes culture.
//!
//! Food and wood grow back (slowly for a forest, faster with trees
//! planted); stone, clay and ores come out of the cell's deposit, which
//! never refills: a quarry ends, and the people look further.

use crate::catalog::{Biome, Catalog, Category};
use crate::planet::{Order, Planet};

/// Crews the workforce is split into: each order (or extracting building)
/// takes one share of the workers; with more, the shares shrink.
pub const SLOTS: usize = 9;
/// The share of the workers the crews can take together: the rest always
/// think and make culture.
pub const CREW_SHARE: f64 = 0.75;

/// How far from the settled land people go to work, in cells (over land or
/// sea).
pub const WORK_REACH: usize = crate::step::REACH + 1;

/// Stock a crew spends per hour, and what a cell regains per hour at rest
/// (1.0 is a full cell): meadows, game and fish come back in a day or two,
/// a forest in a week (four times faster where trees were planted).
const SPEND_PER_HOUR: f64 = 0.02;
const REGROW_PER_HOUR: f64 = 0.015;
const WOOD_SPEND_PER_HOUR: f64 = 0.025;
const WOOD_REGROW_PER_HOUR: f64 = 0.008;
const PLANTED_REGROW: f64 = 4.0;
/// How long planted trees keep growing fast, in hours.
pub const PLANTED_HOURS: f64 = 48.0;
/// What planting trees costs (food for the planters).
pub const PLANT_FOOD: f64 = 6.0;
/// A mine draws on its cell's deposit at this share of its output (shafts
/// go deep).
const MINE_DRAW: f64 = 0.25;
/// Below this the crew moves to the nearest fuller cell of the same kind.
const SPENT: f64 = 0.15;

/// Where food can be gathered: meadows, forests (game), beaches and the
/// shallow sea along them (fish).
fn food_ground(planet: &Planet, tile: usize) -> bool {
    match planet.tiles[tile].biome {
        Biome::Plains | Biome::Forest | Biome::Coast => true,
        Biome::Ocean => crate::cells::layout(planet.tiles.len()).is_some_and(|l| {
            l.neighbors[tile]
                .iter()
                .any(|&n| planet.tiles[n].biome != Biome::Ocean)
        }),
        _ => false,
    }
}

/// The resources a crew could gather on `tile` now (food first).
pub fn harvestable(planet: &Planet, catalog: &Catalog, tile: usize) -> Vec<String> {
    let Some(t) = planet.tiles.get(tile) else {
        return Vec::new();
    };
    if t.building.is_some() || planet.construction.iter().any(|c| c.tile == tile) {
        return Vec::new();
    }
    let mut list = Vec::new();
    if food_ground(planet, tile) {
        list.push("food".to_string());
    }
    for r in &catalog.resources {
        if r.biome != Some(t.biome) || r.from_building.is_some() {
            continue;
        }
        if r.deposit && t.deposit <= 0.0 {
            continue;
        }
        if r.unlocked_by.as_ref().is_some_and(|k| !planet.known.contains(k)) {
            continue;
        }
        if catalog.resource_era(&r.id) > planet.era {
            continue;
        }
        list.push(r.id.clone());
    }
    list
}

/// Cells people can reach to work, as a mask.
pub fn in_reach(planet: &Planet) -> Vec<bool> {
    let Some(layout) = crate::cells::layout(planet.tiles.len()) else {
        return vec![false; planet.tiles.len()];
    };
    let mut settled = vec![planet.home];
    settled.extend(
        planet
            .tiles
            .iter()
            .enumerate()
            .filter(|(_, t)| t.building.is_some())
            .map(|(i, _)| i),
    );
    settled.extend(planet.construction.iter().map(|c| c.tile));
    layout
        .hops(&settled, |_| true)
        .into_iter()
        .map(|d| d <= WORK_REACH)
        .collect()
}

/// Buildings with a crew of their own: the ones a resource is extracted
/// from (a mine), with the resources they yield now.
pub fn extractors(planet: &Planet, catalog: &Catalog) -> Vec<(usize, Vec<String>)> {
    let mut list = Vec::new();
    for (i, t) in planet.tiles.iter().enumerate() {
        let Some(b) = &t.building else { continue };
        let yields: Vec<String> = catalog
            .resources
            .iter()
            .filter(|r| r.from_building.as_deref() == Some(b.as_str()))
            .filter(|r| r.unlocked_by.as_ref().is_none_or(|k| planet.known.contains(k)))
            .map(|r| r.id.clone())
            .collect();
        if !yields.is_empty() && t.deposit > 0.0 {
            list.push((i, yields));
        }
    }
    list
}

/// Workers in each crew (orders and extractors alike).
pub fn crew(planet: &Planet, catalog: &Catalog) -> f64 {
    let slots = planet.orders.len() + extractors(planet, catalog).len();
    planet.population * catalog.tuning.worker_share * CREW_SHARE / slots.max(SLOTS) as f64
}

/// What a cell lets a crew gather of `resource`: all of it while the
/// deposit lasts, or as much as the stock of food or trees allows (a bare
/// cell gives almost nothing).
pub fn yield_factor(catalog: &Catalog, resource: &str, stock: f64) -> f64 {
    if is_deposit(catalog, resource) {
        1.0
    } else {
        0.1 + 0.9 * stock.clamp(0.0, 1.0)
    }
}

/// Is `resource` dug out of a deposit?
pub fn is_deposit(catalog: &Catalog, resource: &str) -> bool {
    catalog.resource(resource).is_some_and(|r| r.deposit)
}

/// Can trees be planted on `tile` (a thinned forest nobody works)?
pub fn can_plant(planet: &Planet, tile: usize) -> bool {
    planet.tiles.get(tile).is_some_and(|t| {
        t.biome == Biome::Forest && t.building.is_none() && t.stock < 0.9 && t.planted_until <= planet.step
    }) && !planet.orders.iter().any(|o| o.tile == tile)
        && in_reach(planet)[tile]
}

/// The category a resource's work counts as.
pub fn category(catalog: &Catalog, resource: &str) -> Category {
    catalog
        .resource(resource)
        .map(|r| r.category)
        .unwrap_or(Category::Materials)
}

/// Can a crew be sent to gather `resource` on `tile`?
pub fn can_order(planet: &Planet, catalog: &Catalog, tile: usize, resource: &str) -> bool {
    tile < planet.tiles.len()
        && planet.orders.len() < SLOTS
        && !planet.orders.iter().any(|o| o.tile == tile)
        && in_reach(planet)[tile]
        && harvestable(planet, catalog, tile).iter().any(|r| r == resource)
}

/// The free cell nearest to the home where `resource` can be gathered, in
/// reach, without an order, with at least `min_stock` left.
pub fn nearest_for(planet: &Planet, catalog: &Catalog, resource: &str, min_stock: f64) -> Option<usize> {
    let layout = crate::cells::layout(planet.tiles.len())?;
    let reach = in_reach(planet);
    let from_home = layout.hops(&[planet.home], |_| true);
    let deposit = is_deposit(catalog, resource);
    (0..planet.tiles.len())
        .filter(|&i| reach[i] && (deposit || planet.tiles[i].stock >= min_stock))
        .filter(|&i| !planet.orders.iter().any(|o| o.tile == i))
        .filter(|&i| harvestable(planet, catalog, i).iter().any(|r| r == resource))
        .min_by_key(|&i| (from_home[i], i))
}

/// A new planet's first crews: food, wood and stone near the home.
pub fn first_orders(planet: &mut Planet, catalog: &Catalog) {
    planet.orders_given = true;
    for resource in ["food", "food", "food", "wood", "wood", "stone"] {
        if let Some(tile) = nearest_for(planet, catalog, resource, 0.5) {
            planet.orders.push(Order {
                tile,
                resource: resource.into(),
            });
        }
    }
}

/// One step of work on the land: crews spend their cells (`outputs` is
/// what each order gathers per hour, `extracted` what each mine digs),
/// cells at rest refill, deposits run out for good, and a crew on a spent
/// cell moves to the nearest fuller one (or comes home when there is none).
pub fn run_orders(planet: &mut Planet, catalog: &Catalog, step: i64, outputs: &[f64], extracted: &[(usize, f64)]) {
    let h = crate::STEP_HOURS;
    let mut worked = vec![None; planet.tiles.len()];
    for (k, o) in planet.orders.iter().enumerate() {
        if o.tile < worked.len() {
            worked[o.tile] = Some((o.resource.clone(), outputs.get(k).copied().unwrap_or(0.0)));
        }
    }
    let mut emptied = Vec::new();
    for (i, t) in planet.tiles.iter_mut().enumerate() {
        match &worked[i] {
            Some((resource, out)) if is_deposit(catalog, resource) => {
                if t.deposit > 0.0 {
                    t.deposit = (t.deposit - out * h).max(0.0);
                    if t.deposit == 0.0 {
                        emptied.push((resource.clone(), i));
                    }
                }
            }
            Some((resource, _)) => {
                let spend = if resource == "wood" { WOOD_SPEND_PER_HOUR } else { SPEND_PER_HOUR };
                t.stock = (t.stock - spend * h).max(0.0);
            }
            None if t.stock < 1.0 => {
                let mut regrow = if t.biome == Biome::Forest { WOOD_REGROW_PER_HOUR } else { REGROW_PER_HOUR };
                if t.planted_until > step {
                    regrow *= PLANTED_REGROW;
                }
                t.stock = (t.stock + regrow * h).min(1.0);
            }
            None => {}
        }
    }
    for &(cell, out) in extracted {
        let Some(t) = planet.tiles.get_mut(cell) else { continue };
        if t.deposit > 0.0 {
            t.deposit = (t.deposit - out * MINE_DRAW * h).max(0.0);
            if t.deposit == 0.0 {
                let b = t.building.clone().unwrap_or_default();
                emptied.push((b, cell));
            }
        }
    }
    for (what, cell) in emptied {
        let tile = cell.to_string();
        planet.log(catalog, step, "deposit_exhausted", &[("resource", &what), ("tile", &tile)]);
    }
    // Orders that no longer make sense (a building went up there, the
    // deposit is empty) and crews on spent cells move to the nearest cell
    // where they can work; with none left, they come home and think.
    let mut k = 0;
    while k < planet.orders.len() {
        let order = planet.orders[k].clone();
        let valid = harvestable(planet, catalog, order.tile).contains(&order.resource);
        let spent = !is_deposit(catalog, &order.resource) && planet.tiles[order.tile].stock < SPENT;
        if valid && !spent {
            k += 1;
            continue;
        }
        if let Some(next) = nearest_for(planet, catalog, &order.resource, 0.5) {
            planet.orders[k].tile = next;
            let (from, to) = (order.tile.to_string(), next.to_string());
            planet.log(
                catalog,
                step,
                "crew_moved",
                &[("resource", &order.resource), ("from", &from), ("tile", &to)],
            );
            k += 1;
        } else if !valid || planet.tiles[order.tile].stock < 0.05 {
            planet.orders.remove(k);
            planet.log(catalog, step, "crew_home", &[("resource", &order.resource)]);
        } else {
            k += 1;
        }
    }
}
