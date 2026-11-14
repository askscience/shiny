//! Things to do while playing: finds to pick up, goals to reach, building
//! sites to help, events that happen somewhere on the map.
mod common;

use common::{DAY, HOUR, T0};
use oorbit_sim::{advance, apply, rates, Action, Catalog, Planet};

#[test]
fn finds_turn_up_in_reach_wait_a_while_and_pay_when_picked_up() {
    let catalog = Catalog::builtin();
    let mut p = Planet::new(9, "temperate", T0, &catalog).unwrap();
    let mut seen = 0;
    let mut now = T0;
    while now < T0 + 3 * DAY && seen == 0 {
        now += HOUR;
        advance(&mut p, &catalog, now);
        seen = p.finds.len();
    }
    assert!(seen > 0, "something turned up within three days");
    let reach = oorbit_sim::orders::in_reach(&p);
    assert!(p.finds.iter().all(|f| reach[f.tile] && p.tiles[f.tile].building.is_none()));
    let find = p.finds[0].clone();
    let before = p.resources.clone();
    apply(&mut p, &catalog, now, &Action::Collect { id: find.id }).unwrap();
    assert!(p.resources.iter().any(|(k, v)| *v > before.get(k).copied().unwrap_or(0.0)), "it paid");
    assert_eq!(p.finds_collected, 1);
    assert!(apply(&mut p, &catalog, now, &Action::Collect { id: find.id }).is_err(), "only once");
    // Left alone, finds go.
    let left: Vec<u64> = p.finds.iter().map(|f| f.id).collect();
    advance(&mut p, &catalog, now + 9 * HOUR);
    assert!(left.iter().all(|id| !p.finds.iter().any(|f| f.id == *id)));
}

#[test]
fn goals_guide_and_reward() {
    let catalog = Catalog::builtin();
    let mut p = Planet::new(2, "arid", T0, &catalog).unwrap();
    let active = oorbit_sim::goals::active(&p, &catalog);
    assert_eq!(active.len(), 3);
    assert_eq!(active[0].id, "g_shelter");
    let wood = p.amount("wood");
    apply(&mut p, &catalog, T0, &Action::Research { tech: "shelter".into() }).unwrap();
    advance(&mut p, &catalog, T0 + HOUR);
    assert!(p.goals_done.contains("g_shelter"));
    assert!(p.chronicle.iter().any(|e| e.key == "goal_done"));
    assert!(p.amount("wood") > wood - 1.0, "the reward came");
    assert!(!oorbit_sim::goals::active(&p, &catalog).iter().any(|g| g.id == "g_shelter"));
}

#[test]
fn helping_a_site_halves_its_time_and_pauses_research() {
    let catalog = Catalog::builtin();
    let mut p = Planet::new(5, "temperate", T0, &catalog).unwrap();
    p.resources.insert("stone".into(), 50.0);
    p.resources.insert("wood".into(), 50.0);
    let tile = oorbit_sim::cities::sites_for(&p, &catalog, catalog.building("stone_circle").unwrap())[0];
    apply(&mut p, &catalog, T0, &Action::Build { building: "stone_circle".into(), tile }).unwrap();
    let done = p.construction[0].done_step;
    let left = done - p.step;
    apply(&mut p, &catalog, T0, &Action::Help { tile }).unwrap();
    assert_eq!(p.construction[0].done_step, done - left / 2);
    assert!(apply(&mut p, &catalog, T0, &Action::Help { tile }).is_err(), "once per site");
    advance(&mut p, &catalog, T0 + 600_000);
    let r = rates(&p, &catalog);
    assert_eq!(r.per_hour.get("knowledge").copied().unwrap_or(0.0), 0.0, "no knowledge while helping");
}

#[test]
fn some_events_happen_on_a_cell_of_their_biome() {
    let catalog = Catalog::builtin();
    for seed in 1..8u64 {
        let mut p = Planet::new(seed, "temperate", T0, &catalog).unwrap();
        let mut now = T0;
        while now < T0 + 20 * DAY {
            now += 6 * HOUR;
            advance(&mut p, &catalog, now);
            if let Some(e) = p.pending_event.clone() {
                let def = catalog.event(&e.id).unwrap();
                if let (Some(place), Some(tile)) = (def.place, e.tile) {
                    assert_eq!(p.tiles[tile].biome, place);
                    return;
                }
                let choice = def.choices[0].id.clone();
                apply(&mut p, &catalog, now, &Action::ResolveEvent { choice }).unwrap();
            }
        }
    }
    panic!("no event with a place happened");
}
