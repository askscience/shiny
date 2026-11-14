//! How a planet changes over time: one step at a time, at step boundaries.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::catalog::{Biome, Catalog, Category, Effect, Outcome};
use crate::planet::{Notable, PendingEvent, Planet};
use crate::rng::Rng;
use crate::{hours_to_steps, step_of, STEPS_PER_HOUR, STEP_HOURS};

/// What the planet produces and consumes per hour right now. The window
/// shows it, the assistant reads it, and a step applies it.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Rates {
    /// Net change per hour of each resource (production minus consumption,
    /// conversions included).
    pub per_hour: BTreeMap<String, f64>,
    pub workers: BTreeMap<String, f64>,
    pub housing: f64,
    pub storage: f64,
    /// Where the mood is heading (0..1).
    pub mood_target: f64,
    /// Output multiplier from mood, season and absence.
    pub efficiency: f64,
    /// What each order's crew gathers per hour (in `planet.orders` order).
    pub orders: Vec<f64>,
    /// What each mine digs per hour, by cell.
    pub extracted: Vec<(usize, f64)>,
    pub season: usize,
}

/// Advance the planet to `now_ms`: every step boundary up to and including
/// it, in order. Earlier times are ignored.
pub fn advance(planet: &mut Planet, catalog: &Catalog, now_ms: i64) {
    planet.fit_tiles(catalog);
    let target = step_of(now_ms);
    while planet.step < target {
        let step = planet.step + 1;
        run_step(planet, catalog, step);
        planet.step = step;
    }
}

/// Every lasting effect in force: standing buildings, discoveries and
/// notable people.
fn effects<'a>(planet: &'a Planet, catalog: &'a Catalog) -> Vec<Effect> {
    let mut list = Vec::new();
    for id in planet.built() {
        if let Some(b) = catalog.building(id) {
            list.extend(b.effects.iter().cloned());
        }
    }
    for id in &planet.known {
        if let Some(t) = catalog.tech(id) {
            list.extend(t.effects.iter().cloned());
        }
    }
    for n in &planet.notables {
        list.push(Effect::Bonus {
            category: n.category,
            amount: catalog.tuning.notable_bonus,
        });
    }
    list
}

fn season_of(step: i64, catalog: &Catalog) -> usize {
    let season_steps = (catalog.tuning.season_days * 24.0 * STEPS_PER_HOUR as f64) as i64;
    (step.div_euclid(season_steps.max(1)) % 4) as usize
}

/// The planet's rates at `step`.
pub fn rates(planet: &Planet, catalog: &Catalog) -> Rates {
    rates_at(planet, catalog, planet.step)
}

