//! Cities and the ways between them. The planet starts with one city on its
//! home; a colony built far enough away founds another. New buildings go
//! near a city, so a growing people needs new cities. A road over land or a
//! sea route between two ports links two cities; the crews of a city with
//! no way to the capital work at [`UNLINKED`] of their pace, and every
//! linked colony lifts knowledge and culture a little (ideas travel).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::catalog::{Biome, Catalog};
use crate::planet::Planet;
use crate::hours_to_steps;

/// How far from a city's centre new buildings may go, in cells over land.
pub const CITY_RADIUS: usize = 4;
/// A new city stands at least this many cells from every other one.
pub const CITY_SPACING: usize = 5;
/// How far over land settlers walk to found a city, in cells.
pub const COLONY_RANGE: usize = 8;
/// With a port, settlers sail to any shore this many cells away.
pub const SEA_RANGE: usize = 16;
/// The pace of a city's crews while it has no way to the capital.
pub const UNLINKED: f64 = 0.6;
/// Knowledge and culture bonus per linked colony, up to [`MAX_LINKED`].
pub const LINK_BONUS: f64 = 0.05;
pub const MAX_LINKED: usize = 4;
/// Hours to lay one cell of road, and to fit out a sea route.
pub const ROAD_HOURS_PER_CELL: f64 = 0.5;
pub const SEA_HOURS: f64 = 3.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct City {
    pub tile: usize,
    pub name: String,
    pub founded_step: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkKind {
    Road,
    Sea,
}

impl LinkKind {
    pub fn as_str(self) -> &'static str {
        match self {
            LinkKind::Road => "road",
            LinkKind::Sea => "sea",
        }
    }
}

/// A road or a sea route between two cities (indices into `cities`), along
/// `cells`, usable from `done_step` (a road is laid cell by cell).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Link {
    pub kind: LinkKind,
    pub from: usize,
    pub to: usize,
    pub cells: Vec<usize>,
    pub start_step: i64,
    pub done_step: i64,
}

/// A way that could be opened now: where it goes and what it takes.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Route {
    pub kind: LinkKind,
    pub from: usize,
    pub to: usize,
    pub cells: Vec<usize>,
    pub cost: BTreeMap<String, f64>,
    pub hours: f64,
}

/// A city name drawn from the seed: the same planet always names its cities
/// the same way.
pub fn city_name(seed: u64, index: usize) -> String {
    const START: [&str; 20] = [
        "Ar", "Bel", "Cor", "Dun", "El", "Fen", "Gar", "Hol", "Ir", "Kel", "Lor", "Mar", "Nor",
        "Os", "Pel", "Ran", "Sol", "Tor", "Ul", "Ver",
    ];
    const END: [&str; 12] = [
        "a", "en", "is", "or", "um", "eth", "ia", "on", "ar", "ic", "ost", "wyn",
    ];
    let mut x = seed ^ (index as u64 + 1).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    x ^= x >> 31;
    x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x ^= x >> 29;
    let a = START[(x % START.len() as u64) as usize];
    let b = END[((x >> 16) % END.len() as u64) as usize];
    format!("{a}{b}")
}

/// Give a planet without cities its capital, on the home.
pub fn ensure_capital(planet: &mut Planet) {
    if planet.cities.is_empty() {
        planet.cities.push(City {
            tile: planet.home,
            name: city_name(planet.seed, 0),
            founded_step: planet.created_step,
        });
    }
}

fn land(planet: &Planet, cell: usize) -> bool {
    planet.tiles[cell].biome != Biome::Ocean
}

/// The city each cell belongs to: the nearest centre (lowest index on a
/// tie), counting cells over land and sea alike.
pub fn owners(planet: &Planet) -> Vec<usize> {
    let n = planet.tiles.len();
    let mut owner = vec![0; n];
    let Some(layout) = crate::cells::layout(n) else {
        return owner;
    };
    let mut seen = vec![false; n];
    let mut queue = std::collections::VecDeque::new();
    for (k, c) in planet.cities.iter().enumerate() {
        if c.tile < n && !seen[c.tile] {
            seen[c.tile] = true;
            owner[c.tile] = k;
            queue.push_back(c.tile);
        }
    }
    while let Some(c) = queue.pop_front() {
        for &m in &layout.neighbors[c] {
            if !seen[m] {
                seen[m] = true;
                owner[m] = owner[c];
                queue.push_back(m);
            }
        }
    }
    owner
}

/// Cells within [`CITY_RADIUS`] of a city centre, over land.
pub fn near_city(planet: &Planet) -> Vec<bool> {
    let Some(layout) = crate::cells::layout(planet.tiles.len()) else {
        return vec![false; planet.tiles.len()];
    };
    let centres: Vec<usize> = planet.cities.iter().map(|c| c.tile).collect();
    layout
        .hops(&centres, |c| land(planet, c))
        .into_iter()
        .map(|d| d <= CITY_RADIUS)
        .collect()
}

