//! The state of one planet. It is plain data (serialized as JSON by the
//! plugin) and changes only through [`crate::advance`] and [`crate::apply`].

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::catalog::{Biome, Catalog, Category};
use crate::cells;
use crate::step_of;

/// Version of the serialized state, for migrations of saved planets.
pub const STATE_VERSION: u32 = 5;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tile {
    pub biome: Biome,
    #[serde(default)]
    pub building: Option<String>,
    /// Radius of the ground at the cell's centre (the sea is at 1.0).
    #[serde(default = "default_height")]
    pub height: f64,
    /// What is left to gather here, 0 to 1: work spends it, rest refills it.
    #[serde(default = "default_stock")]
    pub stock: f64,
    /// What is left in the ground (stone, clay, ores), in units: digging
    /// spends it for good. Negative: not drawn yet (planets from before).
    #[serde(default = "unset")]
    pub deposit: f64,
    /// Trees planted here grow back faster until this step.
    #[serde(default)]
    pub planted_until: i64,
}

fn default_stock() -> f64 {
    1.0
}

fn unset() -> f64 {
    -1.0
}

fn default_height() -> f64 {
    1.0
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Construction {
    pub building: String,
    pub tile: usize,
    /// The step it was started at (0 on planets from before it was kept).
    #[serde(default)]
    pub start_step: i64,
    /// The step at which it stands.
    pub done_step: i64,
    /// The thinkers already helped here (once per site).
    #[serde(default)]
    pub helped: bool,
}

/// A standing order: a crew of the people works this cell for this
/// resource until told otherwise (it moves on by itself when the cell is
/// spent).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Order {
    pub tile: usize,
    pub resource: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingEvent {
    pub id: String,
    pub step: i64,
    /// The cell it happens on, for events with a place.
    #[serde(default)]
    pub tile: Option<usize>,
}

/// A person with a name who boosts one category for a while.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Notable {
    pub name: String,
    pub category: Category,
    pub born_step: i64,
    pub until_step: i64,
}

/// One line of the Chronicle: a key the window (or Laura) turns into words,
/// with its parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub step: i64,
    pub key: String,
    #[serde(default)]
    pub params: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Planet {
    pub version: u32,
    pub seed: u64,
    pub climate: String,
    pub created_step: i64,
    /// The last step processed.
    pub step: i64,
    pub last_action_step: i64,
    /// 1-based era number.
    pub era: u32,
    /// The last era of the catalog is done: nothing more to unlock (yet).
    pub eras_done: bool,
    pub population: f64,
    /// 0..1.
    pub mood: f64,
    pub resources: BTreeMap<String, f64>,
    pub tiles: Vec<Tile>,
    /// The cell the first settlement stands on.
    #[serde(default)]
    pub home: usize,
    pub construction: Vec<Construction>,
    pub known: BTreeSet<String>,
    pub research: Option<String>,
    /// Where the people work (see `Order`); the rest think and make culture.
    #[serde(default)]
    pub orders: Vec<Order>,
    /// The first orders were given (a new or reborn planet gets some).
    #[serde(default)]
    pub orders_given: bool,
    pub pending_event: Option<PendingEvent>,
    pub last_event_step: i64,
    pub traits: BTreeMap<String, i32>,
    pub notables: Vec<Notable>,
    pub chronicle: Vec<Entry>,
    /// Highest population milestone already announced (multiples of 10).
    pub population_milestone: u32,
    /// True while food ran out on the last step.
    pub starving: bool,
    /// The planets next door (see `neighbors`).
    #[serde(default)]
    pub neighbors: Vec<crate::neighbors::Neighbor>,
    /// Ships under way, in both directions.
    #[serde(default)]
    pub ships: Vec<crate::neighbors::Ship>,
    /// Trade offers waiting for an answer.
    #[serde(default)]
    pub offers: Vec<crate::neighbors::Offer>,
    /// Source of ship and offer ids.
    #[serde(default)]
    pub next_id: u64,
    /// Things to pick up on the land (see `finds`).
    #[serde(default)]
    pub finds: Vec<crate::finds::Find>,
    #[serde(default)]
    pub finds_collected: u32,
    #[serde(default)]
    pub trees_planted: u32,
    /// Goals reached (see `goals`).
    #[serde(default)]
    pub goals_done: BTreeSet<String>,
    /// The thinkers are helping on building sites until this step: no
    /// knowledge meanwhile.
    #[serde(default)]
    pub helping_until: i64,
    /// The cities, the capital first (see `cities`).
    #[serde(default)]
    pub cities: Vec<crate::cities::City>,
    /// Roads and sea routes between cities.
    #[serde(default)]
    pub links: Vec<crate::cities::Link>,
    /// Testing only: how far this planet's clock runs ahead of the wall
    /// clock (the plugin's time-skip tool). The simulation itself never
    /// reads it; the plugin adds it to "now".
    #[serde(default)]
    pub clock_offset_ms: i64,
    /// Planet time against wall time (kept by the plugin, see `clock`).
    #[serde(default)]
    pub clock: crate::clock::Clock,
}

