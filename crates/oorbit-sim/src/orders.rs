//! Where the people work. The player points at a cell (a forest, a hill, a
//! meadow, a shallow sea) and a crew goes there for one of its resources;
//! the crew stays until told otherwise and moves on by itself when the cell
//! is spent. Buildings that extract something (mines) keep a crew of their
//! own. Whoever has no order thinks and makes culture.

use crate::catalog::{Biome, Catalog, Category};
use crate::planet::{Order, Planet};

/// Crews the workforce is split into: each order (or extracting building)
/// takes one share of the workers; with more, the shares shrink.
pub const SLOTS: usize = 9;

/// How far from the settled land people go to work, in cells (over land or
/// sea).
pub const WORK_REACH: usize = crate::step::REACH + 1;

/// Stock a full crew spends per hour, and what a cell regains per hour at
/// rest (1.0 is a full cell).
const SPEND_PER_HOUR: f64 = 0.02;
const REGROW_PER_HOUR: f64 = 0.015;
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
        if !yields.is_empty() {
            list.push((i, yields));
        }
    }
    list
}

/// Workers in each crew (orders and extractors alike).
pub fn crew(planet: &Planet, catalog: &Catalog) -> f64 {
    let slots = planet.orders.len() + extractors(planet, catalog).len();
    planet.population * catalog.tuning.worker_share / slots.max(SLOTS) as f64
}

/// What a cell's stock lets a crew gather (a spent cell still gives a little).
pub fn yield_factor(stock: f64) -> f64 {
    0.4 + 0.6 * stock.clamp(0.0, 1.0)
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
    (0..planet.tiles.len())
        .filter(|&i| reach[i] && planet.tiles[i].stock >= min_stock)
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

/// One step of work on the land: crews spend their cells, cells at rest
/// refill, and a crew on a spent cell moves to the nearest fuller one.
pub fn run_orders(planet: &mut Planet, catalog: &Catalog, step: i64) {
    let worked: Vec<usize> = planet.orders.iter().map(|o| o.tile).collect();
    for (i, t) in planet.tiles.iter_mut().enumerate() {
        if worked.contains(&i) {
            t.stock = (t.stock - SPEND_PER_HOUR * crate::STEP_HOURS).max(0.0);
        } else if t.stock < 1.0 {
            t.stock = (t.stock + REGROW_PER_HOUR * crate::STEP_HOURS).min(1.0);
        }
    }
    // Orders that no longer make sense (a building went up there, the
    // resource is gone) are dropped; spent cells are left for fuller ones.
    let mut k = 0;
    while k < planet.orders.len() {
        let order = planet.orders[k].clone();
        let valid = harvestable(planet, catalog, order.tile)
            .iter()
            .any(|r| *r == order.resource);
        if !valid {
            planet.orders.remove(k);
            continue;
        }
        if planet.tiles[order.tile].stock < SPENT {
            if let Some(next) = nearest_for(planet, catalog, &order.resource, 0.6) {
                planet.orders[k].tile = next;
                let (from, to) = (order.tile.to_string(), next.to_string());
                planet.log(
                    catalog,
                    step,
                    "crew_moved",
                    &[("resource", &order.resource), ("from", &from), ("tile", &to)],
                );
            }
        }
        k += 1;
    }
}