/// Is `building` a landing for boats?
pub fn is_port(catalog: &Catalog, building: &str) -> bool {
    catalog.building(building).is_some_and(|b| b.port)
}

/// The ports a city has (port buildings on its cells).
pub fn ports_of(planet: &Planet, catalog: &Catalog, city: usize) -> Vec<usize> {
    let owner = owners(planet);
    planet
        .tiles
        .iter()
        .enumerate()
        .filter(|(i, t)| owner[*i] == city && t.building.as_deref().is_some_and(|b| is_port(catalog, b)))
        .map(|(i, _)| i)
        .collect()
}

/// Where a colony could be founded now: free land far enough from every
/// city and colony site, within walking range of a city over land, or by
/// any shore within sailing range once a port stands. Nearest to the home
/// first.
pub fn city_sites(planet: &Planet, catalog: &Catalog) -> Vec<usize> {
    let Some(layout) = crate::cells::layout(planet.tiles.len()) else {
        return Vec::new();
    };
    let mut centres: Vec<usize> = planet.cities.iter().map(|c| c.tile).collect();
    centres.extend(
        planet
            .construction
            .iter()
            .filter(|c| catalog.building(&c.building).is_some_and(|b| b.city))
            .map(|c| c.tile),
    );
    let apart = layout.hops(&centres, |_| true);
    let city_tiles: Vec<usize> = planet.cities.iter().map(|c| c.tile).collect();
    let walk = layout.hops(&city_tiles, |c| land(planet, c));
    let has_port = planet
        .tiles
        .iter()
        .any(|t| t.building.as_deref().is_some_and(|b| is_port(catalog, b)));
    let sail = layout.hops(&city_tiles, |_| true);
    let shore = |c: usize| layout.neighbors[c].iter().any(|&m| !land(planet, m));
    let from_home = layout.hops(&[planet.home], |_| true);
    let mut sites: Vec<usize> = (0..planet.tiles.len())
        .filter(|&c| {
            land(planet, c)
                && planet.tiles[c].building.is_none()
                && !planet.construction.iter().any(|w| w.tile == c)
                && apart[c] >= CITY_SPACING
                && (walk[c] <= COLONY_RANGE || (has_port && shore(c) && sail[c] <= SEA_RANGE))
        })
        .collect();
    sites.sort_by_key(|&c| (from_home[c], c));
    sites
}

/// Cells a building of `def` could go on now, nearest to the home first.
pub fn sites_for(planet: &Planet, catalog: &Catalog, def: &crate::catalog::BuildingDef) -> Vec<usize> {
    let cells = if def.city {
        city_sites(planet, catalog)
    } else {
        crate::step::buildable(planet)
    };
    cells
        .into_iter()
        .filter(|&i| def.biomes.is_empty() || def.biomes.contains(&planet.tiles[i].biome))
        .collect()
}

/// What laying a road along `cells` costs: wood and stone per cell walked.
fn road_cost(cells: &[usize]) -> BTreeMap<String, f64> {
    let n = cells.len().saturating_sub(1) as f64;
    BTreeMap::from([("wood".to_string(), 3.0 * n), ("stone".to_string(), 2.0 * n)])
}

/// What fitting out a sea route takes: boats, and a little per cell sailed.
fn sea_cost(cells: &[usize]) -> BTreeMap<String, f64> {
    let n = cells.len().saturating_sub(1) as f64;
    BTreeMap::from([("wood".to_string(), 20.0 + 1.5 * n)])
}

/// The price of walking into a cell when laying a road.
fn road_price(biome: Biome) -> Option<u32> {
    match biome {
        Biome::Ocean => None,
        Biome::Plains | Biome::Coast | Biome::Desert => Some(2),
        Biome::Forest => Some(3),
        Biome::Ice => Some(4),
        Biome::Mountain => Some(6),
    }
}

/// The road from one city to another, if there is land all the way.
pub fn road_path(planet: &Planet, from: usize, to: usize) -> Option<Vec<usize>> {
    let layout = crate::cells::layout(planet.tiles.len())?;
    let (a, b) = (planet.cities.get(from)?.tile, planet.cities.get(to)?.tile);
    layout.cheapest_path(&[a], &[b], |c| road_price(planet.tiles[c].biome))
}

/// The sea route from one city's ports to another's, if the water joins.
pub fn sea_path(planet: &Planet, catalog: &Catalog, from: usize, to: usize) -> Option<Vec<usize>> {
    let layout = crate::cells::layout(planet.tiles.len())?;
    let a = ports_of(planet, catalog, from);
    let b = ports_of(planet, catalog, to);
    if a.is_empty() || b.is_empty() {
        return None;
    }
    layout.cheapest_path(&a, &b, |c| (!land(planet, c) || b.contains(&c)).then_some(1))
}

/// Is there already a way of this kind between two cities (built or not)?
pub fn linked(planet: &Planet, kind: LinkKind, a: usize, b: usize) -> bool {
    planet
        .links
        .iter()
        .any(|l| l.kind == kind && ((l.from == a && l.to == b) || (l.from == b && l.to == a)))
}

