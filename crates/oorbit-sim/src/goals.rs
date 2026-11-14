//! Small goals that guide the player: three at a time from the catalog's
//! list (`[[goals]]`, in order, up to the current era), each with a little
//! reward when reached. They always leave something to do next.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::catalog::Catalog;
use crate::planet::Planet;

/// Goals shown at once.
pub const ACTIVE: usize = 3;

/// What a goal asks for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Goal {
    /// Know this discovery.
    Tech { tech: String },
    /// Have `count` of this building standing.
    Build { building: String, count: usize },
    /// Have `amount` of a resource in store.
    Have { resource: String, amount: f64 },
    /// Have a crew gathering this resource.
    Order { resource: String },
    /// Reach this many people.
    Population { count: f64 },
    /// Have this many cities.
    Cities { count: usize },
    /// Have this many colonies linked to the capital.
    Linked { count: usize },
    /// Have brought home this many finds.
    Finds { count: u32 },
    /// Have planted trees this many times.
    Planted { count: u32 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalDef {
    pub id: String,
    pub era: u32,
    #[serde(flatten)]
    pub goal: Goal,
    #[serde(default)]
    pub reward: BTreeMap<String, f64>,
}

/// How far along a goal is: (have, need).
pub fn progress(planet: &Planet, goal: &Goal) -> (f64, f64) {
    match goal {
        Goal::Tech { tech } => (planet.known.contains(tech) as u8 as f64, 1.0),
        Goal::Build { building, count } => (
            planet.built().filter(|b| b == building).count() as f64,
            *count as f64,
        ),
        Goal::Have { resource, amount } => (planet.amount(resource).min(*amount), *amount),
        Goal::Order { resource } => (
            planet.orders.iter().any(|o| &o.resource == resource) as u8 as f64,
            1.0,
        ),
        Goal::Population { count } => (planet.population.min(*count).floor(), *count),
        Goal::Cities { count } => (planet.cities.len() as f64, *count as f64),
        Goal::Linked { count } => (
            crate::cities::connected(planet).iter().skip(1).filter(|c| **c).count() as f64,
            *count as f64,
        ),
        Goal::Finds { count } => (planet.finds_collected as f64, *count as f64),
        Goal::Planted { count } => (planet.trees_planted as f64, *count as f64),
    }
}

/// The goals the player works on now.
pub fn active<'a>(planet: &Planet, catalog: &'a Catalog) -> Vec<&'a GoalDef> {
    catalog
        .goals
        .iter()
        .filter(|g| g.era <= planet.era && !planet.goals_done.contains(&g.id))
        .take(ACTIVE)
        .collect()
}

/// Reward every active goal that is reached (repeatedly, so reaching one
/// can bring the next into view).
pub fn run_goals(planet: &mut Planet, catalog: &Catalog, step: i64) {
    loop {
        let reached: Vec<GoalDef> = active(planet, catalog)
            .into_iter()
            .filter(|g| {
                let (have, need) = progress(planet, &g.goal);
                have >= need
            })
            .cloned()
            .collect();
        if reached.is_empty() {
            return;
        }
        for g in reached {
            planet.goals_done.insert(g.id.clone());
            for (r, a) in &g.reward {
                planet.add(r, *a);
            }
            planet.log(catalog, step, "goal_done", &[("goal", &g.id)]);
        }
    }
}
