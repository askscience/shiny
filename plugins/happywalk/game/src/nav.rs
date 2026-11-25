//! `NavPlugin`: "go to the bakery". An order naming a place sets a route
//! along the streets to the nearest one, and the character walks it by
//! itself until it arrives, is told something else, or walks into
//! something.
//!
//! Routes follow the street grid: down the street the character is on to
//! the right crossing, then along the street the place is on. They are
//! plain lists of points, worked out by a function with no engine in it.

use std::collections::VecDeque;

use bevy::prelude::*;

use crate::city::{CitySeed, PITCH};
use crate::gen::{self, PlaceKind};
use crate::intent::Order;
use crate::player::{Bumped, Player, PlayerSet};

/// The character reached the place it was walking to.
#[derive(Message, Clone, Copy, Debug)]
pub struct Arrived {
    pub kind: PlaceKind,
}

/// A place was asked for and there is none near enough.
#[derive(Message, Clone, Copy, Debug)]
pub struct NoSuchPlace {
    pub kind: PlaceKind,
}

/// Where the character is walking to by itself, if anywhere.
#[derive(Resource, Default, Debug)]
pub struct Route {
    pub to: Option<PlaceKind>,
    points: VecDeque<Vec2>,
}

/// How many chunks around the character a place is looked for in.
pub const REACH: i32 = 8;
/// A point of the route is reached within this many metres.
const CLOSE: f32 = 0.6;

/// The middle line of the street nearest to a coordinate.
pub fn street(v: f32) -> f32 {
    ((v / PITCH - 0.5).round() + 0.5) * PITCH
}

/// The way from `from` to `to` along the streets: the points to walk
/// through, the last being `to`. Both are taken to the nearest street first.
pub fn route(from: Vec2, to: Vec2) -> Vec<Vec2> {
    // Is a point on a street that runs along `z` (at a fixed `x`)?
    let along_z = |p: Vec2| (p.x - street(p.x)).abs() <= (p.y - street(p.y)).abs();
    let onto = |p: Vec2| {
        if along_z(p) {
            Vec2::new(street(p.x), p.y)
        } else {
            Vec2::new(p.x, street(p.y))
        }
    };
    let start = onto(from);
    let end = onto(to);
    let mut points = vec![start];
    let mut go = |p: Vec2| {
        if points.last().is_none_or(|last| last.distance(p) > 0.01) {
            points.push(p);
        }
    };
    match (along_z(from), along_z(to)) {
        (true, false) => go(Vec2::new(start.x, end.y)),
        (false, true) => go(Vec2::new(end.x, start.y)),
        (true, true) if (start.x - end.x).abs() > 0.01 => {
            // Two streets side by side: across on the nearest cross street.
            let cross = street(start.y);
            go(Vec2::new(start.x, cross));
            go(Vec2::new(end.x, cross));
        }
        (false, false) if (start.y - end.y).abs() > 0.01 => {
            let cross = street(start.x);
            go(Vec2::new(cross, start.y));
            go(Vec2::new(cross, end.y));
        }
        _ => {}
    }
    go(end);
    go(to);
    points
}

/// The length of a route from `from`.
pub fn length(from: Vec2, points: &[Vec2]) -> f32 {
    let mut total = 0.0;
    let mut at = from;
    for point in points {
        total += at.distance(*point);
        at = *point;
    }
    total
}

pub struct NavPlugin;

impl Plugin for NavPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Route>()
            .add_message::<Arrived>()
            .add_message::<NoSuchPlace>()
            .add_systems(Update, (set_out, steer).chain().before(PlayerSet))
            .add_systems(Update, give_up.after(PlayerSet));
    }
}

/// An order naming a place starts a route; any other order about where to
/// go ends it.
fn set_out(
    seed: Res<CitySeed>,
    mut orders: MessageReader<Order>,
    players: Query<&Transform, With<Player>>,
    mut route: ResMut<Route>,
    mut none: MessageWriter<NoSuchPlace>,
) {
    let Ok(player) = players.single() else { return };
    let at = Vec2::new(player.translation.x, player.translation.z);
    for order in orders.read() {
        match order {
            Order::GoTo(kind) => match gen::nearest_place(seed.0, at, *kind, REACH) {
                Some(place) => {
                    route.to = Some(*kind);
                    route.points = route_points(at, place.at);
                }
                None => {
                    none.write(NoSuchPlace { kind: *kind });
                }
            },
            Order::Forward | Order::Back | Order::Left | Order::Right | Order::Stop => {
                *route = Route::default()
            }
            Order::Faster | Order::Slower | Order::Wave => {}
        }
    }
}

