//! The planets next door. Until friends' planets travel over the network,
//! every planet has three simulated neighbours, each with a personality,
//! who send ships (gifts, ideas, visitors, pranks, trade offers, messages)
//! and answer the ships the player sends. Ships take real hours to arrive,
//! fewer in later eras, and carry more.
//!
//! Like the rest of the simulation this is deterministic: neighbours are
//! drawn from the planet seed, and every step makes the same number of draws.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::catalog::{Catalog, Category};
use crate::planet::Planet;
use crate::rng::Rng;
use crate::{hours_to_steps, STEP_HOURS};

/// What a ship carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShipKind {
    /// Resources, freely given.
    Gift,
    /// A discovery the receiver does not know yet.
    Idea,
    /// One of the people, who comes back with stories.
    Visit,
    /// A harmless mischief: some of a resource goes missing.
    Prank,
    /// A trade offer (from a neighbour), or what a trade delivers.
    Trade,
    /// A few words.
    Message,
}

impl ShipKind {
    pub const ALL: [ShipKind; 6] = [
        ShipKind::Gift,
        ShipKind::Idea,
        ShipKind::Visit,
        ShipKind::Prank,
        ShipKind::Trade,
        ShipKind::Message,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ShipKind::Gift => "gift",
            ShipKind::Idea => "idea",
            ShipKind::Visit => "visit",
            ShipKind::Prank => "prank",
            ShipKind::Trade => "trade",
            ShipKind::Message => "message",
        }
    }

    /// The first era that can send this kind of ship.
    pub fn first_era(self) -> u32 {
        match self {
            ShipKind::Gift | ShipKind::Message => 1,
            ShipKind::Prank | ShipKind::Trade => 2,
            ShipKind::Idea => 3,
            ShipKind::Visit => 4,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Neighbor {
    pub id: String,
    pub name: String,
    pub climate: String,
    /// generous, trader or prankster.
    pub persona: String,
    /// Its era is the player's plus this (-1, 0 or +1), within the eras.
    pub era_offset: i32,
    /// How the two planets get on: gifts raise it, pranks lower it.
    pub friendship: i32,
}

impl Neighbor {
    pub fn era(&self, planet: &Planet, catalog: &Catalog) -> u32 {
        (planet.era as i32 + self.era_offset).clamp(1, catalog.eras.len() as i32) as u32
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ship {
    pub id: u64,
    /// "home" or a neighbour id.
    pub from: String,
    pub to: String,
    pub kind: ShipKind,
    #[serde(default)]
    pub resource: Option<String>,
    #[serde(default)]
    pub amount: f64,
    #[serde(default)]
    pub tech: Option<String>,
    /// The era of the sender when it left: decides how the ship looks.
    pub era: u32,
    pub depart_step: i64,
    pub arrive_step: i64,
    /// A visitor coming back, a reply, a trade delivery.
    #[serde(default)]
    pub returning: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Offer {
    pub id: u64,
    pub from: String,
    pub give_resource: String,
    pub give_amount: f64,
    pub want_resource: String,
    pub want_amount: f64,
    pub expires_step: i64,
}

const PERSONAS: [&str; 3] = ["generous", "trader", "prankster"];
const SYLLABLES: [&str; 12] = [
    "ve", "lu", "ka", "ro", "mi", "sa", "to", "ri", "an", "zo", "el", "da",
];

/// Hours a ship takes from a sender in this era.
pub fn travel_hours(catalog: &Catalog, era: u32) -> f64 {
    let list = &catalog.tuning.travel_hours;
    list.get(era.saturating_sub(1) as usize)
        .or(list.last())
        .copied()
        .unwrap_or(6.0)
}

/// How much a ship from a sender in this era can carry.
pub fn capacity(catalog: &Catalog, era: u32) -> f64 {
    let list = &catalog.tuning.ship_capacity;
    list.get(era.saturating_sub(1) as usize)
        .or(list.last())
        .copied()
        .unwrap_or(10.0)
}

/// Give a planet its neighbours (once).
pub fn ensure_neighbors(planet: &mut Planet, catalog: &Catalog) {
    if !planet.neighbors.is_empty() {
        return;
    }
    let mut rng = Rng::new(planet.seed ^ 0x0E16_4B0F);
    let climates: Vec<&str> = catalog
        .climates
        .iter()
        .map(|c| c.id.as_str())
        .filter(|c| *c != planet.climate)
        .collect();
    for (i, persona) in PERSONAS.iter().enumerate() {
        let syllables = 2 + rng.below(2);
        let mut name: String = (0..syllables)
            .map(|_| SYLLABLES[rng.below(SYLLABLES.len())])
            .collect();
        if let Some(first) = name.get_mut(0..1) {
            first.make_ascii_uppercase();
        }
        let climate = climates
            .get(i % climates.len().max(1))
            .copied()
            .unwrap_or("temperate");
        planet.neighbors.push(Neighbor {
            id: format!("n{}", i + 1),
            name,
            climate: climate.to_string(),
            persona: persona.to_string(),
            era_offset: [1, 0, -1][i],
            friendship: 0,
        });
    }
}

/// Resources the player's planet can use now (for gifts and trades).
pub fn usable_resources(planet: &Planet, catalog: &Catalog) -> Vec<String> {
    catalog
        .resources
        .iter()
        .filter(|r| r.category == Category::Materials || r.category == Category::Food)
        .filter(|r| {
            catalog.resource_era(&r.id) <= planet.era
                && r.unlocked_by
                    .as_ref()
                    .is_none_or(|t| planet.known.contains(t))
        })
        .map(|r| r.id.clone())
        .collect()
}

fn next_id(planet: &mut Planet) -> u64 {
    planet.next_id += 1;
    planet.next_id
}

/// Launch a ship from `from` to `to` at `step`.
pub(crate) fn launch(planet: &mut Planet, catalog: &Catalog, step: i64, mut ship: Ship) {
    ship.id = next_id(planet);
    ship.depart_step = step;
    ship.arrive_step = step + hours_to_steps(travel_hours(catalog, ship.era));
    planet.ships.push(ship);
}

fn neighbor_name(planet: &Planet, id: &str) -> String {
    planet
        .neighbors
        .iter()
        .find(|n| n.id == id)
        .map(|n| n.name.clone())
        .unwrap_or_else(|| id.to_string())
}

/// One step of neighbourly life: ships arrive, offers expire, neighbours
/// decide to send something. Always the same number of draws.
pub(crate) fn run_neighbors(planet: &mut Planet, catalog: &Catalog, step: i64, rng: &mut Rng) {
    let draws: Vec<[f64; 4]> = (0..PERSONAS.len())
        .map(|_| [rng.unit(), rng.unit(), rng.unit(), rng.unit()])
        .collect();

    // Arrivals, in launch order.
    let mut arrived = Vec::new();
    planet.ships.retain(|s| {
        if s.arrive_step <= step {
            arrived.push(s.clone());
            false
        } else {
            true
        }
    });
    for (k, ship) in arrived.into_iter().enumerate() {
        let roll = draws.get(k % draws.len()).map(|d| d[3]).unwrap_or(0.5);
        if ship.to == "home" {
            arrive_home(planet, catalog, step, ship);
        } else {
            arrive_at_neighbor(planet, catalog, step, ship, roll);
        }
    }

    // Offers nobody answered.
    let expired: Vec<Offer> = planet
        .offers
        .iter()
        .filter(|o| o.expires_step <= step)
        .cloned()
        .collect();
    planet.offers.retain(|o| o.expires_step > step);
    for offer in expired {
        let name = neighbor_name(planet, &offer.from);
        planet.log(catalog, step, "offer_expired", &[("from", &name)]);
    }

    // Each neighbour, now and then, sends something of its own.
    let per_step = catalog.tuning.neighbor_act_per_hour * STEP_HOURS;
    for i in 0..planet.neighbors.len() {
        let d = draws[i % draws.len()];
        let id = planet.neighbors[i].id.clone();
        let busy =
            planet.ships.iter().any(|s| s.from == id) || planet.offers.iter().any(|o| o.from == id);
        if busy || d[0] >= per_step {
            continue;
        }
        neighbor_acts(planet, catalog, step, i, d[1], d[2]);
    }
}

/// What a neighbour sends on its own, by personality.
fn neighbor_acts(
    planet: &mut Planet,
    catalog: &Catalog,
    step: i64,
    index: usize,
    pick: f64,
    amount_roll: f64,
) {
    let neighbor = planet.neighbors[index].clone();
    let era = neighbor.era(planet, catalog);
    let weights: [u32; 6] = match neighbor.persona.as_str() {
        // gift, idea, visit, prank, trade, message
        "generous" => [5, 2, 2, 0, 1, 2],
        "trader" => [1, 1, 1, 0, 6, 1],
        _ => [1, 1, 1, 5, 1, 2],
    };
    let idea = idea_for(planet, catalog, era);
    let weights: Vec<u32> = ShipKind::ALL
        .iter()
        .zip(weights)
        .map(|(kind, w)| {
            let possible = era >= kind.first_era() && (*kind != ShipKind::Idea || idea.is_some());
            if possible {
                w
            } else {
                0
            }
        })
        .collect();
    let mut chooser = Rng::new((pick * u64::MAX as f64) as u64);
    let Some(k) = chooser.weighted(&weights) else {
        return;
    };
    let kind = ShipKind::ALL[k];
    let usable = usable_resources(planet, catalog);
    if usable.is_empty() {
        return;
    }
    let resource = usable[(amount_roll * usable.len() as f64) as usize % usable.len()].clone();
    let generosity = 1.0 + 0.1 * neighbor.friendship.clamp(-5, 10) as f64;
    let amount = (capacity(catalog, era) * (0.1 + 0.15 * amount_roll) * generosity)
        .round()
        .max(1.0);
    let ship = Ship {
        id: 0,
        from: neighbor.id.clone(),
        to: "home".into(),
        kind,
        resource: matches!(kind, ShipKind::Gift | ShipKind::Prank | ShipKind::Trade)
            .then_some(resource),
        amount,
        tech: if kind == ShipKind::Idea { idea } else { None },
        era,
        depart_step: 0,
        arrive_step: 0,
        returning: false,
    };
    launch(planet, catalog, step, ship);
}

/// A discovery a neighbour in `era` could teach: one the player can study
/// now, or of an era the player has reached.
fn idea_for(planet: &Planet, catalog: &Catalog, era: u32) -> Option<String> {
    catalog
        .techs
        .iter()
        .filter(|t| t.era <= era.min(planet.era) && !planet.known.contains(&t.id))
        .filter(|t| t.requires.iter().all(|r| planet.known.contains(r)))
        .map(|t| t.id.clone())
        .next()
}

fn arrive_home(planet: &mut Planet, catalog: &Catalog, step: i64, ship: Ship) {
    let from = neighbor_name(planet, &ship.from);
    let amount = ship.amount;
    match ship.kind {
        ShipKind::Gift => {
            let resource = ship.resource.clone().unwrap_or_default();
            planet.add(&resource, amount);
            let n = format!("{amount:.0}");
            planet.log(
                catalog,
                step,
                "ship_gift",
                &[("from", &from), ("resource", &resource), ("amount", &n)],
            );
        }
        ShipKind::Trade if ship.returning => {
            let resource = ship.resource.clone().unwrap_or_default();
            planet.add(&resource, amount);
            let n = format!("{amount:.0}");
            planet.log(
                catalog,
                step,
                "ship_gift",
                &[("from", &from), ("resource", &resource), ("amount", &n)],
            );
        }
        ShipKind::Trade => {
            // A neighbour's trade offer: what it gives, what it wants back.
            let give = ship.resource.clone().unwrap_or_default();
            let usable = usable_resources(planet, catalog);
            let want = usable
                .iter()
                .filter(|r| **r != give)
                .max_by(|a, b| planet.amount(a).total_cmp(&planet.amount(b)))
                .cloned()
                .unwrap_or_else(|| "food".into());
            let id = next_id(planet);
            planet.offers.push(Offer {
                id,
                from: ship.from.clone(),
                give_resource: give,
                give_amount: amount,
                want_resource: want,
                want_amount: (amount * 0.8).round().max(1.0),
                expires_step: step + hours_to_steps(24.0),
            });
            planet.log(catalog, step, "ship_offer", &[("from", &from)]);
        }
        ShipKind::Idea => {
            if let Some(tech) = ship.tech.clone() {
                if planet.known.insert(tech.clone()) {
                    if planet.research.as_deref() == Some(tech.as_str()) {
                        planet.research = None;
                    }
                    planet.log(
                        catalog,
                        step,
                        "ship_idea",
                        &[("from", &from), ("tech", &tech)],
                    );
                }
            }
        }
        ShipKind::Visit => {
            let era = ship.era as f64;
            planet.add("culture", 5.0 * era * era);
            planet.add("knowledge", 10.0 * era * era);
            let key = if ship.returning {
                "ship_visitor_back"
            } else {
                "ship_visit"
            };
            planet.log(catalog, step, key, &[("from", &from)]);
        }
        ShipKind::Prank => {
            let resource = ship.resource.clone().unwrap_or_default();
            let lost = (planet.amount(&resource) * 0.1).min(amount).floor();
            planet.add(&resource, -lost);
            let n = format!("{lost:.0}");
            let prank = prank_kind(ship.era);
            planet.log(
                catalog,
                step,
                "ship_prank",
                &[
                    ("from", &from),
                    ("resource", &resource),
                    ("amount", &n),
                    ("prank", prank),
                ],
            );
        }
        ShipKind::Message => {
            let persona = planet
                .neighbors
                .iter()
                .find(|n| n.id == ship.from)
                .map(|n| n.persona.clone())
                .unwrap_or_default();
            planet.log(
                catalog,
                step,
                "ship_message",
                &[("from", &from), ("persona", &persona)],
            );
        }
    }
}

/// The prank's flavour, by era: goats in the fields, bells at night, smoke,
/// a hacked billboard.
pub fn prank_kind(era: u32) -> &'static str {
    match era {
        0..=3 => "goats",
        4..=6 => "bells",
        7..=8 => "smoke",
        _ => "hack",
    }
}

/// A ship from home reached a neighbour: it reacts, and often answers.
fn arrive_at_neighbor(planet: &mut Planet, catalog: &Catalog, step: i64, ship: Ship, roll: f64) {
    let Some(index) = planet.neighbors.iter().position(|n| n.id == ship.to) else {
        return;
    };
    let name = planet.neighbors[index].name.clone();
    let era = planet.neighbors[index].era(planet, catalog);
    let usable = usable_resources(planet, catalog);
    let thanks = usable
        .get((roll * usable.len().max(1) as f64) as usize % usable.len().max(1))
        .cloned()
        .unwrap_or_else(|| "food".into());
    let reply = |kind: ShipKind, resource: Option<String>, amount: f64| Ship {
        id: 0,
        from: ship.to.clone(),
        to: "home".into(),
        kind,
        resource,
        amount,
        tech: None,
        era,
        depart_step: 0,
        arrive_step: 0,
        returning: true,
    };
    planet.log(
        catalog,
        step,
        "ship_delivered",
        &[("to", &name), ("kind", ship.kind.as_str())],
    );
    match ship.kind {
        ShipKind::Gift | ShipKind::Idea => {
            planet.neighbors[index].friendship += if ship.kind == ShipKind::Idea { 2 } else { 1 };
            // A thank-you gift, a bit larger than what was given.
            let amount = (capacity(catalog, era) * 0.15)
                .max(ship.amount * 1.1)
                .round();
            launch(
                planet,
                catalog,
                step,
                reply(ShipKind::Gift, Some(thanks), amount),
            );
        }
        ShipKind::Visit => {
            planet.neighbors[index].friendship += 1;
            let mut back = reply(ShipKind::Visit, None, 0.0);
            back.era = ship.era;
            launch(planet, catalog, step, back);
        }
        ShipKind::Prank => {
            planet.neighbors[index].friendship -= 1;
            let pranks_back = planet.neighbors[index].persona == "prankster" || roll < 0.5;
            if pranks_back && era >= ShipKind::Prank.first_era() {
                let amount = (capacity(catalog, era) * 0.3).round();
                launch(
                    planet,
                    catalog,
                    step,
                    reply(ShipKind::Prank, Some(thanks), amount),
                );
            } else {
                launch(planet, catalog, step, reply(ShipKind::Message, None, 0.0));
            }
        }
        ShipKind::Message => {
            launch(planet, catalog, step, reply(ShipKind::Message, None, 0.0));
        }
        ShipKind::Trade => {}
    }
}

/// Totals sent and received per neighbour, for the dock's summary.
pub fn in_flight_counts(planet: &Planet) -> BTreeMap<String, (usize, usize)> {
    let mut map = BTreeMap::new();
    for s in &planet.ships {
        let (key, out) = if s.from == "home" {
            (s.to.clone(), true)
        } else {
            (s.from.clone(), false)
        };
        let entry = map.entry(key).or_insert((0, 0));
        if out {
            entry.0 += 1;
        } else {
            entry.1 += 1;
        }
    }
    map
}
