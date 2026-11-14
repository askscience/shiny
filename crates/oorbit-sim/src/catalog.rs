//! The game's data: eras, resources, buildings, discoveries, events and
//! climates. It lives in `data/catalog.toml` so the game can be balanced
//! without touching the rules. Names here are English fallbacks; the window
//! translates by id.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// What a worker can be busy with. The player sets a priority per category
/// and the population distributes itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Food,
    Materials,
    Knowledge,
    Culture,
}

impl Category {
    pub const ALL: [Category; 4] = [
        Category::Food,
        Category::Materials,
        Category::Knowledge,
        Category::Culture,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Category::Food => "food",
            Category::Materials => "materials",
            Category::Knowledge => "knowledge",
            Category::Culture => "culture",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Biome {
    Coast,
    Plains,
    Forest,
    Mountain,
    Desert,
    Ice,
    /// Open sea: nothing is built on it (ports stand on the coast).
    Ocean,
}

impl Biome {
    /// The land biomes, the ones a climate weighs.
    pub const LAND: [Biome; 6] = [
        Biome::Coast,
        Biome::Plains,
        Biome::Forest,
        Biome::Mountain,
        Biome::Desert,
        Biome::Ice,
    ];
}

/// A lasting effect of a building, a discovery or a notable person.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Effect {
    /// Room for more people.
    Housing { amount: f64 },
    /// `+amount` (0.25 = +25%) to a category's output.
    Bonus { category: Category, amount: f64 },
    /// Raises the storage cap of food and materials.
    Storage { amount: f64 },
    /// Turns inputs into an output every hour while the inputs last.
    Convert {
        inputs: BTreeMap<String, f64>,
        output: String,
        per_hour: f64,
    },
    /// A steady push on the people's mood (0.05 = +5 points).
    Mood { amount: f64 },
}

/// A one-off consequence of an event choice.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Outcome {
    Resource {
        id: String,
        amount: f64,
    },
    /// Immediate mood change, in mood units (0.1 = 10 points).
    Mood {
        amount: f64,
    },
    Population {
        amount: f64,
    },
    Trait {
        id: String,
        amount: i32,
    },
    /// An early discovery.
    Grant {
        tech: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Era {
    pub id: String,
    pub name: String,
    /// Discoveries needed to leave this era.
    pub requires: Vec<String>,
    /// The building that marks the era; it must stand to leave it.
    pub monument: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceDef {
    pub id: String,
    pub name: String,
    pub category: Category,
    /// For a material: the biome whose tiles yield it.
    #[serde(default)]
    pub biome: Option<Biome>,
    /// For a material: the discovery that lets people gather it.
    #[serde(default)]
    pub unlocked_by: Option<String>,
    /// For a material: the building it is extracted from (one share per
    /// building instead of one per tile).
    #[serde(default)]
    pub from_building: Option<String>,
    /// Stored without the storage cap (knowledge, culture).
    #[serde(default)]
    pub uncapped: bool,
    /// Dug out of the ground: gathering it empties the cell's deposit,
    /// which never grows back.
    #[serde(default)]
    pub deposit: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildingDef {
    pub id: String,
    pub name: String,
    pub era: u32,
    #[serde(default)]
    pub requires: Option<String>,
    pub cost: BTreeMap<String, f64>,
    pub build_hours: f64,
    /// Where it can stand; empty means any tile.
    #[serde(default)]
    pub biomes: Vec<Biome>,
    #[serde(default)]
    pub effects: Vec<Effect>,
    #[serde(default)]
    pub monument: bool,
    /// Founds a new city where it stands (it becomes the city's centre).
    #[serde(default)]
    pub city: bool,
    /// A landing for boats: two cities with one each can open a sea route.
    #[serde(default)]
    pub port: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TechDef {
    pub id: String,
    pub name: String,
    pub era: u32,
    /// Knowledge needed.
    pub cost: f64,
    #[serde(default)]
    pub requires: Vec<String>,
    #[serde(default)]
    pub effects: Vec<Effect>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Choice {
    pub id: String,
    #[serde(default)]
    pub outcomes: Vec<Outcome>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventDef {
    pub id: String,
    pub era_min: u32,
    #[serde(default)]
    pub era_max: Option<u32>,
    pub weight: u32,
    /// Only while this discovery is still unknown.
    #[serde(default)]
    pub unless_known: Option<String>,
    /// Only once this discovery is known.
    #[serde(default)]
    pub if_known: Option<String>,
    /// The first choice is the prudent one, taken when nobody answers.
    pub choices: Vec<Choice>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClimateDef {
    pub id: String,
    pub name: String,
    /// How the land splits between biomes (weights).
    pub biomes: BTreeMap<Biome, u32>,
    /// Multiplies food output.
    pub food: f64,
    /// Share of the planet under the sea.
    #[serde(default = "default_ocean")]
    pub ocean: f64,
}

fn default_ocean() -> f64 {
    0.55
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tuning {
    pub tiles: usize,
    pub start_population: f64,
    pub start_resources: BTreeMap<String, f64>,
    pub min_population: f64,
    pub base_housing: f64,
    pub base_storage: f64,
    pub worker_share: f64,
    pub food_per_worker_hour: f64,
    pub materials_per_worker_hour: f64,
    pub knowledge_per_worker_hour: f64,
    pub culture_per_worker_hour: f64,
    pub food_per_person_hour: f64,
    pub growth_per_hour: f64,
    pub starvation_per_hour: f64,
    pub emigration_per_hour: f64,
    pub mood_approach_per_hour: f64,
    pub culture_for_full_mood: f64,
    pub idle_after_hours: f64,
    pub idle_factor: f64,
    /// Settlers' zeal: a new planet works `1 + zeal_bonus` times faster at
    /// first, fading with a half-life-like `zeal_hours` (e^-age/zeal_hours),
    /// so the first session already sees discoveries and buildings.
    #[serde(default)]
    pub zeal_bonus: f64,
    #[serde(default = "default_zeal_hours")]
    pub zeal_hours: f64,
    pub season_days: f64,
    pub seasons: [f64; 4],
    pub event_gap_hours: f64,
    pub event_chance_per_hour: f64,
    pub event_expires_hours: f64,
    pub notable_chance_per_hour: f64,
    pub notable_days: f64,
    pub notable_bonus: f64,
    pub max_notables: usize,
    pub chronicle_len: usize,
    /// Hours a ship takes, by the sender's era (one entry per era).
    #[serde(default)]
    pub travel_hours: Vec<f64>,
    /// How much a ship carries, by the sender's era.
    #[serde(default)]
    pub ship_capacity: Vec<f64>,
    /// How often a neighbour sends something of its own.
    #[serde(default)]
    pub neighbor_act_per_hour: f64,
}

fn default_zeal_hours() -> f64 {
    8.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Catalog {
    pub tuning: Tuning,
    pub eras: Vec<Era>,
    pub resources: Vec<ResourceDef>,
    pub buildings: Vec<BuildingDef>,
    pub techs: Vec<TechDef>,
    pub events: Vec<EventDef>,
    pub climates: Vec<ClimateDef>,
}

/// The catalog shipped with the game.
pub const DEFAULT_CATALOG: &str = include_str!("../data/catalog.toml");

impl Catalog {
    /// Parse and check a catalog: every id it refers to must exist.
    pub fn parse(text: &str) -> Result<Catalog, String> {
        let catalog: Catalog = toml::from_str(text).map_err(|e| e.to_string())?;
        catalog.validate()?;
        Ok(catalog)
    }

    /// The shipped catalog. It is checked by the tests, so this never fails
    /// at run time.
    pub fn builtin() -> Catalog {
        Catalog::parse(DEFAULT_CATALOG).expect("the built-in catalog is valid")
    }

    pub fn era(&self, number: u32) -> Option<&Era> {
        number
            .checked_sub(1)
            .and_then(|i| self.eras.get(i as usize))
    }

    pub fn building(&self, id: &str) -> Option<&BuildingDef> {
        self.buildings.iter().find(|b| b.id == id)
    }

    pub fn tech(&self, id: &str) -> Option<&TechDef> {
        self.techs.iter().find(|t| t.id == id)
    }

    pub fn resource(&self, id: &str) -> Option<&ResourceDef> {
        self.resources.iter().find(|r| r.id == id)
    }

    pub fn event(&self, id: &str) -> Option<&EventDef> {
        self.events.iter().find(|e| e.id == id)
    }

    /// The era a resource first matters in: the earliest of the discovery
    /// that unlocks it, the buildings that produce it and the buildings that
    /// cost it. Basic resources (food, wood, stone) are era 1.
    pub fn resource_era(&self, id: &str) -> u32 {
        let mut era = u32::MAX;
        if let Some(r) = self.resource(id) {
            if let Some(t) = r.unlocked_by.as_ref().and_then(|t| self.tech(t)) {
                era = era.min(t.era);
            }
            if let Some(b) = r.from_building.as_ref().and_then(|b| self.building(b)) {
                era = era.min(b.era);
            }
        }
        for b in &self.buildings {
            let produces = b
                .effects
                .iter()
                .any(|e| matches!(e, Effect::Convert { output, .. } if output == id));
            if produces || b.cost.contains_key(id) {
                era = era.min(b.era);
            }
        }
        if era == u32::MAX {
            1
        } else {
            era
        }
    }

    pub fn climate(&self, id: &str) -> Option<&ClimateDef> {
        self.climates.iter().find(|c| c.id == id)
    }

    fn validate(&self) -> Result<(), String> {
        let unique = |kind: &str, ids: Vec<&str>| -> Result<BTreeSet<String>, String> {
            let mut seen = BTreeSet::new();
            for id in ids {
                if !seen.insert(id.to_string()) {
                    return Err(format!("duplicate {kind} id `{id}`"));
                }
            }
            Ok(seen)
        };
        let resources = unique(
            "resource",
            self.resources.iter().map(|r| r.id.as_str()).collect(),
        )?;
        let buildings = unique(
            "building",
            self.buildings.iter().map(|b| b.id.as_str()).collect(),
        )?;
        let techs = unique("tech", self.techs.iter().map(|t| t.id.as_str()).collect())?;
        unique("event", self.events.iter().map(|e| e.id.as_str()).collect())?;
        unique(
            "climate",
            self.climates.iter().map(|c| c.id.as_str()).collect(),
        )?;

        let resource = |id: &str, owner: &str| {
            if resources.contains(id) {
                Ok(())
            } else {
                Err(format!("{owner}: unknown resource `{id}`"))
            }
        };
        let tech = |id: &str, owner: &str| {
            if techs.contains(id) {
                Ok(())
            } else {
                Err(format!("{owner}: unknown tech `{id}`"))
            }
        };
        let effects = |list: &[Effect], owner: &str| -> Result<(), String> {
            for effect in list {
                if let Effect::Convert { inputs, output, .. } = effect {
                    resource(output, owner)?;
                    for id in inputs.keys() {
                        resource(id, owner)?;
                    }
                }
            }
            Ok(())
        };

        if self.eras.is_empty() {
            return Err("no eras".into());
        }
        if self.climates.is_empty() {
            return Err("no climates".into());
        }
        for (i, era) in self.eras.iter().enumerate() {
            let owner = format!("era `{}`", era.id);
            for id in &era.requires {
                tech(id, &owner)?;
            }
            let monument = self
                .building(&era.monument)
                .ok_or_else(|| format!("{owner}: unknown monument `{}`", era.monument))?;
            if !monument.monument || monument.era != i as u32 + 1 {
                return Err(format!(
                    "{owner}: `{}` is not this era's monument",
                    era.monument
                ));
            }
        }
        for r in &self.resources {
            let owner = format!("resource `{}`", r.id);
            if let Some(id) = &r.unlocked_by {
                tech(id, &owner)?;
            }
            if let Some(id) = &r.from_building {
                if !buildings.contains(id) {
                    return Err(format!("{owner}: unknown building `{id}`"));
                }
            }
        }
        for b in &self.buildings {
            let owner = format!("building `{}`", b.id);
            if let Some(id) = &b.requires {
                tech(id, &owner)?;
            }
            for id in b.cost.keys() {
                resource(id, &owner)?;
            }
            effects(&b.effects, &owner)?;
        }
        for t in &self.techs {
            let owner = format!("tech `{}`", t.id);
            for id in &t.requires {
                tech(id, &owner)?;
            }
            effects(&t.effects, &owner)?;
        }
        for e in &self.events {
            let owner = format!("event `{}`", e.id);
            if e.choices.is_empty() {
                return Err(format!("{owner}: no choices"));
            }
            for id in e.unless_known.iter().chain(e.if_known.iter()) {
                tech(id, &owner)?;
            }
            for choice in &e.choices {
                for outcome in &choice.outcomes {
                    match outcome {
                        Outcome::Resource { id, .. } => resource(id, &owner)?,
                        Outcome::Grant { tech: id } => tech(id, &owner)?,
                        _ => {}
                    }
                }
            }
        }
        for id in self.tuning.start_resources.keys() {
            resource(id, "tuning.start_resources")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_builtin_catalog_is_valid() {
        let catalog = Catalog::builtin();
        assert!(catalog.eras.len() >= 3);
    }

    #[test]
    fn a_dangling_reference_is_rejected() {
        let broken = DEFAULT_CATALOG.replacen("requires = [\"fire\"", "requires = [\"plasma\"", 1);
        assert_ne!(
            broken, DEFAULT_CATALOG,
            "the test needs a reference to break"
        );
        let error = Catalog::parse(&broken).unwrap_err();
        assert!(error.contains("plasma"), "{error}");
    }
}