fn rates_at(planet: &Planet, catalog: &Catalog, step: i64) -> Rates {
    let t = &catalog.tuning;
    let effects = effects(planet, catalog);
    let mut bonus: BTreeMap<Category, f64> = BTreeMap::new();
    let mut housing = t.base_housing;
    let mut storage = t.base_storage;
    let mut mood_push = 0.0;
    for effect in &effects {
        match effect {
            Effect::Housing { amount } => housing += amount,
            Effect::Bonus { category, amount } => *bonus.entry(*category).or_insert(0.0) += amount,
            Effect::Storage { amount } => storage += amount,
            Effect::Mood { amount } => mood_push += amount,
            Effect::Convert { .. } => {}
        }
    }
    // Ideas travel along roads and sea routes.
    let linked = crate::cities::link_bonus(planet);
    if linked > 0.0 {
        for category in [Category::Knowledge, Category::Culture] {
            *bonus.entry(category).or_insert(0.0) += linked;
        }
    }
    for (id, value) in &planet.traits {
        // Traits lean the society: peaceful people are happier, curious ones
        // learn faster, traditional ones keep their culture.
        let v = *value as f64 * 0.02;
        match id.as_str() {
            "peaceful" => mood_push += v,
            "curious" => *bonus.entry(Category::Knowledge).or_insert(0.0) += v,
            "traditional" => *bonus.entry(Category::Culture).or_insert(0.0) += v,
            "open" => *bonus.entry(Category::Food).or_insert(0.0) += v,
            _ => {}
        }
    }

    let season = season_of(step, catalog);
    let idle =
        if (step - planet.last_action_step) as f64 / STEPS_PER_HOUR as f64 > t.idle_after_hours {
            t.idle_factor
        } else {
            1.0
        };
    let age_hours = (step - planet.created_step) as f64 / STEPS_PER_HOUR as f64;
    let zeal = 1.0 + t.zeal_bonus * (-age_hours.max(0.0) / t.zeal_hours.max(0.01)).exp();
    let efficiency = (0.6 + 0.8 * planet.mood) * idle * zeal;
    let food_factor = catalog
        .climate(&planet.climate)
        .map(|c| c.food)
        .unwrap_or(1.0);

    // Crews on their cells (and in their mines); everyone else thinks
    // (two thirds) and makes culture (one third).
    let workers_total = planet.population * t.worker_share;
    let crew = crate::orders::crew(planet, catalog);
    let mut workers: BTreeMap<String, f64> = Category::ALL
        .iter()
        .map(|c| (c.as_str().to_string(), 0.0))
        .collect();
    let mut per_hour: BTreeMap<String, f64> = BTreeMap::new();
    let mult = |category: Category| (1.0 + bonus.get(&category).copied().unwrap_or(0.0)) * efficiency;
    let per_worker = |category: Category| match category {
        Category::Food => t.food_per_worker_hour * t.seasons[season] * food_factor,
        Category::Materials => t.materials_per_worker_hour,
        Category::Knowledge => t.knowledge_per_worker_hour,
        Category::Culture => t.culture_per_worker_hour,
    };
    let mut busy = 0.0;
    let mut order_out = Vec::new();
    // A city with no way to the capital works slower.
    let pace = crate::cities::pace(planet);
    let pace_at = |cell: usize| pace.get(cell).copied().unwrap_or(1.0);
    for order in &planet.orders {
        let category = crate::orders::category(catalog, &order.resource);
        let stock = planet.tiles.get(order.tile).map(|t| t.stock).unwrap_or(1.0);
        let out = crew
            * per_worker(category)
            * mult(category)
            * crate::orders::yield_factor(catalog, &order.resource, stock)
            * pace_at(order.tile);
        *per_hour.entry(order.resource.clone()).or_insert(0.0) += out;
        order_out.push(out);
        *workers.entry(category.as_str().into()).or_insert(0.0) += crew;
        busy += crew;
    }
    let mut extracted = Vec::new();
    for (cell, yields) in crate::orders::extractors(planet, catalog) {
        let out = crew * per_worker(Category::Materials) * mult(Category::Materials) * pace_at(cell);
        extracted.push((cell, out));
        for id in &yields {
            *per_hour.entry(id.clone()).or_insert(0.0) += out / yields.len() as f64;
        }
        *workers.entry("materials".into()).or_insert(0.0) += crew;
        busy += crew;
    }
    let thinkers = (workers_total - busy).max(0.0);
    for (category, share) in [(Category::Knowledge, 2.0 / 3.0), (Category::Culture, 1.0 / 3.0)] {
        let w = thinkers * share;
        workers.insert(category.as_str().into(), w);
        *per_hour.entry(category.as_str().into()).or_insert(0.0) +=
            w * per_worker(category) * mult(category);
    }
    *per_hour.entry("food".into()).or_insert(0.0) -= planet.population * t.food_per_person_hour;

    // Conversions run while there is input for a whole step. A workshop's
    // throughput grows with the people who can staff it and with what
    // helps materials, so a big city's foundry is not a village's.
    let staffing = (1.0 + bonus.get(&Category::Materials).copied().unwrap_or(0.0))
        * efficiency.min(3.0)
        * (1.0 + planet.population / 200.0);
    for effect in &effects {
        if let Effect::Convert {
            inputs,
            output,
            per_hour: base_rate,
        } = effect
        {
            let rate = &(base_rate * staffing);
            // Workshops leave a reserve (a tenth of the storage) of every
            // input that is still needed to build something (wood, stone,
            // coal...), and stop when their output is full. An input only
            // workshops use (copper, tin) flows freely.
            let reserve = |id: &str| {
                let builds_with = catalog
                    .buildings
                    .iter()
                    .any(|b| b.era >= planet.era && b.cost.contains_key(id));
                if builds_with {
                    storage * 0.1
                } else {
                    0.0
                }
            };
            let enough = inputs
                .iter()
                .all(|(id, need)| planet.amount(id) >= need * rate * STEP_HOURS + reserve(id));
            let room = planet.amount(output) < storage;
            if enough && room {
                for (id, need) in inputs {
                    *per_hour.entry(id.clone()).or_insert(0.0) -= need * rate;
                }
                *per_hour.entry(output.clone()).or_insert(0.0) += rate;
            }
        }
    }

    let food_ok = if planet.starving { 0.0 } else { 1.0 };
    let housing_ok = (housing / planet.population.max(1.0)).min(1.0);
    let culture_rate = per_hour.get("culture").copied().unwrap_or(0.0);
    let culture_ok = (culture_rate / planet.population.max(1.0) / t.culture_for_full_mood).min(1.0);
    let mood_target =
        (0.15 + 0.35 * food_ok + 0.2 * housing_ok + 0.3 * culture_ok + mood_push).clamp(0.0, 1.0);

    Rates {
        per_hour,
        workers,
        housing,
        storage,
        mood_target,
        efficiency,
        orders: order_out,
        extracted,
        season,
    }
}