fn route_points(from: Vec2, to: Vec2) -> VecDeque<Vec2> {
    route(from, to).into()
}

/// Walk the route: face the next point, keep walking, tick points off.
fn steer(
    mut route: ResMut<Route>,
    mut players: Query<(&mut Player, &Transform)>,
    mut arrived: MessageWriter<Arrived>,
) {
    let Some(kind) = route.to else { return };
    let Ok((mut player, transform)) = players.single_mut() else {
        return;
    };
    let at = Vec2::new(transform.translation.x, transform.translation.z);
    while route
        .points
        .front()
        .is_some_and(|next| next.distance(at) < CLOSE)
    {
        route.points.pop_front();
    }
    let Some(next) = route.points.front().copied() else {
        *route = Route::default();
        player.walking = false;
        arrived.write(Arrived { kind });
        return;
    };
    // The heading nearest to where it faces now, so it never spins round.
    let towards = (-(next.x - at.x)).atan2(-(next.y - at.y));
    let turn = (towards - player.facing + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
        - std::f32::consts::PI;
    player.heading = player.facing + turn;
    player.walking = true;
}

/// Walking into something ends the walk: the character stops there.
fn give_up(mut bumps: MessageReader<Bumped>, mut route: ResMut<Route>) {
    if bumps.read().count() > 0 {
        *route = Route::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::city::Solids;

    /// Every leg of a route runs along one axis, on a street line.
    fn follows_streets(from: Vec2, points: &[Vec2]) {
        let mut at = points[0];
        // The first point is the nearest street; the last leg leaves the
        // middle line to reach the place itself.
        for point in &points[1..points.len() - 1] {
            assert!(
                (point.x - at.x).abs() < 0.01 || (point.y - at.y).abs() < 0.01,
                "{at:?} to {point:?} is not along a street"
            );
            let on_street = (point.x - street(point.x)).abs() < 0.01
                || (point.y - street(point.y)).abs() < 0.01;
            assert!(on_street, "{point:?} from {from:?}");
            at = *point;
        }
    }

    #[test]
    fn routes_follow_the_streets() {
        let from = Vec2::new(11.0, 11.0);
        for to in [
            Vec2::new(40.0, 32.0),
            Vec2::new(-4.0, -12.0),
            Vec2::new(10.0, -60.0),
            Vec2::new(-55.0, 12.0),
            Vec2::new(32.0, 40.0),
        ] {
            let points = route(from, to);
            assert_eq!(*points.last().unwrap(), to);
            follows_streets(from, &points);
            // Never much longer than walking round the blocks would be.
            let direct = (to - from).abs();
            assert!(
                length(from, &points) <= direct.x + direct.y + 2.0 * PITCH,
                "{to:?}: {points:?}"
            );
        }
    }

    #[test]
    fn a_route_to_every_kind_of_place_is_clear_of_the_city() {
        for seed in [1u64, 2026] {
            let city = Solids::new(seed);
            let from = Vec2::new(11.0, 11.0);
            for kind in PlaceKind::ALL {
                let place =
                    gen::nearest_place(seed, from, kind, REACH).expect("a place of every kind");
                let points = route(from, place.at);
                let mut at = from;
                for point in &points {
                    let steps = (at.distance(*point) / 0.3).ceil().max(1.0) as usize;
                    for i in 0..=steps {
                        let p = at.lerp(*point, i as f32 / steps as f32);
                        assert_eq!(city.hit(p, 0.38), None, "seed {seed}, to {kind:?} at {p:?}");
                    }
                    at = *point;
                }
            }
        }
    }

    #[test]
    fn standing_at_the_place_is_no_walk() {
        let here = Vec2::new(11.0, 4.0);
        let points = route(here, here);
        assert!(length(here, &points) < 0.01);
    }
}
