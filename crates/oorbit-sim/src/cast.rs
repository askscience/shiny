//! The villagers with a name: a small cast (a few more as the planet grows)
//! who arrive or are born, fall in love and marry, tell their little
//! stories, grow old and die, some remembered by a stone or a statue near
//! the home. The planet's notable people are part of the cast.
//!
//! All of it is story: the cast never changes resources, mood or anything
//! the rest of the simulation reads, and draws its randomness from its own
//! generator, so the planet's economy runs exactly as without it.

use serde::{Deserialize, Serialize};

use crate::planet::Celebration;
use crate::rng::Rng;
use crate::{Catalog, Planet, STEPS_PER_HOUR};

/// The cast's own random stream (mixed into the planet seed).
const SALT: u64 = 0x5eed_c457_0f_f01c;
/// Steps in one planet day.
const DAY: i64 = 24 * STEPS_PER_HOUR;
/// Most people in the cast, and how it grows with the population.
pub const MAX_CAST: usize = 8;
/// A life, in planet days (a planet year is seven).
const LIFE_DAYS: (i64, i64) = (16, 30);
/// Children grow up after this many planet days.
pub const CHILD_DAYS: i64 = 3;
/// Most memorials kept (the oldest go).
pub const MAX_MEMORIALS: usize = 12;
/// Chances per planet hour.
const ARRIVAL: f64 = 0.08;
const LOVE: f64 = 0.03;
const ANECDOTE: f64 = 0.12;
/// A wedding party, in planet hours.
const WEDDING_HOURS: i64 = 4;

pub const QUIRKS: [&str; 8] = [
    "cheerful", "grumpy", "curious", "shy", "dreamer", "brave", "clumsy", "greedy",
];

/// The little stories, each with the quirks that make it likelier, and
/// whether it needs a second person.
const STORIES: [(&str, &[&str], bool); 14] = [
    ("goat_roof", &["clumsy", "curious"], false),
    ("song", &["cheerful", "dreamer"], true),
    ("lost", &["curious", "dreamer"], false),
    ("star", &["dreamer", "curious"], false),
    ("nap", &["dreamer", "greedy"], false),
    ("bet", &["clumsy", "brave"], true),
    ("pie", &["cheerful", "clumsy", "greedy"], false),
    ("quarrel", &["grumpy", "greedy"], true),
    ("peace", &["shy", "cheerful"], true),
    ("gift", &["shy", "cheerful"], true),
    ("wolf", &["brave"], false),
    ("invention", &["curious", "clumsy"], false),
    ("snore", &["grumpy", "greedy"], true),
    ("dance", &["cheerful", "shy"], true),
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Person {
    pub id: u64,
    pub name: String,
    pub female: bool,
    /// What they do: a category (food, materials, knowledge, culture).
    pub role: String,
    pub quirk: String,
    pub born_step: i64,
    pub until_step: i64,
    #[serde(default)]
    pub partner: Option<u64>,
    #[serde(default)]
    pub parents: Vec<u64>,
    /// One of the planet's notable people.
    #[serde(default)]
    pub famous: bool,
    /// Stories told about them.
    #[serde(default)]
    pub stories: u32,
}

/// A stone (or a statue, for the famous) for someone remembered.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Memorial {
    pub name: String,
    pub tile: usize,
    pub step: i64,
    pub famous: bool,
}

/// What happens to the cast at `step`.
pub(crate) fn run(planet: &mut Planet, catalog: &Catalog, step: i64) {
    let mut rng = Rng::for_step(planet.seed ^ SALT, step);
    follow_notables(planet, catalog, step, &mut rng);
    for p in planet.people.clone() {
        if p.until_step <= step {
            die(planet, catalog, step, &p, &mut rng);
        }
    }
    if let Some((a, b, at)) = planet.wedding {
        if step >= at && planet.celebration.is_none() {
            planet.wedding = None;
            if let (Some(pa), Some(pb)) = (person(planet, a), person(planet, b)) {
                let (na, nb) = (pa.name.clone(), pb.name.clone());
                planet.log(catalog, step, "wedding", &[("name", &na), ("other", &nb)]);
                planet.celebration = Some(Celebration {
                    id: "wedding".into(),
                    from_step: step,
                    until_step: step + WEDDING_HOURS * STEPS_PER_HOUR,
                });
            }
        }
    }
    if step.rem_euclid(STEPS_PER_HOUR) != 0 {
        return;
    }
    let target = (3 + (planet.population / 25.0) as usize).min(MAX_CAST);
    let few = planet.people.len() < 2;
    if planet.people.len() < target && rng.chance(if few { 0.3 } else { ARRIVAL }) {
        arrive(planet, catalog, step, &mut rng);
    }
    if planet.wedding.is_none() && rng.chance(LOVE) {
        fall_in_love(planet, catalog, step, &mut rng);
    }
    if rng.chance(ANECDOTE) {
        anecdote(planet, catalog, step, &mut rng);
    }
}

