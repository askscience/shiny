//! The planets next door: they send ships, answer the player's, and trade.
mod common;

use common::{play_session, DAY, HOUR, T0};
use oorbit_sim::neighbors::ShipKind;
use oorbit_sim::{advance, apply, Action, Catalog, Planet};

fn keys(planet: &Planet) -> Vec<String> {
    planet.chronicle.iter().map(|e| e.key.clone()).collect()
}

#[test]
fn every_planet_has_three_neighbors_with_their_own_personality() {
    let catalog = Catalog::builtin();
    let planet = Planet::new(9, "temperate", T0, &catalog).unwrap();
    assert_eq!(planet.neighbors.len(), 3);
    let personas: Vec<&str> = planet
        .neighbors
        .iter()
        .map(|n| n.persona.as_str())
        .collect();
    assert_eq!(personas, ["generous", "trader", "prankster"]);
    assert!(planet.neighbors.iter().all(|n| n.climate != "temperate"));
}

#[test]
fn neighbors_send_ships_on_their_own() {
    let catalog = Catalog::builtin();
    let mut planet = Planet::new(9, "temperate", T0, &catalog).unwrap();
    for i in 0..(6 * 2) {
        play_session(&mut planet, &catalog, T0 + i * 12 * HOUR);
    }
    let arrivals = keys(&planet)
        .iter()
        .filter(|k| k.starts_with("ship_") && *k != "ship_sent" && *k != "ship_delivered")
        .count();
    assert!(
        arrivals >= 3,
        "a few ships arrived in six days: {:?}",
        keys(&planet)
    );
}

#[test]
fn a_gift_is_answered_with_a_gift() {
    let catalog = Catalog::builtin();
    let mut planet = Planet::new(5, "temperate", T0, &catalog).unwrap();
    planet.resources.insert("wood".into(), 50.0);
    let to = planet.neighbors[0].id.clone();
    let send = Action::Send {
        to: to.clone(),
        kind: ShipKind::Gift,
        resource: Some("wood".into()),
        amount: 5.0,
    };
    apply(&mut planet, &catalog, T0, &send).unwrap();
    assert!(planet.amount("wood") <= 45.0, "the cargo left the planet");
    assert!(planet
        .ships
        .iter()
        .any(|s| s.to == to && s.kind == ShipKind::Gift));
    // Stone Age ships take 12 hours each way.
    advance(&mut planet, &catalog, T0 + 13 * HOUR);
    assert!(keys(&planet).contains(&"ship_delivered".to_string()));
    assert_eq!(planet.neighbors[0].friendship, 1);
    advance(&mut planet, &catalog, T0 + 26 * HOUR);
    assert!(planet
        .chronicle
        .iter()
        .any(|e| e.key == "ship_gift" && e.params.get("from") == Some(&planet.neighbors[0].name)));
}

#[test]
fn too_much_cargo_or_a_locked_kind_is_refused() {
    let catalog = Catalog::builtin();
    let mut planet = Planet::new(5, "temperate", T0, &catalog).unwrap();
    planet.resources.insert("wood".into(), 500.0);
    let to = planet.neighbors[0].id.clone();
    let too_much = Action::Send {
        to: to.clone(),
        kind: ShipKind::Gift,
        resource: Some("wood".into()),
        amount: 400.0,
    };
    assert!(apply(&mut planet, &catalog, T0, &too_much).is_err());
    let visit = Action::Send {
        to,
        kind: ShipKind::Visit,
        resource: None,
        amount: 0.0,
    };
    assert!(
        apply(&mut planet, &catalog, T0, &visit).is_err(),
        "visits open in the Classical Age"
    );
}

#[test]
fn an_accepted_offer_is_delivered() {
    let catalog = Catalog::builtin();
    let mut planet = Planet::new(2, "temperate", T0, &catalog).unwrap();
    planet.era = 3; // traders sail from the Agricultural Age
    let mut now = T0;
    while planet.offers.is_empty() && now < T0 + 30 * DAY {
        now += 6 * HOUR;
        advance(&mut planet, &catalog, now);
    }
    let offer = planet
        .offers
        .first()
        .cloned()
        .expect("a trade offer within a month");
    planet
        .resources
        .insert(offer.want_resource.clone(), offer.want_amount + 10.0);
    let before = planet.amount(&offer.give_resource);
    apply(
        &mut planet,
        &catalog,
        now,
        &Action::AcceptOffer { id: offer.id },
    )
    .unwrap();
    assert!(planet.offers.is_empty());
    advance(&mut planet, &catalog, now + DAY);
    assert!(
        planet.amount(&offer.give_resource) > before,
        "the goods arrived"
    );
}