/// The way of `kind` from one city to another that could be opened now.
pub fn route(planet: &Planet, catalog: &Catalog, kind: LinkKind, from: usize, to: usize) -> Option<Route> {
    if from == to || from >= planet.cities.len() || to >= planet.cities.len() || linked(planet, kind, from, to) {
        return None;
    }
    let _ = catalog;
    let (cells, cost, hours) = match kind {
        LinkKind::Road => {
            let cells = road_path(planet, from, to)?;
            let cost = road_cost(&cells);
            let hours = ROAD_HOURS_PER_CELL * cells.len().saturating_sub(1) as f64;
            (cells, cost, hours)
        }
        LinkKind::Sea => {
            let cells = sea_path(planet, catalog, from, to)?;
            let cost = sea_cost(&cells);
            (cells, cost, SEA_HOURS)
        }
    };
    Some(Route {
        kind,
        from,
        to,
        cells,
        cost,
        hours,
    })
}

/// Every way that could be opened now, between any two cities.
pub fn routes(planet: &Planet, catalog: &Catalog) -> Vec<Route> {
    let mut list = Vec::new();
    for a in 0..planet.cities.len() {
        for b in a + 1..planet.cities.len() {
            for kind in [LinkKind::Road, LinkKind::Sea] {
                if let Some(r) = route(planet, catalog, kind, a, b) {
                    list.push(r);
                }
            }
        }
    }
    list
}

/// Open a way: pay for it and start the work. The caller checked `route`.
pub fn open(planet: &mut Planet, catalog: &Catalog, step: i64, r: Route) {
    for (id, need) in &r.cost {
        planet.add(id, -need);
    }
    let names = (planet.cities[r.from].name.clone(), planet.cities[r.to].name.clone());
    planet.links.push(Link {
        kind: r.kind,
        from: r.from,
        to: r.to,
        cells: r.cells,
        start_step: step,
        done_step: step + hours_to_steps(r.hours),
    });
    planet.log(
        catalog,
        step,
        &format!("{}_started", r.kind.as_str()),
        &[("from", &names.0), ("to", &names.1)],
    );
}

/// The cities that can reach the capital by finished ways.
pub fn connected(planet: &Planet) -> Vec<bool> {
    let mut reach = vec![false; planet.cities.len()];
    if reach.is_empty() {
        return reach;
    }
    reach[0] = true;
    let mut changed = true;
    while changed {
        changed = false;
        for l in planet.links.iter().filter(|l| l.done_step <= planet.step) {
            if l.from < reach.len() && l.to < reach.len() && reach[l.from] != reach[l.to] {
                reach[l.from] = true;
                reach[l.to] = true;
                changed = true;
            }
        }
    }
    reach
}

/// The pace of the crews on each cell: [`UNLINKED`] where the cell's city
/// cannot reach the capital.
pub fn pace(planet: &Planet) -> Vec<f64> {
    if planet.cities.len() < 2 {
        return vec![1.0; planet.tiles.len()];
    }
    let reach = connected(planet);
    owners(planet)
        .into_iter()
        .map(|c| if reach.get(c).copied().unwrap_or(true) { 1.0 } else { UNLINKED })
        .collect()
}

/// Knowledge and culture bonus from the linked colonies.
pub fn link_bonus(planet: &Planet) -> f64 {
    let linked = connected(planet).iter().skip(1).filter(|c| **c).count();
    LINK_BONUS * linked.min(MAX_LINKED) as f64
}

/// One step: colonies that stand now found their city, ways that are done
/// are told.
pub fn run_cities(planet: &mut Planet, catalog: &Catalog, step: i64, finished: &[(String, usize)]) {
    for (building, tile) in finished {
        if catalog.building(building).is_some_and(|b| b.city)
            && !planet.cities.iter().any(|c| c.tile == *tile)
        {
            let name = city_name(planet.seed, planet.cities.len());
            planet.cities.push(City {
                tile: *tile,
                name: name.clone(),
                founded_step: step,
            });
            planet.log(catalog, step, "city_founded", &[("city", &name)]);
        }
    }
    let done: Vec<(LinkKind, String, String)> = planet
        .links
        .iter()
        .filter(|l| l.done_step == step)
        .filter_map(|l| {
            Some((l.kind, planet.cities.get(l.from)?.name.clone(), planet.cities.get(l.to)?.name.clone()))
        })
        .collect();
    for (kind, from, to) in done {
        planet.log(catalog, step, &format!("{}_done", kind.as_str()), &[("from", &from), ("to", &to)]);
    }
}

/// How far along a way is at `step`, 0 to 1.
pub fn progress(link: &Link, step: i64) -> f64 {
    let span = (link.done_step - link.start_step).max(1) as f64;
    ((step - link.start_step) as f64 / span).clamp(0.0, 1.0)
}
