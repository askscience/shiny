//! A steady player: checks in at given times and plays sensibly, for any era
//! of the catalog. The balancing tests use it, and so do the preview tools
//! (a planet some weeks old to look at).

use std::collections::BTreeSet;

use crate::actions::{can_build, can_research};
use crate::catalog::{Biome, BuildingDef, Catalog, Effect};
use crate::step::free_tiles;
use crate::{apply, Action, Planet, Priorities};

/// Buildings that produce `resource`: a converter making it, or the building
/// it is extracted from.
fn producers<'a>(catalog: &'a Catalog, resource: &str) -> Vec<&'a BuildingDef> {
    let from = catalog
        .resource(resource)
        .and_then(|r| r.from_building.clone());
    catalog
        .buildings
        .iter()
        .filter(|b| {
            Some(&b.id) == from.as_ref()
                || b.effects
                    .iter()
                    .any(|e| matches!(e, Effect::Convert { output, .. } if output == resource))
        })
        .collect()
}

/// Every building needed (transitively) to produce what `target` costs and
/// nobody produces yet.
fn supply_chain(planet: &Planet, catalog: &Catalog, target: &BuildingDef) -> Vec<String> {
    let rates = crate::rates(planet, catalog);
    let mut wanted = Vec::new();
    let mut seen = BTreeSet::new();
    let mut queue: Vec<String> = target.cost.keys().cloned().collect();
    while let Some(resource) = queue.pop() {
        if !seen.insert(resource.clone()) {
            continue;
        }
        let produced = rates.per_hour.get(&resource).copied().unwrap_or(0.0) > 0.0;
        // An intermediate input (copper for bronze) is never "enough": it
        // has to flow.
        let enough = target
            .cost
            .get(&resource)
            .is_some_and(|need| planet.amount(&resource) >= *need);
        if produced || enough {
            continue;
        }
        for b in producers(catalog, &resource) {
            if !planet.has_built(&b.id) && !planet.construction.iter().any(|c| c.building == b.id) {
                wanted.push(b.id.clone());
            }
            for e in &b.effects {
                if let Effect::Convert { inputs, .. } = e {
                    queue.extend(inputs.keys().cloned());
                }
            }
        }
    }
    wanted
}

fn count(planet: &Planet, id: &str) -> usize {
    planet.built().filter(|b| *b == id).count()
        + planet
            .construction
            .iter()
            .filter(|c| c.building == id)
            .count()
}

fn housing(catalog: &Catalog, b: &BuildingDef) -> f64 {
    let _ = catalog;
    b.effects
        .iter()
        .map(|e| {
            if let Effect::Housing { amount } = e {
                *amount
            } else {
                0.0
            }
        })
        .sum()
}

/// Try to start `id` on a sensible free tile; true when started.
fn try_build(
    planet: &mut Planet,
    catalog: &Catalog,
    now: i64,
    id: &str,
    keep_for: Option<&BuildingDef>,
) -> bool {
    let Some(def) = catalog.building(id) else {
        return false;
    };
    if can_build(planet, catalog, id).is_err() {
        return false;
    }
    let mut tiles = free_tiles(planet, &def.biomes);
    // Keep a couple of tiles the monument can stand on.
    if let Some(monument) = keep_for {
        if !def.monument {
            let room = free_tiles(planet, &monument.biomes);
            tiles.retain(|t| room.len() > 2 || !room.contains(t));
        }
    }
    // Buildings that fit anywhere go on the least useful land.
    let rank = |b: Biome| {
        [
            Biome::Ice,
            Biome::Desert,
            Biome::Coast,
            Biome::Forest,
            Biome::Plains,
            Biome::Mountain,
        ]
        .iter()
        .position(|x| *x == b)
        .unwrap()
    };
    if def.biomes.is_empty() {
        tiles.sort_by_key(|&i| rank(planet.tiles[i].biome));
    }
    if tiles.is_empty() {
        // No room: clear the oldest ordinary building on land that fits.
        let victim = planet
            .tiles
            .iter()
            .enumerate()
            .filter(|(_, t)| def.biomes.is_empty() || def.biomes.contains(&t.biome))
            .filter_map(|(i, t)| {
                t.building
                    .as_ref()
                    .map(|b| (i, catalog.building(b).unwrap()))
            })
            .filter(|(_, b)| {
                !b.monument && b.era < planet.era.saturating_sub(1).max(1) + 1 && b.id != id
            })
            .min_by_key(|(_, b)| b.era)
            .map(|(i, _)| i);
        let Some(tile) = victim else { return false };
        if apply(planet, catalog, now, &Action::Demolish { tile }).is_err() {
            return false;
        }
        tiles.push(tile);
    }
    let Some(&tile) = tiles.first() else {
        return false;
    };
    apply(
        planet,
        catalog,
        now,
        &Action::Build {
            building: id.into(),
            tile,
        },
    )
    .is_ok()
}