fn run_step(planet: &mut Planet, catalog: &Catalog, step: i64) {
    let t = &catalog.tuning;
    let mut rng = Rng::for_step(planet.seed, step);
    let rates = rates_at(planet, catalog, step);

    // Resources, capped by storage except the uncapped ones.
    let mut ran_out_of_food = false;
    for (id, rate) in &rates.per_hour {
        let before = planet.amount(id);
        let mut value = before + rate * STEP_HOURS;
        if id == "food" && value < 0.0 {
            ran_out_of_food = true;
        }
        value = value.max(0.0);
        let capped = catalog.resource(id).map(|r| !r.uncapped).unwrap_or(true);
        if capped {
            value = value.min(rates.storage.max(before));
        }
        planet.resources.insert(id.clone(), value);
    }
    planet.starving = ran_out_of_food;

    // People: growth while fed and housed, slow loss when hungry, people
    // leaving when there is no room. Never below the floor.
    let pop = planet.population;
    if planet.starving {
        planet.population = (pop - pop * t.starvation_per_hour * STEP_HOURS).max(t.min_population);
    } else if pop < rates.housing {
        let growth = pop * t.growth_per_hour * planet.mood * STEP_HOURS * rates.efficiency.min(1.0);
        planet.population = (pop + growth).min(rates.housing);
    } else if pop > rates.housing {
        let leaving = (pop - rates.housing) * t.emigration_per_hour * STEP_HOURS;
        planet.population = (pop - leaving).max(rates.housing.max(t.min_population));
    }
    let milestone = (planet.population / 10.0).floor() as u32;
    if milestone > planet.population_milestone {
        planet.population_milestone = milestone;
        let n = (milestone * 10).to_string();
        planet.log(catalog, step, "population_milestone", &[("population", &n)]);
    }

    planet.mood += (rates.mood_target - planet.mood) * t.mood_approach_per_hour * STEP_HOURS;
    planet.mood = planet.mood.clamp(0.0, 1.0);

    crate::orders::run_orders(planet, catalog, step, &rates.orders, &rates.extracted);

    // Buildings that are finished now.
    let mut finished = Vec::new();
    planet.construction.retain(|c| {
        if c.done_step <= step {
            finished.push(c.clone());
            false
        } else {
            true
        }
    });
    let stood: Vec<(String, usize)> = finished.iter().map(|c| (c.building.clone(), c.tile)).collect();
    for c in finished {
        if let Some(tile) = planet.tiles.get_mut(c.tile) {
            tile.building = Some(c.building.clone());
        }
        let tile = c.tile.to_string();
        planet.log(
            catalog,
            step,
            "building_done",
            &[("building", &c.building), ("tile", &tile)],
        );
    }
    crate::cities::run_cities(planet, catalog, step, &stood);

    // Research completes as soon as the knowledge is there.
    if let Some(id) = planet.research.clone() {
        if let Some(tech) = catalog.tech(&id) {
            if planet.amount("knowledge") >= tech.cost {
                planet.add("knowledge", -tech.cost);
                planet.known.insert(id.clone());
                planet.research = None;
                planet.log(catalog, step, "tech_done", &[("tech", &id)]);
            }
        }
    }

    check_era(planet, catalog, step);
    run_events(planet, catalog, step, &mut rng);
    run_notables(planet, catalog, step, &mut rng);
    crate::neighbors::run_neighbors(planet, catalog, step, &mut rng);
}