impl Planet {
    /// A new planet founded at `now_ms`. `climate` must be a catalog climate.
    pub fn new(seed: u64, climate: &str, now_ms: i64, catalog: &Catalog) -> Result<Planet, String> {
        let climate_def = catalog
            .climate(climate)
            .ok_or_else(|| format!("unknown climate `{climate}`"))?;
        let layout = cells::layout(catalog.tuning.tiles)
            .ok_or_else(|| format!("no cell layout for {} cells", catalog.tuning.tiles))?;
        let (tiles, home) = crate::terrain::generate(seed, climate_def, layout);

        let step = step_of(now_ms);
        let mut planet = Planet {
            version: STATE_VERSION,
            seed,
            climate: climate.to_string(),
            created_step: step,
            step,
            last_action_step: step,
            era: 1,
            eras_done: false,
            population: catalog.tuning.start_population,
            mood: 0.6,
            resources: BTreeMap::new(),
            tiles,
            home,
            construction: Vec::new(),
            known: BTreeSet::new(),
            research: None,
            orders: Vec::new(),
            orders_given: false,
            pending_event: None,
            last_event_step: step,
            traits: BTreeMap::new(),
            notables: Vec::new(),
            chronicle: Vec::new(),
            population_milestone: 0,
            starving: false,
            neighbors: Vec::new(),
            ships: Vec::new(),
            offers: Vec::new(),
            next_id: 0,
            finds: Vec::new(),
            finds_collected: 0,
            trees_planted: 0,
            goals_done: BTreeSet::new(),
            helping_until: 0,
            cities: Vec::new(),
            links: Vec::new(),
            clock_offset_ms: 0,
            clock: crate::clock::Clock::default(),
        };
        crate::cities::ensure_capital(&mut planet);
        crate::orders::first_orders(&mut planet, catalog);
        crate::neighbors::ensure_neighbors(&mut planet, catalog);
        for r in &catalog.resources {
            planet.resources.insert(r.id.clone(), 0.0);
        }
        for (id, amount) in &catalog.tuning.start_resources {
            planet.resources.insert(id.clone(), *amount);
        }
        planet.log(catalog, step, "founded", &[("climate", climate)]);
        Ok(planet)
    }