/// One visit: answer the event, keep the people fed and housed, research
/// and build what helps reach the next era.
pub fn play_session(planet: &mut Planet, catalog: &Catalog, now: i64) {
    crate::advance(planet, catalog, now);
    if let Some(pending) = planet.pending_event.clone() {
        let choice = catalog.event(&pending.id).unwrap().choices[0].id.clone();
        apply(planet, catalog, now, &Action::ResolveEvent { choice }).unwrap();
    }
    // A fair trade from a neighbour, when there is plenty to give.
    for offer in planet.offers.clone() {
        if planet.amount(&offer.want_resource) > offer.want_amount * 3.0 {
            let _ = apply(planet, catalog, now, &Action::AcceptOffer { id: offer.id });
        }
    }
    let rates = crate::rates(planet, catalog);
    let food_rate = rates.per_hour.get("food").copied().unwrap_or(0.0);
    let priorities = if food_rate < 0.2 || planet.amount("food") < 20.0 {
        Priorities {
            food: 4,
            materials: 2,
            knowledge: 2,
            culture: 1,
        }
    } else {
        Priorities {
            food: 2,
            materials: 3,
            knowledge: 3,
            culture: 1,
        }
    };
    apply(planet, catalog, now, &Action::SetPriorities { priorities }).unwrap();

    let era = catalog.era(planet.era).unwrap().clone();
    if planet.research.is_none() {
        let mut options: Vec<_> = catalog
            .techs
            .iter()
            .filter(|t| can_research(planet, catalog, &t.id).is_ok())
            .collect();
        // Era requirements first, then the cheapest.
        options.sort_by(|a, b| {
            let ra = !era.requires.contains(&a.id);
            let rb = !era.requires.contains(&b.id);
            (ra, a.cost as i64).cmp(&(rb, b.cost as i64))
        });
        if let Some(t) = options.first() {
            let tech = t.id.clone();
            apply(planet, catalog, now, &Action::Research { tech }).unwrap();
        }
    }

    let monument = catalog.building(&era.monument).unwrap().clone();
    let monument_pending = !planet.has_built(&monument.id)
        && !planet
            .construction
            .iter()
            .any(|c| c.building == monument.id);
    let saving = monument_pending && era.requires.iter().all(|t| planet.known.contains(t));
    let crowded = planet.population > rates.housing - 2.0;

    let mut wanted: Vec<String> = Vec::new();
    if crowded {
        // The best housing available.
        let mut homes: Vec<&BuildingDef> = catalog
            .buildings
            .iter()
            .filter(|b| housing(catalog, b) > 0.0 && !b.monument)
            .collect();
        homes.sort_by(|a, b| housing(catalog, b).total_cmp(&housing(catalog, a)));
        wanted.extend(homes.iter().map(|b| b.id.clone()));
    }
    if saving {
        wanted.extend(supply_chain(planet, catalog, &monument));
        wanted.insert(0, monument.id.clone());
    } else {
        // Producers of the coming monument's inputs, then one of each
        // building of the eras reached (cheapest first), a few fields.
        wanted.extend(supply_chain(planet, catalog, &monument));
        let mut rest: Vec<&BuildingDef> = catalog
            .buildings
            .iter()
            .filter(|b| !b.monument && b.era <= planet.era && housing(catalog, b) == 0.0)
            .collect();
        rest.sort_by(|a, b| {
            (b.era, a.cost.values().sum::<f64>() as i64)
                .cmp(&(a.era, b.cost.values().sum::<f64>() as i64))
        });
        wanted.extend(rest.iter().map(|b| b.id.clone()));
    }
    let mut started = 0;
    for id in wanted {
        let def = catalog.building(&id).unwrap();
        let limit = if housing(catalog, def) > 0.0 {
            99
        } else if id == "field"
            || def.biomes == [Biome::Mountain]
            || def
                .effects
                .iter()
                .any(|e| matches!(e, Effect::Convert { .. }))
        {
            3
        } else {
            1
        };
        if !def.monument && count(planet, &id) >= limit {
            continue;
        }
        // While saving for the monument, nothing may eat into what the
        // monument still needs.
        if saving && !def.monument {
            let eats = def.cost.iter().any(|(r, c)| {
                monument
                    .cost
                    .get(r)
                    .is_some_and(|need| planet.amount(r) - c < *need)
            });
            if eats {
                continue;
            }
        }
        if try_build(
            planet,
            catalog,
            now,
            &id,
            monument_pending.then_some(&monument),
        ) {
            started += 1;
            if saving || started >= 2 {
                break;
            }
        }
    }
}

/// Play `planet` from its current step to `until_ms`: `visits_per_day`
/// visits spread over waking hours (from 09:00 UTC), as a steady player.
pub fn play_until(planet: &mut Planet, catalog: &Catalog, until_ms: i64, visits_per_day: i64) {
    const HOUR: i64 = 3_600_000;
    const DAY: i64 = 24 * HOUR;
    let visits = visits_per_day.clamp(1, 8);
    let gap = 12 * HOUR / visits;
    let mut day = (planet.step * crate::STEP_MS).div_euclid(DAY) * DAY;
    while day < until_ms {
        for k in 0..visits {
            let now = day + 9 * HOUR + k * gap;
            if now > planet.step * crate::STEP_MS && now < until_ms {
                play_session(planet, catalog, now);
            }
        }
        day += DAY;
    }
    crate::advance(planet, catalog, until_ms);
}
