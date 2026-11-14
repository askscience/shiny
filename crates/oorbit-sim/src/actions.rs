//! What the player can do. Every action happens at a time: the planet is
//! advanced to it first, then the action is checked and applied.

use serde::{Deserialize, Serialize};

use crate::catalog::Catalog;
use crate::neighbors::{capacity, launch, Ship, ShipKind};
use crate::planet::{Construction, Order, Planet};
use crate::step::{advance, check_era, resolve};
use crate::{hours_to_steps, step_of};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    /// Send a crew to gather `resource` on `tile`.
    Harvest {
        tile: usize,
        resource: String,
    },
    /// Call the crew on `tile` back (they go back to thinking).
    StopHarvest {
        tile: usize,
    },
    Research {
        tech: String,
    },
    Build {
        building: String,
        tile: usize,
    },
    ResolveEvent {
        choice: String,
    },
    /// Send a ship to a neighbour: resources (`gift`, `prank` aims at one),
    /// a discovery (`idea`), one of the people (`visit`) or a `message`.
    Send {
        to: String,
        kind: ShipKind,
        #[serde(default)]
        resource: Option<String>,
        #[serde(default)]
        amount: f64,
    },
    /// Clear a tile for something new. Nothing is refunded; monuments stay.
    Demolish {
        tile: usize,
    },
    /// Open a road or a sea route between two cities (indices into
    /// `cities`).
    Link {
        kind: crate::cities::LinkKind,
        from: usize,
        to: usize,
    },
    AcceptOffer {
        id: u64,
    },
    DeclineOffer {
        id: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionError {
    CannotHarvest(usize),
    NoCrew(usize),
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
    UnknownNeighbor(String),
    ShipLocked(String),
    BadCargo,
    NoOffer(u64),
    CannotDemolish(usize),
    CannotLink,
}

impl std::fmt::Display for ActionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ActionError::CannotHarvest(i) => {
                write!(f, "no crew can gather that on tile {i} (too far, nothing there, or every crew is busy)")
            }
            ActionError::NoCrew(i) => write!(f, "nobody works tile {i}"),
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
            ActionError::UnknownNeighbor(id) => write!(f, "no neighbour `{id}`"),
            ActionError::ShipLocked(kind) => write!(f, "`{kind}` ships cannot be sent yet"),
            ActionError::BadCargo => write!(f, "that cargo cannot be sent"),
            ActionError::NoOffer(id) => write!(f, "no offer {id}"),
            ActionError::CannotLink => {
                write!(f, "no way can be opened between those cities (no path, already linked, or too costly)")
            }
            ActionError::CannotDemolish(i) => {
                write!(f, "tile {i} has nothing that can be demolished")
            }
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
        Action::Harvest { tile, resource } => {
            if !crate::orders::can_order(planet, catalog, *tile, resource) {
                return Err(ActionError::CannotHarvest(*tile));
            }
            planet.orders.push(Order {
                tile: *tile,
                resource: resource.clone(),
            });
        }
        Action::StopHarvest { tile } => {
            if !planet.orders.iter().any(|o| o.tile == *tile) {
                return Err(ActionError::NoCrew(*tile));
            }
            planet.orders.retain(|o| o.tile != *tile);
        }
        Action::Research { tech } => {
            can_research(planet, catalog, tech)?;
            planet.research = Some(tech.clone());
            planet.log(catalog, step, "research_started", &[("tech", tech)]);
        }
        Action::Build { building, tile } => {
            can_build(planet, catalog, building)?;
            let def = catalog.building(building).expect("checked by can_build");
            if !crate::cities::sites_for(planet, catalog, def).contains(tile) {
                return Err(ActionError::BadTile(*tile));
            }
            for (r, need) in &def.cost {
                planet.add(r, -need);
            }
            planet.construction.push(Construction {
                building: building.clone(),
                tile: *tile,
                start_step: step,
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
        Action::Send {
            to,
            kind,
            resource,
            amount,
        } => {
            send(
                planet,
                catalog,
                step,
                to,
                *kind,
                resource.as_deref(),
                *amount,
            )?;
        }
        Action::Demolish { tile } => {
            let building = planet
                .tiles
                .get(*tile)
                .and_then(|t| t.building.clone())
                .ok_or(ActionError::CannotDemolish(*tile))?;
            if catalog.building(&building).is_some_and(|b| b.monument || b.city) {
                return Err(ActionError::CannotDemolish(*tile));
            }
            planet.tiles[*tile].building = None;
            let t = tile.to_string();
            planet.log(
                catalog,
                step,
                "building_demolished",
                &[("building", &building), ("tile", &t)],
            );
        }
        Action::Link { kind, from, to } => {
            let route = crate::cities::route(planet, catalog, *kind, *from, *to)
                .ok_or(ActionError::CannotLink)?;
            if !route.cost.iter().all(|(r, need)| planet.amount(r) >= *need) {
                return Err(ActionError::CannotLink);
            }
            crate::cities::open(planet, catalog, step, route);
        }
        Action::AcceptOffer { id } => {
            let offer = planet
                .offers
                .iter()
                .find(|o| o.id == *id)
                .cloned()
                .ok_or(ActionError::NoOffer(*id))?;
            if planet.amount(&offer.want_resource) < offer.want_amount {
                return Err(ActionError::NotAffordable(offer.want_resource.clone()));
            }
            planet.offers.retain(|o| o.id != *id);
            planet.add(&offer.want_resource, -offer.want_amount);
            let era = planet
                .neighbors
                .iter()
                .find(|n| n.id == offer.from)
                .map(|n| n.era(planet, catalog))
                .unwrap_or(planet.era);
            if let Some(n) = planet.neighbors.iter_mut().find(|n| n.id == offer.from) {
                n.friendship += 1;
            }
            let delivery = Ship {
                id: 0,
                from: offer.from.clone(),
                to: "home".into(),
                kind: ShipKind::Trade,
                resource: Some(offer.give_resource.clone()),
                amount: offer.give_amount,
                tech: None,
                era,
                depart_step: 0,
                arrive_step: 0,
                returning: true,
            };
            launch(planet, catalog, step, delivery);
            planet.log(catalog, step, "offer_accepted", &[]);
        }
        Action::DeclineOffer { id } => {
            if !planet.offers.iter().any(|o| o.id == *id) {
                return Err(ActionError::NoOffer(*id));
            }
            planet.offers.retain(|o| o.id != *id);
        }
    }
    planet.last_action_step = step;
    check_era(planet, catalog, step);
    Ok(())
}

/// Can the planet send this kind of ship now?
pub fn can_send(planet: &Planet, kind: ShipKind) -> bool {
    planet.era >= kind.first_era() && kind != ShipKind::Trade
}

fn send(
    planet: &mut Planet,
    catalog: &Catalog,
    step: i64,
    to: &str,
    kind: ShipKind,
    resource: Option<&str>,
    amount: f64,
) -> Result<(), ActionError> {
    if !planet.neighbors.iter().any(|n| n.id == to) {
        return Err(ActionError::UnknownNeighbor(to.into()));
    }
    if !can_send(planet, kind) {
        return Err(ActionError::ShipLocked(kind.as_str().into()));
    }
    let mut ship = Ship {
        id: 0,
        from: "home".into(),
        to: to.into(),
        kind,
        resource: None,
        amount: 0.0,
        tech: None,
        era: planet.era,
        depart_step: 0,
        arrive_step: 0,
        returning: false,
    };
    match kind {
        ShipKind::Gift => {
            let resource = resource.ok_or(ActionError::BadCargo)?;
            let amount = amount.floor();
            let usable = crate::neighbors::usable_resources(planet, catalog);
            if !usable.iter().any(|r| r == resource)
                || amount < 1.0
                || amount > capacity(catalog, planet.era)
                || planet.amount(resource) < amount
            {
                return Err(ActionError::BadCargo);
            }
            planet.add(resource, -amount);
            ship.resource = Some(resource.into());
            ship.amount = amount;
        }
        ShipKind::Prank => {
            ship.resource = resource.map(String::from);
        }
        ShipKind::Idea => {
            // Ideas are shared knowledge: they cost a little of it.
            let cost = 10.0 * planet.era as f64;
            if planet.amount("knowledge") < cost {
                return Err(ActionError::NotAffordable("knowledge".into()));
            }
            planet.add("knowledge", -cost);
        }
        ShipKind::Visit | ShipKind::Message => {}
        ShipKind::Trade => return Err(ActionError::ShipLocked("trade".into())),
    }
    let name = planet
        .neighbors
        .iter()
        .find(|n| n.id == to)
        .map(|n| n.name.clone())
        .unwrap_or_default();
    launch(planet, catalog, step, ship);
    planet.log(
        catalog,
        step,
        "ship_sent",
        &[("to", &name), ("kind", kind.as_str())],
    );
    Ok(())
}