    /// Bring a saved planet up to the catalog's cells. A planet saved on
    /// another layout (versions 1 to 3 had 32 or 72 tiles) is reborn with
    /// continents and seas drawn from its seed and climate: it keeps its era,
    /// discoveries, people and stock, and its buildings move to the land
    /// around the new home.
    pub fn fit_tiles(&mut self, catalog: &Catalog) {
        let want = catalog.tuning.tiles;
        if self.tiles.len() != want {
            if let (Some(climate), Some(layout)) =
                (catalog.climate(&self.climate), cells::layout(want))
            {
                let (tiles, home) = crate::terrain::generate(self.seed, climate, layout);
                let old = std::mem::replace(&mut self.tiles, tiles);
                self.home = home;
                self.cities.clear();
                self.links.clear();
                crate::cities::ensure_capital(self);
                let mut moved: Vec<String> = old
                    .into_iter()
                    .filter_map(|t| t.building)
                    .filter(|b| !catalog.building(b).is_some_and(|d| d.city))
                    .collect();
                let mut works = std::mem::take(&mut self.construction);
                // Monuments first, so they find room.
                moved.sort_by_key(|b| !catalog.building(b).is_some_and(|d| d.monument));
                for building in moved {
                    let biomes = catalog
                        .building(&building)
                        .map(|b| b.biomes.clone())
                        .unwrap_or_default();
                    if let Some(&cell) = crate::step::free_tiles(self, &biomes).first() {
                        self.tiles[cell].building = Some(building);
                    }
                }
                for c in works.iter_mut() {
                    let biomes = catalog
                        .building(&c.building)
                        .map(|b| b.biomes.clone())
                        .unwrap_or_default();
                    if let Some(&cell) = crate::step::free_tiles(self, &biomes).first() {
                        c.tile = cell;
                        self.construction.push(c.clone());
                    }
                }
                let step = self.step;
                self.log(catalog, step, "reborn", &[]);
                self.orders.clear();
                self.orders_given = false;
            }
        }
        crate::cities::ensure_capital(self);
        // Buildings the catalog no longer has (the jetty) are cleared.
        for t in self.tiles.iter_mut() {
            if t.building.as_ref().is_some_and(|b| catalog.building(b).is_none()) {
                t.building = None;
            }
        }
        self.construction.retain(|c| catalog.building(&c.building).is_some());
        for (i, t) in self.tiles.iter_mut().enumerate() {
            if t.deposit < 0.0 {
                t.deposit = crate::terrain::deposit_for(self.seed, i, t.biome);
            }
        }
        if !self.orders_given {
            crate::orders::first_orders(self, catalog);
        }
        crate::neighbors::ensure_neighbors(self, catalog);
        self.version = STATE_VERSION;
    }

    /// Planet time at the wall time `wall_ms`, moving the planet's clock
    /// there (see `clock`). A planet from before the clock starts it where
    /// its own time was (the old test offset included).
    pub fn time_at(&mut self, wall_ms: i64) -> i64 {
        if self.clock == crate::clock::Clock::default() {
            self.clock = crate::clock::Clock::starting(wall_ms + self.clock_offset_ms);
            self.clock.wall_ms = wall_ms;
            self.clock_offset_ms = 0;
        }
        self.clock.tick(wall_ms)
    }

    /// Someone is playing at wall time `wall_ms`: the planet runs at play
    /// speed for a while, and playing counts as acting (the planet never
    /// slows down for idleness while someone watches it).
    pub fn present(&mut self, wall_ms: i64) {
        self.time_at(wall_ms);
        self.clock.present(wall_ms);
        let step = step_of(self.clock.planet_ms());
        self.last_action_step = self.last_action_step.max(step);
    }

    pub fn amount(&self, resource: &str) -> f64 {
        self.resources.get(resource).copied().unwrap_or(0.0)
    }

    pub fn add(&mut self, resource: &str, amount: f64) {
        let value = self.resources.entry(resource.to_string()).or_insert(0.0);
        *value = (*value + amount).max(0.0);
    }

    /// Buildings standing on the planet, by id (one per tile).
    pub fn built(&self) -> impl Iterator<Item = &str> {
        self.tiles.iter().filter_map(|t| t.building.as_deref())
    }

    pub fn has_built(&self, building: &str) -> bool {
        self.built().any(|b| b == building)
    }

    pub fn log(&mut self, catalog: &Catalog, step: i64, key: &str, params: &[(&str, &str)]) {
        self.chronicle.push(Entry {
            step,
            key: key.to_string(),
            params: params
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        });
        let max = catalog.tuning.chronicle_len;
        if self.chronicle.len() > max {
            let extra = self.chronicle.len() - max;
            self.chronicle.drain(..extra);
        }
    }
}
