//! What the window gets: the planet as of now, plus everything it needs to
//! draw choices without knowing the rules (what can be researched or built,
//! and why not).

use serde_json::{json, Value as Json};

use crate::actions::{can_build, can_research, ActionError};
use crate::step::buildable;
use crate::{rates, Catalog, Planet, STEP_MS};

fn reason(error: &ActionError) -> &'static str {
    match error {
        ActionError::AlreadyKnown(_) => "known",
        ActionError::TechLocked(_) | ActionError::BuildingLocked(_) => "locked",
        ActionError::MonumentDone(_) => "monument_done",
        ActionError::NotAffordable(_) => "not_affordable",
        _ => "unavailable",
    }
}

pub fn planet_view(planet: &Planet, catalog: &Catalog, now_ms: i64) -> Json {
    let techs: Vec<Json> = catalog
        .techs
        .iter()
        .filter(|t| t.era <= planet.era)
        .map(|t| {
            let status = if planet.known.contains(&t.id) {
                "known".to_string()
            } else if planet.research.as_deref() == Some(t.id.as_str()) {
                "researching".to_string()
            } else {
                match can_research(planet, catalog, &t.id) {
                    Ok(()) => "available".to_string(),
                    Err(e) => reason(&e).to_string(),
                }
            };
            json!({ "id": t.id, "era": t.era, "cost": t.cost, "requires": t.requires, "status": status })
        })
        .collect();

    let buildings: Vec<Json> = catalog
        .buildings
        .iter()
        .filter(|b| b.era <= planet.era)
        .map(|b| {
            let status = match can_build(planet, catalog, &b.id) {
                Ok(()) => "available",
                Err(e) => reason(&e),
            };
            json!({
                "id": b.id,
                "era": b.era,
                "cost": b.cost,
                "build_hours": b.build_hours,
                "biomes": b.biomes,
                "monument": b.monument,
                "requires": b.requires,
                "status": status,
            })
        })
        .collect();

    let era = catalog.era(planet.era);
    let dock = json!({
        "capacity": crate::neighbors::capacity(catalog, planet.era),
        "travel_hours": crate::neighbors::travel_hours(catalog, planet.era),
        "kinds": crate::neighbors::ShipKind::ALL
            .iter()
            .filter(|k| **k != crate::neighbors::ShipKind::Trade)
            .map(|k| json!({
                "kind": k.as_str(),
                "first_era": k.first_era(),
                "unlocked": crate::actions::can_send(planet, *k),
            }))
            .collect::<Vec<_>>(),
        "resources": crate::neighbors::usable_resources(planet, catalog),
        "idea_cost": 10.0 * planet.era as f64,
        "neighbors": planet.neighbors.iter().map(|n| json!({
            "id": n.id,
            "name": n.name,
            "climate": n.climate,
            "persona": n.persona,
            "friendship": n.friendship,
            "era": n.era(planet, catalog),
        })).collect::<Vec<_>>(),
    });
    let resource_eras: serde_json::Map<String, Json> = catalog
        .resources
        .iter()
        .map(|r| (r.id.clone(), json!(catalog.resource_era(&r.id))))
        .collect();
    json!({
        "resource_eras": resource_eras,
        "buildable": buildable(planet),
        // Planet time runs `rate` times as fast as the wall clock now.
        "clock": {
            "rate": planet.clock.rate(),
            "present": planet.clock.is_present(),
            "present_left_ms": (planet.clock.present_until - planet.clock.wall_ms).max(0),
            "active_rate": crate::clock::ACTIVE_RATE,
            "away_rate": crate::clock::AWAY_RATE_HALVES as f64 / 2.0,
        },
        "crew": crate::orders::crew(planet, catalog),
        "slots": crate::orders::SLOTS,
        "work_reach": crate::orders::in_reach(planet)
            .iter()
            .enumerate()
            .filter(|(_, r)| **r)
            .map(|(i, _)| i)
            .collect::<Vec<_>>(),
        "harvest": (0..planet.tiles.len())
            .filter_map(|i| {
                let list = crate::orders::harvestable(planet, catalog, i);
                (!list.is_empty()).then(|| (i.to_string(), json!(list)))
            })
            .collect::<serde_json::Map<String, Json>>(),
        "dock": dock,
        "now_ms": now_ms,
        "step_ms": STEP_MS,
        "planet": planet,
        "rates": rates(planet, catalog),
        "era": era.map(|e| json!({
            "id": e.id,
            "number": planet.era,
            "requires": e.requires,
            "monument": e.monument,
        })),
        "techs": techs,
        "buildings": buildings,
        "event": planet.pending_event.as_ref().and_then(|p| catalog.event(&p.id)).map(|e| json!({
            "id": e.id,
            "choices": e.choices.iter().map(|c| json!({ "id": c.id, "outcomes": c.outcomes })).collect::<Vec<_>>(),
        })),
    })
}
