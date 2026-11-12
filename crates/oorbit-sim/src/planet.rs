//! The state of one planet. It is plain data (serialized as JSON by the
//! plugin) and changes only through [`crate::advance`] and [`crate::apply`].

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::catalog::{Biome, Catalog, Category, ClimateDef};
use crate::rng::Rng;
use crate::step_of;

/// Version of the serialized state, for migrations of saved planets.
pub const STATE_VERSION: u32 = 2;

/// Draw `count` tiles from a climate's biome weights. With `guarantee`,
/// every planet stays playable whatever the dice say: at least one tile in
/// eight each of plains, forest and mountain (the biomes the early eras build
/// on).
fn draw_tiles(rng: &mut Rng, climate: &ClimateDef, count: usize, guarantee: bool) -> Vec<Tile> {
    let biomes: Vec<Biome> = climate.biomes.keys().copied().collect();
    let weights: Vec<u32> = climate.biomes.values().copied().collect();
    let mut tiles: Vec<Tile> = (0..count)
        .map(|_| Tile {
            biome: biomes[rng.weighted(&weights).unwrap_or(0)],
            building: None,
        })
        .collect();
    if !guarantee || count == 0 {
        return tiles;
    }
    const GUARANTEED: [Biome; 3] = [Biome::Plains, Biome::Forest, Biome::Mountain];
    let min_each = count.div_ceil(8);
    let tally = |tiles: &[Tile], b: Biome| tiles.iter().filter(|t| t.biome == b).count();
    let mut next = 0;
    for biome in GUARANTEED {
        while tally(&tiles, biome) < min_each {
            // Take a tile no guarantee depends on.
            let i = next % tiles.len();
            next += 1;
            let current = tiles[i].biome;
            if current == biome
                || (GUARANTEED.contains(&current) && tally(&tiles, current) <= min_each)
            {
                continue;
            }
            tiles[i].biome = biome;
        }
    }
    tiles
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tile {
    pub biome: Biome,
    #[serde(default)]
    pub building: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Construction {
    pub building: String,
    pub tile: usize,
    /// The step at which it stands.
    pub done_step: i64,
}

/// How much of the workforce goes to each category, 0 to 4 each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Priorities {
    pub food: u8,
    pub materials: u8,
    pub knowledge: u8,
    pub culture: u8,
}

impl Default for Priorities {
    fn default() -> Self {
        Priorities {
            food: 3,
            materials: 2,
            knowledge: 2,
            culture: 1,
        }
    }
}

impl Priorities {
    pub const MAX: u8 = 4;

    pub fn get(&self, category: Category) -> u8 {
        match category {
            Category::Food => self.food,
            Category::Materials => self.materials,
            Category::Knowledge => self.knowledge,
            Category::Culture => self.culture,
        }
    }

    pub fn total(&self) -> u32 {
        Category::ALL.iter().map(|&c| self.get(c) as u32).sum()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingEvent {
    pub id: String,
    pub step: i64,
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
    pub construction: Vec<Construction>,
    pub known: BTreeSet<String>,
    pub research: Option<String>,
    pub priorities: Priorities,
    pub pending_event: Option<PendingEvent>,
    pub last_event_step: i64,
    pub traits: BTreeMap<String, i32>,
    pub notables: Vec<Notable>,
    pub chronicle: Vec<Entry>,
    /// Highest population milestone already announced (multiples of 10).
    pub population_milestone: u32,
    /// True while food ran out on the last step.
    pub starving: bool,
}

impl Planet {
    /// A new planet founded at `now_ms`. `climate` must be a catalog climate.
    pub fn new(seed: u64, climate: &str, now_ms: i64, catalog: &Catalog) -> Result<Planet, String> {
        let climate_def = catalog
            .climate(climate)
            .ok_or_else(|| format!("unknown climate `{climate}`"))?;
        let mut rng = Rng::new(seed ^ 0x0A0B_C0DE);
        let tiles = draw_tiles(&mut rng, climate_def, catalog.tuning.tiles, true);

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
            construction: Vec::new(),
            known: BTreeSet::new(),
            research: None,
            priorities: Priorities::default(),
            pending_event: None,
            last_event_step: step,
            traits: BTreeMap::new(),
            notables: Vec::new(),
            chronicle: Vec::new(),
            population_milestone: 0,
            starving: false,
        };
        for r in &catalog.resources {
            planet.resources.insert(r.id.clone(), 0.0);
        }
        for (id, amount) in &catalog.tuning.start_resources {
            planet.resources.insert(id.clone(), *amount);
        }
        planet.log(catalog, step, "founded", &[("climate", climate)]);
        Ok(planet)
    }

    /// Bring a saved planet up to the catalog's tile count: a planet saved
    /// with fewer tiles (version 1 had 32) gains new land drawn from its
    /// climate, and keeps every tile, building and construction it had.
    pub fn fit_tiles(&mut self, catalog: &Catalog) {
        let want = catalog.tuning.tiles;
        if self.tiles.len() < want {
            if let Some(climate) = catalog.climate(&self.climate) {
                let mut rng = Rng::new(self.seed ^ 0x0071_11E5 ^ self.tiles.len() as u64);
                let extra = draw_tiles(&mut rng, climate, want - self.tiles.len(), false);
                self.tiles.extend(extra);
            }
        }
        self.version = STATE_VERSION;
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
