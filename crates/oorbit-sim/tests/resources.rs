//! The land runs out: quarries end for good, forests thin and grow back
//! slowly (faster with trees planted), a crew whose cell is built on moves
//! on, and someone watching the planet keeps it at full pace.
mod common;

use common::{HOUR, T0};
use oorbit_sim::catalog::Biome;
use oorbit_sim::{advance, apply, rates, Action, Catalog, Planet};

fn planet() -> (Planet, Catalog) {
    let catalog = Catalog::builtin();
    (Planet::new(4, "temperate", T0, &catalog).unwrap(), catalog)
}

#[test]
fn a_quarry_runs_out_for_good() {
    let (mut p, catalog) = planet();
    let order = p.orders.iter().position(|o| o.resource == "stone").unwrap();
    let cell = p.orders[order].tile;
    p.tiles[cell].deposit = 3.0;
    advance(&mut p, &catalog, T0 + 40 * HOUR);
    assert_eq!(p.tiles[cell].deposit, 0.0);
    assert!(p.chronicle.iter().any(|e| e.key == "deposit_exhausted"));
    assert!(!p.orders.iter().any(|o| o.tile == cell), "the crew left the empty quarry");
    assert!(!oorbit_sim::orders::harvestable(&p, &catalog, cell).contains(&"stone".to_string()));
    advance(&mut p, &catalog, T0 + 400 * HOUR);
    assert_eq!(p.tiles[cell].deposit, 0.0, "a deposit never grows back");
}

#[test]
fn forests_thin_and_grow_back_faster_where_trees_are_planted() {
    let (mut p, catalog) = planet();
    let cell = p.orders.iter().find(|o| o.resource == "wood").unwrap().tile;
    advance(&mut p, &catalog, T0 + 20 * HOUR);
    let thinned = p.tiles[cell].stock;
    assert!(thinned < 0.7, "worked forest thins ({thinned})");
    let now = T0 + 20 * HOUR;
    // Every wood crew home, so none moves onto the resting forest.
    for tile in p.orders.iter().filter(|o| o.resource == "wood").map(|o| o.tile).collect::<Vec<_>>() {
        apply(&mut p, &catalog, now, &Action::StopHarvest { tile }).unwrap();
    }
    let mut planted = p.clone();
    planted.resources.insert("food".into(), 50.0);
    apply(&mut planted, &catalog, now, &Action::Plant { tile: cell }).unwrap();
    advance(&mut p, &catalog, now + 24 * HOUR);
    advance(&mut planted, &catalog, now + 24 * HOUR);
    assert!(p.tiles[cell].stock > thinned, "a forest at rest grows back");
    assert!(planted.tiles[cell].stock > p.tiles[cell].stock + 0.2, "planted trees grow faster: {} vs {} from {thinned}, worked {:?}", planted.tiles[cell].stock, p.tiles[cell].stock, planted.orders.iter().any(|o| o.tile == cell));
}

#[test]
fn building_on_a_worked_cell_moves_its_crew() {
    let (mut p, catalog) = planet();
    p.known.insert("shelter".into());
    p.resources.insert("wood".into(), 50.0);
    let order = p.orders.iter().find(|o| p.tiles[o.tile].biome == Biome::Plains).unwrap().clone();
    let crews = p.orders.len();
    apply(&mut p, &catalog, T0, &Action::Build { building: "hut".into(), tile: order.tile }).unwrap();
    assert_eq!(p.orders.len(), crews, "the crew was not lost");
    assert!(!p.orders.iter().any(|o| o.tile == order.tile));
    assert!(p.chronicle.iter().any(|e| e.key == "crew_moved"));
}

#[test]
fn watching_the_planet_counts_as_acting() {
    let (mut p, catalog) = planet();
    p.clock = oorbit_sim::clock::Clock::starting(T0);
    // Two real hours of watching at play speed: 120 planet hours.
    let mut wall = T0;
    for _ in 0..240 {
        wall += 30_000;
        p.present(wall);
        let now = p.time_at(wall);
        advance(&mut p, &catalog, now);
    }
    let idle = p.step - p.last_action_step;
    assert!(idle < 6, "presence keeps the planet active ({idle} steps)");
    assert!(rates(&p, &catalog).efficiency > 0.6, "no idleness penalty while watched");
}
