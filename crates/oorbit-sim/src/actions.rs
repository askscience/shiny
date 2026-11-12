//! What the player can do. Every action happens at a time: the planet is
//! advanced to it first, then the action is checked and applied.

use serde::{Deserialize, Serialize};

use crate::catalog::Catalog;
use crate::planet::{Construction, Planet, Priorities};
use crate::step::{advance, check_era, free_tiles, resolve};
use crate::{hours_to_steps, step_of};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    SetPriorities { priorities: Priorities },
    Research { tech: String },
    Build { building: String, tile: usize },
    ResolveEvent { choice: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionError {
    InvalidPriorities,
    UnknownTech(String),
    AlreadyKnown(String),
    TechLocked(String),
    UnknownBuilding(String),
    BuildingLocked(String),
    MonumentDone(String),
    BadTile(usize),
    NotAffordable(String),
    NoPendingEvent,
    UnknownChoice(String),
}

impl std::fmt::Display for ActionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ActionError::InvalidPriorities => {
                write!(f, "priorities must be 0 to 4 each, not all zero")
            }
            ActionError::UnknownTech(id) => write!(f, "unknown discovery `{id}`"),
            ActionError::AlreadyKnown(id) => write!(f, "`{id}` is already known"),
            ActionError::TechLocked(id) => write!(f, "`{id}` cannot be researched yet"),
            ActionError::UnknownBuilding(id) => write!(f, "unknown building `{id}`"),
            ActionError::BuildingLocked(id) => write!(f, "`{id}` cannot be built yet"),
            ActionError::MonumentDone(id) => write!(f, "`{id}` already stands or is being built"),
            ActionError::BadTile(i) => write!(f, "tile {i} cannot take this building"),
            ActionError::NotAffordable(id) => write!(f, "not enough resources for `{id}`"),
            ActionError::NoPendingEvent => write!(f, "no event is waiting for an answer"),
            ActionError::UnknownChoice(id) => write!(f, "unknown choice `{id}`"),
        }
    }
}

impl std::error::Error for ActionError {}

/// Can the planet research this discovery now?
pub fn can_research(planet: &Planet, catalog: &Catalog, id: &str) -> Result<(), ActionError> {
    let tech = catalog
        .tech(id)
        .ok_or_else(|| ActionError::UnknownTech(id.into()))?;
    if planet.known.contains(id) {
        return Err(ActionError::AlreadyKnown(id.into()));
    }
    if tech.era > planet.era || !tech.requires.iter().all(|r| planet.known.contains(r)) {
        return Err(ActionError::TechLocked(id.into()));
    }
    Ok(())
}

/// Can the planet start this building now (anywhere)?
pub fn can_build(planet: &Planet, catalog: &Catalog, id: &str) -> Result<(), ActionError> {
    let b = catalog
        .building(id)
        .ok_or_else(|| ActionError::UnknownBuilding(id.into()))?;
    if b.era > planet.era
        || b.requires
            .as_ref()
            .is_some_and(|t| !planet.known.contains(t))
    {
        return Err(ActionError::BuildingLocked(id.into()));
    }
    if b.monument {
        if b.era != planet.era {
            return Err(ActionError::BuildingLocked(id.into()));
        }
        if planet.has_built(id) || planet.construction.iter().any(|c| c.building == id) {
            return Err(ActionError::MonumentDone(id.into()));
        }
    }
    if !b.cost.iter().all(|(r, need)| planet.amount(r) >= *need) {
        return Err(ActionError::NotAffordable(id.into()));
    }
    Ok(())
}

/// Apply one action at `now_ms`.
pub fn apply(
    planet: &mut Planet,
    catalog: &Catalog,
    now_ms: i64,
    action: &Action,
) -> Result<(), ActionError> {
    advance(planet, catalog, now_ms);
    let step = step_of(now_ms).max(planet.step);
    match action {
        Action::SetPriorities { priorities } => {
            let valid = priorities.total() > 0
                && [
                    priorities.food,
                    priorities.materials,
                    priorities.knowledge,
                    priorities.culture,
                ]
                .iter()
                .all(|&p| p <= Priorities::MAX);
            if !valid {
                return Err(ActionError::InvalidPriorities);
            }
            planet.priorities = *priorities;
        }
        Action::Research { tech } => {
            can_research(planet, catalog, tech)?;
            planet.research = Some(tech.clone());
            planet.log(catalog, step, "research_started", &[("tech", tech)]);
        }
        Action::Build { building, tile } => {
            can_build(planet, catalog, building)?;
            let def = catalog.building(building).expect("checked by can_build");
            if !free_tiles(planet, &def.biomes).contains(tile) {
                return Err(ActionError::BadTile(*tile));
            }
            for (r, need) in &def.cost {
                planet.add(r, -need);
            }
            planet.construction.push(Construction {
                building: building.clone(),
                tile: *tile,
                done_step: step + hours_to_steps(def.build_hours),
            });
            let t = tile.to_string();
            planet.log(
                catalog,
                step,
                "building_started",
                &[("building", building), ("tile", &t)],
            );
        }
        Action::ResolveEvent { choice } => {
            let pending = planet
                .pending_event
                .clone()
                .ok_or(ActionError::NoPendingEvent)?;
            let event = catalog
                .event(&pending.id)
                .ok_or(ActionError::NoPendingEvent)?;
            if !event.choices.iter().any(|c| &c.id == choice) {
                return Err(ActionError::UnknownChoice(choice.clone()));
            }
            planet.pending_event = None;
            resolve(planet, catalog, step, &pending.id, choice);
            planet.log(
                catalog,
                step,
                "event_answered",
                &[("event", &pending.id), ("choice", choice)],
            );
        }
    }
    planet.last_action_step = step;
    check_era(planet, catalog, step);
    Ok(())
}