fn person(planet: &Planet, id: u64) -> Option<&Person> {
    planet.people.iter().find(|p| p.id == id)
}

fn adult(p: &Person, step: i64) -> bool {
    step - p.born_step >= CHILD_DAYS * DAY
}

fn new_person(planet: &mut Planet, rng: &mut Rng, step: i64, role: &str) -> Person {
    planet.next_person += 1;
    let life = LIFE_DAYS.0 + rng.below((LIFE_DAYS.1 - LIFE_DAYS.0) as usize) as i64;
    Person {
        id: planet.next_person,
        name: crate::step::notable_name(rng),
        female: rng.chance(0.5),
        role: role.to_string(),
        quirk: QUIRKS[rng.below(QUIRKS.len())].to_string(),
        born_step: step,
        until_step: step + life * DAY,
        partner: None,
        parents: Vec::new(),
        famous: false,
        stories: 0,
    }
}

/// Someone new: the child of a couple, or a newcomer.
fn arrive(planet: &mut Planet, catalog: &Catalog, step: i64, rng: &mut Rng) {
    let couples: Vec<(u64, u64)> = planet
        .people
        .iter()
        .filter_map(|p| p.partner.filter(|&q| p.id < q).map(|q| (p.id, q)))
        .collect();
    let role = crate::catalog::Category::ALL[rng.below(4)].as_str();
    let mut p = new_person(planet, rng, step, role);
    if !couples.is_empty() && rng.chance(0.6) {
        let (a, b) = couples[rng.below(couples.len())];
        let (Some(pa), Some(pb)) = (person(planet, a), person(planet, b)) else {
            return;
        };
        let (na, nb) = (pa.name.clone(), pb.name.clone());
        // A child takes after one parent's trade.
        p.role = if rng.chance(0.5) {
            pa.role.clone()
        } else {
            pb.role.clone()
        };
        p.parents = vec![a, b];
        let (name, girl) = (p.name.clone(), if p.female { "1" } else { "0" });
        planet.people.push(p);
        planet.log(
            catalog,
            step,
            "person_born",
            &[("name", &name), ("a", &na), ("b", &nb), ("female", girl)],
        );
    } else {
        let (name, role, quirk, female) =
            (p.name.clone(), p.role.clone(), p.quirk.clone(), p.female);
        // Newcomers arrive grown up.
        p.born_step = step - CHILD_DAYS * DAY;
        planet.people.push(p);
        planet.log(
            catalog,
            step,
            "person_arrived",
            &[
                ("name", &name),
                ("role", &role),
                ("quirk", &quirk),
                ("female", if female { "1" } else { "0" }),
            ],
        );
    }
}

/// Two grown-ups without a partner fall in love; the wedding is the next
/// evening.
fn fall_in_love(planet: &mut Planet, catalog: &Catalog, step: i64, rng: &mut Rng) {
    let free: Vec<usize> = (0..planet.people.len())
        .filter(|&k| planet.people[k].partner.is_none() && adult(&planet.people[k], step))
        .collect();
    if free.len() < 2 {
        return;
    }
    let i = free[rng.below(free.len())];
    let others: Vec<usize> = free
        .iter()
        .copied()
        .filter(|&k| k != i && !siblings(&planet.people[i], &planet.people[k]))
        .collect();
    if others.is_empty() {
        return;
    }
    let j = others[rng.below(others.len())];
    let (a, b) = (planet.people[i].id, planet.people[j].id);
    planet.people[i].partner = Some(b);
    planet.people[j].partner = Some(a);
    let (na, nb) = (planet.people[i].name.clone(), planet.people[j].name.clone());
    planet.log(catalog, step, "love", &[("name", &na), ("other", &nb)]);
    let evening = step - step.rem_euclid(DAY) + 18 * STEPS_PER_HOUR + DAY;
    planet.wedding = Some((a, b, evening));
}

fn siblings(a: &Person, b: &Person) -> bool {
    a.parents.iter().any(|p| b.parents.contains(p))
        || a.parents.contains(&b.id)
        || b.parents.contains(&a.id)
}

