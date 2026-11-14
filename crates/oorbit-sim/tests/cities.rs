//! Cities, roads and sea routes: a colony founds a city, a road links it,
//! and a city with no way to the capital works slower.
mod common;

use common::{HOUR, T0};
use oorbit_sim::catalog::Biome;
use oorbit_sim::cities::{self, LinkKind};
use oorbit_sim::{apply, advance, Action, Catalog, Planet};

fn rich(planet: &mut Planet) {
    planet.era = 2;
    planet.known.insert("farming".into());
    for r in ["food", "wood", "stone", "clay"] {
        planet.resources.insert(r.into(), 400.0);
    }
}

/// A planet with a founded colony (not linked yet).
fn with_colony(seed: u64) -> (Planet, Catalog, i64) {
    let catalog = Catalog::builtin();
    let mut planet = Planet::new(seed, "temperate", T0, &catalog).unwrap();
    rich(&mut planet);
    let site = cities::sites_for(&planet, &catalog, catalog.building("colony").unwrap())[0];
    apply(&mut planet, &catalog, T0, &Action::Build { building: "colony".into(), tile: site }).unwrap();
    let now = T0 + 7 * HOUR;
    advance(&mut planet, &catalog, now);
    assert_eq!(planet.cities.len(), 2, "the colony founded a city");
    assert_eq!(planet.cities[1].tile, site);
    (planet, catalog, now)
}

#[test]
fn a_new_planet_has_its_capital_on_the_home() {
    let catalog = Catalog::builtin();
    let planet = Planet::new(5, "arid", T0, &catalog).unwrap();
    assert_eq!(planet.cities.len(), 1);
    assert_eq!(planet.cities[0].tile, planet.home);
    assert!(!planet.cities[0].name.is_empty());
}

#[test]
fn colonies_keep_their_distance_and_buildings_stay_near_a_city() {
    let catalog = Catalog::builtin();
    let mut planet = Planet::new(3, "temperate", T0, &catalog).unwrap();
    rich(&mut planet);
    let layout = oorbit_sim::cells::layout(planet.tiles.len()).unwrap();
    let apart = layout.hops(&[planet.home], |_| true);
    let sites = cities::city_sites(&planet, &catalog);
    assert!(!sites.is_empty());
    for &s in &sites {
        assert!(apart[s] >= cities::CITY_SPACING);
        assert_ne!(planet.tiles[s].biome, Biome::Ocean);
    }
    let near = layout.hops(&[planet.home], |c| planet.tiles[c].biome != Biome::Ocean);
    for c in oorbit_sim::step::buildable(&planet) {
        assert!(near[c] <= cities::CITY_RADIUS);
    }
    // A colony anywhere else is refused.
    let bad = oorbit_sim::step::buildable(&planet)[0];
    assert!(apply(&mut planet, &catalog, T0, &Action::Build { building: "colony".into(), tile: bad }).is_err());
}

#[test]
fn a_road_links_a_colony_and_lifts_its_pace() {
    let (mut planet, catalog, now) = with_colony(3);
    assert_eq!(cities::connected(&planet), vec![true, false]);
    let colony = planet.cities[1].tile;
    assert!(cities::pace(&planet)[colony] < 1.0, "unlinked colonies work slower");
    assert!(apply(&mut planet, &catalog, now, &Action::Demolish { tile: colony }).is_err());

    let route = cities::route(&planet, &catalog, LinkKind::Road, 0, 1).expect("a road over land");
    for w in route.cells.windows(2) {
        assert!(oorbit_sim::cells::layout(642).unwrap().neighbors[w[0]].contains(&w[1]));
    }
    assert!(route.cells.iter().all(|&c| planet.tiles[c].biome != Biome::Ocean));
    let wood = planet.amount("wood");
    apply(&mut planet, &catalog, now, &Action::Link { kind: LinkKind::Road, from: 0, to: 1 }).unwrap();
    assert!(planet.amount("wood") < wood, "the road is paid for");
    assert!(apply(&mut planet, &catalog, now, &Action::Link { kind: LinkKind::Road, from: 1, to: 0 }).is_err());
    assert_eq!(cities::connected(&planet), vec![true, false], "not before it is laid");
    advance(&mut planet, &catalog, now + (route.hours as i64 + 1) * HOUR);
    assert_eq!(cities::connected(&planet), vec![true, true]);
    assert_eq!(cities::pace(&planet)[colony], 1.0);
    assert!(cities::link_bonus(&planet) > 0.0);
    assert!(planet.chronicle.iter().any(|e| e.key == "road_done"));
}

#[test]
fn a_boat_links_two_cities_on_the_same_sea() {
    let catalog = Catalog::builtin();
    for seed in 1..20u64 {
        let (mut planet, _, now) = with_colony(seed);
        if cities::shores_of(&planet, 0).is_empty() || cities::shores_of(&planet, 1).is_empty() {
            assert!(cities::route(&planet, &catalog, LinkKind::Sea, 0, 1).is_none(), "no shore, no boat");
            continue;
        }
        let Some(route) = cities::route(&planet, &catalog, LinkKind::Sea, 0, 1) else {
            continue;
        };
        let inner = &route.cells[1..route.cells.len() - 1];
        assert!(inner.iter().all(|&c| planet.tiles[c].biome == Biome::Ocean), "a boat sails only on water");
        assert!(route.cost.contains_key("wood"));
        apply(&mut planet, &catalog, now, &Action::Link { kind: LinkKind::Sea, from: 0, to: 1 }).unwrap();
        advance(&mut planet, &catalog, now + 4 * HOUR);
        assert_eq!(cities::connected(&planet), vec![true, true]);
        return;
    }
    panic!("no seed had two cities on the same sea");
}
