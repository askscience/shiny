//! A steady player: checks in at given times and plays sensibly, for any era
//! of the catalog. The balancing tests use it, and so do the preview tools
//! (a planet some weeks old to look at).

use std::collections::BTreeSet;

use crate::actions::{can_build, can_research};
use crate::catalog::{Biome, BuildingDef, Catalog, Effect};
use crate::cities::{sites_for, LinkKind};
use crate::{apply, Action, Planet};

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
    let mut tiles = sites_for(planet, catalog, def);
    // Keep a couple of tiles the monument can stand on.
    if let Some(monument) = keep_for {
        if !def.monument && !def.city {
            let room = sites_for(planet, catalog, monument);
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
        .unwrap_or(9)
    };
    if def.biomes.is_empty() {
        tiles.sort_by_key(|&i| rank(planet.tiles[i].biome));
    }
    if tiles.is_empty() && def.city {
        return false;
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
                !b.monument && !b.city && b.era < planet.era.saturating_sub(1).max(1) + 1 && b.id != id
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

/// Point the crews where they are needed: more on food when it runs short,
/// otherwise a balance of food and materials, the rest thinking.
pub fn set_orders(planet: &mut Planet, catalog: &Catalog, now: i64, hungry: bool) {
    // With nothing left to study, thinking is wasted: more hands on wood
    // and stone.
    let minds_idle = planet.research.is_none()
        && !catalog.techs.iter().any(|t| can_research(planet, catalog, &t.id).is_ok());
    let mut want: Vec<(&str, usize)> = if hungry {
        vec![("food", 4), ("wood", 1), ("stone", 1)]
    } else if minds_idle {
        vec![("food", 2), ("wood", 3), ("stone", 3)]
    } else {
        vec![("food", 2), ("wood", 1), ("stone", 1)]
    };
    // One more crew on whatever the era's monument still lacks.
    if let Some(monument) = catalog.era(planet.era).and_then(|e| catalog.building(&e.monument)) {
        for (resource, need) in &monument.cost {
            if planet.amount(resource) >= *need {
                continue;
            }
            if let Some(w) = want.iter_mut().find(|(r, _)| r == resource) {
                w.1 += 1;
            }
        }
    }
    // One crew on the newest material the land gives (clay, then marble),
    // and the rest of the people thinking.
    if !hungry {
        for extra in ["marble", "clay"] {
            if planet.orders.iter().any(|o| o.resource == extra)
                || crate::orders::nearest_for(planet, catalog, extra, 0.3).is_some()
            {
                want.push((extra, 1));
                break;
            }
        }
    }
    for (resource, count) in &want {
        let mut have: Vec<usize> = planet
            .orders
            .iter()
            .filter(|o| o.resource == *resource)
            .map(|o| o.tile)
            .collect();
        while have.len() > *count {
            let tile = have.pop().unwrap();
            let _ = apply(planet, catalog, now, &Action::StopHarvest { tile });
        }
    }
    let known: Vec<&str> = want.iter().map(|(r, _)| *r).collect();
    let stray: Vec<usize> = planet
        .orders
        .iter()
        .filter(|o| !known.contains(&o.resource.as_str()))
        .map(|o| o.tile)
        .collect();
    for tile in stray {
        let _ = apply(planet, catalog, now, &Action::StopHarvest { tile });
    }
    for (resource, count) in want {
        while planet.orders.iter().filter(|o| o.resource == resource).count() < count {
            let Some(tile) = crate::orders::nearest_for(planet, catalog, resource, 0.3) else {
                break;
            };
            let action = Action::Harvest {
                tile,
                resource: resource.into(),
            };
            if apply(planet, catalog, now, &action).is_err() {
                break;
            }
        }
    }
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
    let hungry = food_rate < 0.2 || planet.amount("food") < 20.0;
    set_orders(planet, catalog, now, hungry);

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
            .filter(|b| housing(catalog, b) > 0.0 && !b.monument && !b.city)
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
            .filter(|b| !b.monument && !b.city && b.era <= planet.era && housing(catalog, b) == 0.0)
            .collect();
        rest.sort_by(|a, b| {
            (b.era, a.cost.values().sum::<f64>() as i64)
                .cmp(&(a.era, b.cost.values().sum::<f64>() as i64))
        });
        wanted.extend(rest.iter().map(|b| b.id.clone()));
    }
    // What the era's monument and the next buildings are made of.
    let mut needs: Vec<String> = monument.cost.keys().cloned().collect();
    needs.extend(["wood".to_string(), "stone".to_string()]);
    grow_cities(planet, catalog, now, &needs);
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

/// Found a new city when the land near the old ones runs short or a
/// resource the era needs lies out of reach (next to that resource), and
/// link every city to the capital.
fn grow_cities(planet: &mut Planet, catalog: &Catalog, now: i64, needs: &[String]) {
    let Some(colony) = catalog.buildings.iter().find(|b| b.city).cloned() else {
        return;
    };
    let founding = planet
        .construction
        .iter()
        .any(|c| catalog.building(&c.building).is_some_and(|b| b.city));
    let reach = crate::orders::in_reach(planet);
    let out_of_reach: Vec<usize> = needs
        .iter()
        .filter(|r| !planet.orders.iter().any(|o| &o.resource == *r))
        .filter(|r| crate::orders::nearest_for(planet, catalog, r, 0.3).is_none())
        .flat_map(|r| {
            (0..planet.tiles.len())
                .filter(|&i| !reach[i])
                .filter(|&i| crate::orders::harvestable(planet, catalog, i).contains(r))
                .collect::<Vec<_>>()
        })
        .collect();
    let free = crate::step::buildable(planet).len();
    // One more city every other era, sooner when the land runs out.
    let crowded = planet.cities.len() < 1 + (planet.era as usize).saturating_sub(1) / 2 || free < 3;
    if !founding && (crowded || !out_of_reach.is_empty()) && planet.cities.len() < 8 {
        let mut sites = sites_for(planet, catalog, &colony);
        if !out_of_reach.is_empty() {
            if let Some(layout) = crate::cells::layout(planet.tiles.len()) {
                let near = layout.hops(&out_of_reach, |_| true);
                sites.sort_by_key(|&c| (near[c], c));
            }
        }
        if let Some(&tile) = sites.first() {
            let _ = apply(planet, catalog, now, &Action::Build { building: colony.id.clone(), tile });
        }
    }
    let linked = crate::cities::connected(planet);
    for city in 1..planet.cities.len() {
        let pending = planet.links.iter().any(|l| l.from == city || l.to == city);
        if linked[city] || pending {
            continue;
        }
        let to = (0..city).find(|&c| linked[c]).unwrap_or(0);
        for kind in [LinkKind::Road, LinkKind::Sea] {
            let action = Action::Link { kind, from: to, to: city };
            if apply(planet, catalog, now, &action).is_ok() {
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