/// Move to the next era when the current one is complete.
pub(crate) fn check_era(planet: &mut Planet, catalog: &Catalog, step: i64) {
    if planet.eras_done {
        return;
    }
    let Some(era) = catalog.era(planet.era) else {
        return;
    };
    let complete =
        era.requires.iter().all(|t| planet.known.contains(t)) && planet.has_built(&era.monument);
    if !complete {
        return;
    }
    if planet.era as usize >= catalog.eras.len() {
        planet.eras_done = true;
        let id = era.id.clone();
        planet.log(catalog, step, "eras_done", &[("era", &id)]);
    } else {
        planet.era += 1;
        let id = catalog
            .era(planet.era)
            .map(|e| e.id.clone())
            .unwrap_or_default();
        planet.log(catalog, step, "era_reached", &[("era", &id)]);
    }
}

fn run_events(planet: &mut Planet, catalog: &Catalog, step: i64, rng: &mut Rng) {
    let t = &catalog.tuning;
    // Draw first, whatever happens: the number of draws per step must not
    // depend on the branch taken, or later draws would shift.
    let roll = rng.unit();
    let pick = rng.next_u64();

    if let Some(pending) = planet.pending_event.clone() {
        if step - pending.step >= hours_to_steps(t.event_expires_hours) {
            planet.pending_event = None;
            if let Some(event) = catalog.event(&pending.id) {
                let choice = event.choices[0].id.clone();
                resolve(planet, catalog, step, &pending.id, &choice);
                planet.log(
                    catalog,
                    step,
                    "event_auto",
                    &[("event", &pending.id), ("choice", &choice)],
                );
            }
        }
        return;
    }
    if step - planet.last_event_step < hours_to_steps(t.event_gap_hours) {
        return;
    }
    if roll >= t.event_chance_per_hour * STEP_HOURS {
        return;
    }
    let eligible: Vec<&crate::catalog::EventDef> = catalog
        .events
        .iter()
        .filter(|e| planet.era >= e.era_min && e.era_max.is_none_or(|max| planet.era <= max))
        .filter(|e| {
            e.unless_known
                .as_ref()
                .is_none_or(|id| !planet.known.contains(id))
        })
        .filter(|e| {
            e.if_known
                .as_ref()
                .is_none_or(|id| planet.known.contains(id))
        })
        .collect();
    let weights: Vec<u32> = eligible.iter().map(|e| e.weight).collect();
    let mut pick_rng = Rng::new(pick);
    if let Some(i) = pick_rng.weighted(&weights) {
        let id = eligible[i].id.clone();
        planet.pending_event = Some(PendingEvent {
            id: id.clone(),
            step,
        });
        planet.last_event_step = step;
        planet.log(catalog, step, "event", &[("event", &id)]);
    }
}

/// Apply the outcomes of one choice of an event.
pub(crate) fn resolve(
    planet: &mut Planet,
    catalog: &Catalog,
    step: i64,
    event: &str,
    choice: &str,
) {
    let Some(def) = catalog.event(event) else {
        return;
    };
    let Some(choice) = def.choices.iter().find(|c| c.id == choice) else {
        return;
    };
    for outcome in &choice.outcomes {
        match outcome {
            Outcome::Resource { id, amount } => planet.add(id, *amount),
            Outcome::Mood { amount } => planet.mood = (planet.mood + amount).clamp(0.0, 1.0),
            Outcome::Population { amount } => {
                planet.population = (planet.population + amount).max(catalog.tuning.min_population)
            }
            Outcome::Trait { id, amount } => {
                *planet.traits.entry(id.clone()).or_insert(0) += amount
            }
            Outcome::Grant { tech } => {
                if planet.known.insert(tech.clone()) {
                    if planet.research.as_deref() == Some(tech.as_str()) {
                        planet.research = None;
                    }
                    planet.log(catalog, step, "tech_granted", &[("tech", tech)]);
                }
            }
        }
    }
    check_era(planet, catalog, step);
}

