//! What the window gets: the planet as of now, plus everything it needs to
//! draw choices without knowing the rules (what can be researched or built,
//! and why not).

use serde_json::{json, Value as Json};

use crate::actions::{can_build, can_research, ActionError};
use crate::step::free_tiles;
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
                "tiles": free_tiles(planet, &b.biomes),
            })
        })
        .collect();

    let era = catalog.era(planet.era);
    json!({
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