/// A little story about someone, picked by their quirk.
fn anecdote(planet: &mut Planet, catalog: &Catalog, step: i64, rng: &mut Rng) {
    let grown: Vec<usize> = (0..planet.people.len())
        .filter(|&k| adult(&planet.people[k], step))
        .collect();
    if grown.is_empty() {
        return;
    }
    let i = grown[rng.below(grown.len())];
    let quirk = planet.people[i].quirk.clone();
    let alone = grown.len() < 2;
    let weights: Vec<u32> = STORIES
        .iter()
        .map(|(_, quirks, two)| {
            if *two && alone {
                0
            } else if quirks.contains(&quirk.as_str()) {
                5
            } else {
                1
            }
        })
        .collect();
    let Some(s) = rng.weighted(&weights) else {
        return;
    };
    let (story, _, two) = STORIES[s];
    let name = planet.people[i].name.clone();
    let mut params = vec![("story", story.to_string()), ("name", name)];
    if two {
        let others: Vec<usize> = grown.iter().copied().filter(|&k| k != i).collect();
        let j = others[rng.below(others.len())];
        params.push(("other", planet.people[j].name.clone()));
        planet.people[j].stories += 1;
    }
    planet.people[i].stories += 1;
    let refs: Vec<(&str, &str)> = params.iter().map(|(k, v)| (*k, v.as_str())).collect();
    planet.log(catalog, step, "anecdote", &refs);
}

/// The notable people are in the cast: a new one joins it (famous), and
/// one who is gone leaves it.
fn follow_notables(planet: &mut Planet, catalog: &Catalog, step: i64, rng: &mut Rng) {
    let notables = planet.notables.clone();
    for n in &notables {
        if planet.people.iter().any(|p| p.famous && p.name == n.name) {
            continue;
        }
        let mut p = new_person(planet, rng, step, n.category.as_str());
        p.name = n.name.clone();
        p.famous = true;
        p.born_step = n.born_step;
        p.until_step = n.until_step.max(step + 1);
        planet.people.push(p);
    }
    for p in planet.people.clone() {
        if p.famous && !notables.iter().any(|n| n.name == p.name) {
            // The Chronicle already says goodbye (`notable_gone`).
            remember(planet, step, &p, rng);
            remove(planet, p.id);
        }
    }
    let _ = catalog;
}

fn die(planet: &mut Planet, catalog: &Catalog, step: i64, p: &Person, rng: &mut Rng) {
    if p.famous {
        // A notable person leaves with the notables (`follow_notables`).
        return;
    }
    let remembered = p.stories >= 2;
    planet.log(
        catalog,
        step,
        "person_died",
        &[
            ("name", &p.name),
            ("remembered", if remembered { "1" } else { "0" }),
        ],
    );
    if remembered {
        remember(planet, step, p, rng);
    }
    remove(planet, p.id);
}

fn remove(planet: &mut Planet, id: u64) {
    planet.people.retain(|p| p.id != id);
    for p in planet.people.iter_mut() {
        if p.partner == Some(id) {
            p.partner = None;
        }
    }
    if planet.wedding.is_some_and(|(a, b, _)| a == id || b == id) {
        planet.wedding = None;
    }
}

/// A stone (a statue for the famous) on free land near the home.
fn remember(planet: &mut Planet, step: i64, p: &Person, rng: &mut Rng) {
    let free = crate::step::buildable(planet);
    let tile = if free.is_empty() {
        planet.home
    } else {
        free[rng.below(free.len().min(6))]
    };
    planet.memorials.push(Memorial {
        name: p.name.clone(),
        tile,
        step,
        famous: p.famous,
    });
    if planet.memorials.len() > MAX_MEMORIALS {
        planet.memorials.remove(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cast_lives_its_life_without_touching_the_economy() {
        let catalog = Catalog::builtin();
        let mut planet = Planet::new(3, "temperate", 0, &catalog).unwrap();
        planet.population = 60.0;
        let before = (planet.resources.clone(), planet.mood);
        let mut keys = std::collections::BTreeSet::new();
        for step in 1..(80 * DAY) {
            run(&mut planet, &catalog, step);
            if planet
                .celebration
                .as_ref()
                .is_some_and(|c| step >= c.until_step)
            {
                planet.celebration = None;
            }
            for e in planet.chronicle.drain(..) {
                keys.insert(e.key);
            }
        }
        assert_eq!((planet.resources.clone(), planet.mood), before);
        for key in [
            "person_arrived",
            "love",
            "wedding",
            "person_born",
            "anecdote",
            "person_died",
        ] {
            assert!(keys.contains(key), "no {key} in {keys:?}");
        }
        assert!(planet.people.len() <= MAX_CAST + 2);
        assert!(!planet.memorials.is_empty());
    }
}