const SYLLABLES: [&str; 16] = [
    "ka", "lu", "mi", "ra", "to", "ne", "sa", "do", "ul", "ia", "ve", "bo", "ri", "an", "zu", "el",
];

fn notable_name(rng: &mut Rng) -> String {
    let count = 2 + rng.below(2);
    let mut name: String = (0..count)
        .map(|_| SYLLABLES[rng.below(SYLLABLES.len())])
        .collect();
    if let Some(first) = name.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    name
}

fn run_notables(planet: &mut Planet, catalog: &Catalog, step: i64, rng: &mut Rng) {
    let t = &catalog.tuning;
    let roll = rng.unit();
    let mut name_rng = Rng::new(rng.next_u64());

    let mut gone = Vec::new();
    planet.notables.retain(|n| {
        if n.until_step <= step {
            gone.push(n.name.clone());
            false
        } else {
            true
        }
    });
    for name in gone {
        planet.log(catalog, step, "notable_gone", &[("name", &name)]);
    }

    if planet.notables.len() >= t.max_notables || roll >= t.notable_chance_per_hour * STEP_HOURS {
        return;
    }
    // A notable person follows what most of the people do.
    let rates = rates_at(planet, catalog, step);
    let category = Category::ALL
        .iter()
        .copied()
        .max_by(|&a, &b| {
            let wa = rates.workers.get(a.as_str()).copied().unwrap_or(0.0);
            let wb = rates.workers.get(b.as_str()).copied().unwrap_or(0.0);
            wa.total_cmp(&wb).then(b.cmp(&a))
        })
        .unwrap_or(Category::Food);
    let name = notable_name(&mut name_rng);
    planet.notables.push(Notable {
        name: name.clone(),
        category,
        born_step: step,
        until_step: step + hours_to_steps(t.notable_days * 24.0),
    });
    planet.log(
        catalog,
        step,
        "notable_born",
        &[("name", &name), ("category", category.as_str())],
    );
}

/// How far from the settled land (the home, buildings and building sites)
/// new buildings may go, in cells.
pub const REACH: usize = 2;

/// Free land within reach of the settlement and near a city centre
/// (`cities::CITY_RADIUS`), nearest to the home first.
pub fn buildable(planet: &Planet) -> Vec<usize> {
    let Some(layout) = crate::cells::layout(planet.tiles.len()) else {
        return Vec::new();
    };
    let land = |j: usize| planet.tiles[j].biome != Biome::Ocean;
    let mut settled = vec![planet.home];
    settled.extend(
        planet
            .tiles
            .iter()
            .enumerate()
            .filter(|(_, t)| t.building.is_some())
            .map(|(i, _)| i),
    );
    settled.extend(planet.construction.iter().map(|c| c.tile));
    let reach = layout.hops(&settled, land);
    let near = crate::cities::near_city(planet);
    let from_home = layout.hops(&[planet.home], land);
    let mut cells: Vec<usize> = (0..planet.tiles.len())
        .filter(|&i| {
            reach[i] <= REACH
                && (near[i] || planet.cities.is_empty())
                && land(i)
                && planet.tiles[i].building.is_none()
                && !planet.construction.iter().any(|c| c.tile == i)
        })
        .collect();
    cells.sort_by_key(|&i| (from_home[i], i));
    cells
}

/// Free cells within reach a building with these biomes may use, nearest to
/// the home first.
pub fn free_tiles(planet: &Planet, allowed: &[Biome]) -> Vec<usize> {
    buildable(planet)
        .into_iter()
        .filter(|&i| allowed.is_empty() || allowed.contains(&planet.tiles[i].biome))
        .collect()
}
