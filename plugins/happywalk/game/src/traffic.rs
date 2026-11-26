//! `TrafficPlugin`: cars and bikes. They keep to the right-hand lane of the
//! streets, give way at crossings, and **always stop** for the character:
//! being in the road gets a hoot, never a hit. Walking into one is a bump
//! like any other.
//!
//! Traffic is not part of the city's seed: a handful of vehicles is kept
//! around the player, each new one arriving from beyond the fog.

use std::f32::consts::{FRAC_PI_2, PI};

use bevy::prelude::*;

use crate::city::{CitySeed, PITCH};
use crate::figure::{self, Look};
use crate::gen::SolidKind;
use crate::hud::OtherSaid;
use crate::movers::{Dice, Mover, Movers};
use crate::player::{Player, PlayerSet, RADIUS};
use crate::vox::VoxLibrary;

/// A car had to stop because the character is in its way.
#[derive(Message, Clone, Copy, Debug)]
pub struct Hooted {
    pub by: SolidKind,
}

/// How many vehicles are kept around the player.
const CARS: usize = 4;
const BIKES: usize = 2;
/// New vehicles appear this far along a street (beyond the fog), and are
/// taken away past [`GONE`].
const ARRIVE: f32 = 58.0;
const GONE: f32 = 66.0;
/// How far from the middle of the street a lane's middle is.
const LANE: f32 = 1.5;
const VOXEL: f32 = 0.1;
/// A bicycle is drawn finer than a car.
const BIKE_VOXEL: f32 = 0.05;
/// Where a cyclist sits, from the middle of the bicycle's base.
const SADDLE: Vec3 = Vec3::new(0.0, 0.65, 0.175);
/// The crank's axle (the bottom bracket), and how long a crank is.
const BRACKET: Vec3 = Vec3::new(0.0, 0.275, 0.025);
const CRANK: f32 = 0.1;
/// How far apart the feet are, each side of the middle.
const STANCE: f32 = 0.1;
/// A cyclist's thigh and shin: long enough to reach the pedal at the bottom
/// of its turn with the knee still a little bent.
const THIGH: f32 = 0.25;
const SHIN: f32 = 0.27;
/// How far a cyclist leans forward, in radians.
const LEAN: f32 = 0.7;
/// Where a cyclist's hands hold the handlebars (left, right).
const GRIPS: [Vec3; 2] = [
    Vec3::new(-0.17, 0.725, -0.475),
    Vec3::new(0.17, 0.725, -0.475),
];

/// A cyclist's leg piece (thigh or shin) or crank, on one side.
#[derive(Component, Clone, Copy)]
enum Pedalling {
    Thigh(f32),
    Shin(f32),
    Crank(f32),
}

/// Where a crank's pedal is on `side`, `turn` radians into its circle (0 at
/// the top, going forward, the two sides half a turn apart).
pub fn pedal_at(turn: f32, side: f32) -> Vec3 {
    let turn = if side > 0.0 {
        turn
    } else {
        turn + std::f32::consts::PI
    };
    BRACKET + Vec3::new(side * STANCE, CRANK * turn.cos(), -CRANK * turn.sin())
}

/// The knee of a leg from `hip` to `foot` with a thigh and a shin of these
/// lengths, bent forward (towards `-z`). Out of reach, the leg is as straight
/// (or as folded) as it goes.
pub fn knee(hip: Vec3, foot: Vec3, thigh: f32, shin: f32) -> Vec3 {
    let towards = foot - hip;
    let reach = towards
        .length()
        .clamp((thigh - shin).abs() + 1e-3, thigh + shin - 1e-3);
    let along = towards.normalize_or(Vec3::NEG_Y);
    // The angle at the hip between the line to the foot and the thigh.
    let bend = ((thigh * thigh + reach * reach - shin * shin) / (2.0 * thigh * reach))
        .clamp(-1.0, 1.0)
        .acos();
    hip + Quat::from_rotation_x(bend) * along * thigh
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Axis {
    /// Along `x` (a street at a fixed `z`).
    X,
    /// Along `z` (a street at a fixed `x`).
    Z,
}

#[derive(Component, Clone, Copy, Debug)]
pub struct Vehicle {
    pub kind: SolidKind,
    pub axis: Axis,
    /// The street's line: its `z` for an `X` street, its `x` for a `Z` one.
    pub street: f32,
    /// +1 or -1 along the axis.
    pub way: f32,
    /// Where it is along the axis.
    pub along: f32,
    pub speed: f32,
    pub cruise: f32,
    /// Seconds since it last hooted (it does not lean on the horn).
    quiet: f32,
    /// The model it is drawn with.
    pub model: &'static str,
    /// Where a cyclist's pedals are in their turn.
    pedal: f32,
}

impl Vehicle {
    /// Half its width and half its length.
    pub fn half(&self) -> (f32, f32) {
        match self.kind {
            SolidKind::Bike => (0.3, 0.9),
            _ => (0.9, 1.8),
        }
    }

    /// The middle of its lane: traffic keeps to the right.
    pub fn lane(&self) -> f32 {
        // Going `-z` the right-hand side is `+x`; going `+x` it is `+z`.
        let right = match self.axis {
            Axis::Z => -self.way,
            Axis::X => self.way,
        };
        self.street + right * LANE
    }

    pub fn at(&self) -> Vec2 {
        match self.axis {
            Axis::X => Vec2::new(self.along, self.lane()),
            Axis::Z => Vec2::new(self.lane(), self.along),
        }
    }

    /// Radians round the vertical; 0 looks down `-z`.
    pub fn facing(&self) -> f32 {
        match (self.axis, self.way > 0.0) {
            (Axis::Z, false) => 0.0,
            (Axis::Z, true) => PI,
            (Axis::X, false) => FRAC_PI_2,
            (Axis::X, true) => -FRAC_PI_2,
        }
    }

    pub fn mover(&self) -> Mover {
        let (wide, long) = self.half();
        let half = match self.axis {
            Axis::X => Vec2::new(long, wide),
            Axis::Z => Vec2::new(wide, long),
        };
        Mover {
            at: self.at(),
            half,
            kind: self.kind,
            model: self.model,
        }
    }

    /// How far ahead of its nose a box is, if the box is in its lane.
    pub fn gap_to(&self, at: Vec2, half: Vec2) -> Option<f32> {
        let (wide, long) = self.half();
        let (along, across, half_along, half_across) = match self.axis {
            Axis::X => (at.x, at.y, half.x, half.y),
            Axis::Z => (at.y, at.x, half.y, half.x),
        };
        if (across - self.lane()).abs() >= wide + half_across {
            return None;
        }
        let ahead = (along - self.along) * self.way - long - half_along;
        (ahead > -(long + half_along)).then_some(ahead)
    }

    /// One step of `dt` seconds with `gap` metres free ahead (`None`: the
    /// road is clear). It brakes to stand a little short of what is ahead
    /// and never moves into it.
    pub fn drive(&mut self, dt: f32, gap: Option<f32>) {
        const KEEP: f32 = 1.2;
        let want = match gap {
            Some(gap) if gap < KEEP + 0.2 => 0.0,
            // Slower the nearer: it can always stop in the room it has.
            Some(gap) => self.cruise.min((gap - KEEP) * 1.6),
            None => self.cruise,
        };
        let change = if want < self.speed { 14.0 } else { 3.0 } * dt;
        self.speed += (want - self.speed).clamp(-change, change);
        // About one turn of the pedals for every four metres.
        self.pedal += self.speed * dt * 1.6;
        let mut step = self.speed * dt;
        if let Some(gap) = gap {
            step = step.min((gap - 0.05).max(0.0));
        }
        self.along += self.way * step;
    }
}

/// The street lines nearest to a coordinate.
fn street_near(v: f32, offset: i32) -> f32 {
    ((v / PITCH - 0.5).round() + 0.5 + offset as f32) * PITCH
}

#[derive(Resource)]
struct Garage(Dice);

pub struct TrafficPlugin;

impl Plugin for TrafficPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<Hooted>()
            .add_systems(Startup, open_garage)
            .add_systems(Update, (arrivals, drive, pedal).chain().before(PlayerSet));
    }
}

fn open_garage(mut commands: Commands, seed: Res<CitySeed>) {
    commands.insert_resource(Garage(Dice(seed.0 ^ 0x7AFF_1C5E_ED00_0001 | 1)));
}

/// Keep the streets around the player busy; take away what drove off.
fn arrivals(
    mut commands: Commands,
    library: Res<VoxLibrary>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut garage: ResMut<Garage>,
    players: Query<&Transform, With<Player>>,
    vehicles: Query<(Entity, &Vehicle)>,
) {
    let Ok(player) = players.single() else { return };
    let player = Vec2::new(player.translation.x, player.translation.z);
    let (mut cars, mut bikes) = (0, 0);
    for (entity, vehicle) in &vehicles {
        let (along, across) = match vehicle.axis {
            Axis::X => (player.x, player.y),
            Axis::Z => (player.y, player.x),
        };
        if (vehicle.along - along).abs() > GONE || (vehicle.street - across).abs() > GONE {
            commands.entity(entity).despawn();
        } else if vehicle.kind == SolidKind::Bike {
            bikes += 1;
        } else {
            cars += 1;
        }
    }
    // One arrival a frame is plenty.
    let kind = if cars < CARS {
        SolidKind::Car
    } else if bikes < BIKES {
        SolidKind::Bike
    } else {
        return;
    };
    let dice = &mut garage.0;
    let axis = if dice.below(2) == 0 { Axis::X } else { Axis::Z };
    let (along, across) = match axis {
        Axis::X => (player.x, player.y),
        Axis::Z => (player.y, player.x),
    };
    let way = if dice.below(2) == 0 { 1.0 } else { -1.0 };
    let (model, cruise) = match kind {
        SolidKind::Bike => (
            ["bike_a", "bike_b", "bike_c"][dice.below(3) as usize],
            3.2 + dice.unit() * 1.2,
        ),
        _ => (
            ["car_red", "car_blue", "car_cream"][dice.below(3) as usize],
            5.0 + dice.unit() * 2.5,
        ),
    };
    let vehicle = Vehicle {
        kind,
        axis,
        street: street_near(across, dice.below(3) as i32 - 1),
        way,
        // It comes towards the player from beyond the fog.
        along: along - way * (ARRIVE + dice.unit() * 4.0),
        speed: cruise,
        cruise,
        quiet: 10.0,
        model,
        pedal: 0.0,
    };
    // Not on top of one already there.
    if vehicles
        .iter()
        .any(|(_, other)| other.mover().touches(vehicle.at(), 4.0))
    {
        return;
    }
    let at = vehicle.at();
    if kind != SolidKind::Bike {
        commands.spawn((
            vehicle,
            library.place(model, Vec3::new(at.x, 0.0, at.y), VOXEL, 0),
        ));
        return;
    }
    // A cyclist is a person like the passers-by, sat on the saddle.
    let look = Look::random(((dice.roll() as u64) << 32) | dice.roll() as u64);
    let voxel = look.voxel();
    let piece = |mesh: Handle<Mesh>, at: Vec3| {
        (
            Mesh3d(mesh),
            MeshMaterial3d(library.material.clone()),
            Transform::from_translation(at).with_scale(Vec3::splat(voxel)),
        )
    };
    let arm_model = look.arm();
    let arm_length = arm_model.size.y as f32 * voxel;
    let (body, arm) = (
        meshes.add(look.body_without_arms().mesh()),
        meshes.add(arm_model.mesh()),
    );
    commands
        .spawn((
            vehicle,
            Transform::from_xyz(at.x, 0.0, at.y),
            Visibility::default(),
        ))
        .with_children(|bike| {
            bike.spawn(library.place(model, Vec3::ZERO, BIKE_VOXEL, 0));
            // Leaning well over the handlebars, sat on the saddle.
            let lean = Quat::from_rotation_x(-LEAN);
            let seat_at = Vec3::new(0.0, SADDLE.y, SADDLE.z + 2.0 * voxel);
            let mut seat = piece(body, seat_at);
            seat.2.rotation = lean;
            bike.spawn(seat);
            // Each arm from its shoulder to its grip: turned to point there,
            // and as long as it takes (a voxel figure's arms are short).
            for (shoulder, grip) in figure::SHOULDERS.iter().zip(GRIPS) {
                let shoulder = seat_at + lean * (*shoulder * voxel);
                let reach = grip - shoulder;
                let mut hold = piece(arm.clone(), grip);
                hold.2.rotation = Quat::from_rotation_arc(Vec3::Y, -reach.normalize());
                hold.2.scale.y *= (reach.length() / arm_length).clamp(0.8, 1.5);
                bike.spawn(hold);
            }
            // Legs in two pieces each, and the cranks they turn: placed every
            // frame by `pedal`.
            let (thigh, shin) = (
                meshes.add(look.thigh(THIGH).mesh()),
                meshes.add(look.shin(SHIN).mesh()),
            );
            for side in [-1.0, 1.0] {
                bike.spawn((Pedalling::Thigh(side), piece(thigh.clone(), SADDLE)));
                bike.spawn((Pedalling::Shin(side), piece(shin.clone(), SADDLE)));
                bike.spawn((
                    Pedalling::Crank(side),
                    library.place("bike_crank", BRACKET, BIKE_VOXEL, 0),
                ));
            }
        });
}

/// Everything drives on, braking for what is ahead and giving way.
fn drive(
    time: Res<Time>,
    players: Query<&Transform, (With<Player>, Without<Vehicle>)>,
    mut vehicles: Query<(&mut Vehicle, &mut Transform)>,
    mut movers: ResMut<Movers>,
    mut hooted: MessageWriter<Hooted>,
    mut said: MessageWriter<OtherSaid>,
) {
    let dt = time.delta_secs().min(0.05);
    let player = players
        .single()
        .map(|t| Vec2::new(t.translation.x, t.translation.z))
        .ok();
    let before: Vec<Vehicle> = vehicles.iter().map(|(vehicle, _)| *vehicle).collect();
    movers.vehicles.clear();
    for (index, (mut vehicle, mut transform)) in vehicles.iter_mut().enumerate() {
        let mut gap: Option<f32> = None;
        let mut nearer = |g: Option<f32>| {
            if let Some(g) = g {
                gap = Some(gap.map_or(g, |old: f32| old.min(g)));
            }
        };
        for (other_index, other) in before.iter().enumerate() {
            if other_index == index {
                continue;
            }
            let mover = other.mover();
            if other.axis == vehicle.axis {
                nearer(vehicle.gap_to(mover.at, mover.half));
            } else if vehicle.axis == Axis::X {
                // At a crossing the streets along `x` give way: anything on
                // the other street near the crossing counts as in the way.
                let crossing = Vec2::new(other.street, vehicle.lane());
                if (mover.at.y - vehicle.street).abs() < 9.0 {
                    nearer(vehicle.gap_to(crossing, Vec2::splat(2.6)));
                }
            }
        }
        // The character: always stopped for, with room to spare.
        let for_player = player.and_then(|at| vehicle.gap_to(at, Vec2::splat(RADIUS + 0.5)));
        nearer(for_player);
        vehicle.quiet += dt;
        if for_player.is_some_and(|g| g < 3.5) && vehicle.speed > 0.6 && vehicle.quiet > 6.0 {
            vehicle.quiet = 0.0;
            hooted.write(Hooted { by: vehicle.kind });
            let at = vehicle.at();
            let text = if vehicle.kind == SolidKind::Bike {
                "DRIN DRIN!"
            } else {
                "BEEP!"
            };
            said.write(OtherSaid {
                text: text.into(),
                at: Vec3::new(at.x, 2.0, at.y),
            });
        }
        vehicle.drive(dt, gap);
        let at = vehicle.at();
        transform.translation = Vec3::new(at.x, 0.0, at.y);
        transform.rotation = Quat::from_rotation_y(vehicle.facing());
        movers.vehicles.push(vehicle.mover());
    }
    // Where everything is, for tests and screenshots.
    let list: Vec<String> = movers
        .vehicles
        .iter()
        .map(|v| {
            format!(
                r#"{{"model":"{}","x":{:.2},"z":{:.2}}}"#,
                v.model, v.at.x, v.at.y
            )
        })
        .collect();
    crate::bridge::publish_traffic(format!("[{}]", list.join(",")));
}

/// Cyclists' legs go round with the pedals.
fn pedal(vehicles: Query<(&Vehicle, &Children)>, mut parts: Query<(&Pedalling, &mut Transform)>) {
    for (vehicle, children) in &vehicles {
        for child in children.iter() {
            let Ok((part, mut transform)) = parts.get_mut(child) else {
                continue;
            };
            let side = match *part {
                Pedalling::Thigh(side) | Pedalling::Shin(side) | Pedalling::Crank(side) => side,
            };
            let foot = pedal_at(vehicle.pedal, side);
            let hip = Vec3::new(side * STANCE, SADDLE.y, SADDLE.z);
            let knee = knee(hip, foot, THIGH, SHIN);
            // A piece's model stands on its base and rises along `+y`: put
            // the base at one end and turn `+y` towards the other.
            let (base, top) = match *part {
                Pedalling::Thigh(_) => (knee, hip),
                Pedalling::Shin(_) => (foot, knee),
                Pedalling::Crank(_) => (Vec3::new(side * STANCE, BRACKET.y, BRACKET.z), foot),
            };
            transform.translation = base;
            transform.rotation =
                Quat::from_rotation_arc(Vec3::Y, (top - base).normalize_or(Vec3::Y));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn car(axis: Axis, way: f32) -> Vehicle {
        Vehicle {
            kind: SolidKind::Car,
            axis,
            street: 11.0,
            way,
            along: 0.0,
            speed: 6.0,
            cruise: 6.0,
            quiet: 10.0,
            model: "car_red",
            pedal: 0.0,
        }
    }

    #[test]
    fn traffic_keeps_to_the_right() {
        // Going up the street (`-z`), the right is `+x`.
        assert!(car(Axis::Z, -1.0).lane() > 11.0);
        assert!(car(Axis::Z, 1.0).lane() < 11.0);
        // Going `+x`, the right is `+z`.
        assert!(car(Axis::X, 1.0).lane() > 11.0);
        assert!(car(Axis::X, -1.0).lane() < 11.0);
    }

    #[test]
    fn a_vehicle_faces_the_way_it_drives() {
        for (axis, way) in [
            (Axis::Z, -1.0),
            (Axis::Z, 1.0),
            (Axis::X, -1.0),
            (Axis::X, 1.0),
        ] {
            let mut vehicle = car(axis, way);
            let from = vehicle.at();
            let ahead = Vec2::new(-vehicle.facing().sin(), -vehicle.facing().cos());
            vehicle.drive(0.1, None);
            assert!(
                (vehicle.at() - from).normalize().dot(ahead) > 0.99,
                "{axis:?} {way}"
            );
        }
    }

    #[test]
    fn the_middle_of_the_street_is_not_in_a_lane() {
        // The character walking down the middle line is in nobody's way.
        let vehicle = car(Axis::Z, -1.0);
        assert_eq!(
            vehicle.gap_to(Vec2::new(11.0, -20.0), Vec2::splat(RADIUS)),
            None
        );
        assert!(vehicle
            .gap_to(Vec2::new(vehicle.lane(), -20.0), Vec2::splat(RADIUS))
            .is_some());
        // What is behind does not count.
        assert_eq!(
            vehicle.gap_to(Vec2::new(vehicle.lane(), 20.0), Vec2::splat(RADIUS)),
            None
        );
    }

    #[test]
    fn it_always_stops_short_of_the_character() {
        for cruise in [3.0, 6.0, 7.5] {
            for start in [30.0, 6.0, 2.5] {
                let mut vehicle = car(Axis::Z, -1.0);
                (vehicle.speed, vehicle.cruise) = (cruise, cruise);
                let player = Vec2::new(vehicle.lane(), -start);
                let half = Vec2::splat(RADIUS + 0.5);
                for _ in 0..1800 {
                    let gap = vehicle.gap_to(player, half);
                    vehicle.drive(1.0 / 60.0, gap);
                    assert!(
                        !vehicle.mover().touches(player, RADIUS),
                        "cruise {cruise} from {start}"
                    );
                }
                assert!(vehicle.speed < 0.05);
                // Then the character steps aside and it drives on.
                for _ in 0..240 {
                    vehicle.drive(1.0 / 60.0, None);
                }
                assert!(vehicle.speed > cruise * 0.9);
            }
        }
    }

    #[test]
    fn a_cyclist_reaches_the_handlebars() {
        // For every height, the arm stretched from the shoulder ends on the
        // grip without being pulled out of shape.
        for height in 0..3u8 {
            let look = Look {
                height,
                ..Look::default()
            };
            let voxel = look.voxel();
            let arm = look.arm().size.y as f32 * voxel;
            let seat = Vec3::new(0.0, SADDLE.y, SADDLE.z + 2.0 * voxel);
            for (shoulder, grip) in figure::SHOULDERS.iter().zip(GRIPS) {
                let shoulder = seat + Quat::from_rotation_x(-LEAN) * (*shoulder * voxel);
                let stretch = (grip - shoulder).length() / arm;
                assert!(
                    (0.8..=1.5).contains(&stretch),
                    "height {height}: stretch {stretch}"
                );
                assert!(
                    shoulder.z > grip.z,
                    "the hands are in front of the shoulders"
                );
                assert!(shoulder.y > grip.y, "and below them");
            }
        }
    }

    #[test]
    fn feet_stay_on_the_pedals_all_the_way_round() {
        for step in 0..64 {
            let turn = step as f32 / 64.0 * std::f32::consts::TAU;
            for side in [-1.0, 1.0] {
                let foot = pedal_at(turn, side);
                assert!(
                    (foot - Vec3::new(side * STANCE, BRACKET.y, BRACKET.z)).length() - CRANK < 1e-4
                );
                let hip = Vec3::new(side * STANCE, SADDLE.y, SADDLE.z);
                // The pedal is always within reach: the leg is never pulled
                // off it.
                let reach = (foot - hip).length();
                assert!(
                    reach < THIGH + SHIN && reach > (THIGH - SHIN).abs(),
                    "turn {turn}: {reach}"
                );
                let knee = knee(hip, foot, THIGH, SHIN);
                assert!(((knee - hip).length() - THIGH).abs() < 1e-3, "thigh");
                assert!(((foot - knee).length() - SHIN).abs() < 1e-3, "shin");
                // The knee bends forward, in front of the line from hip to foot.
                let line = hip.z + (foot.z - hip.z) * ((knee.y - hip.y) / (foot.y - hip.y));
                assert!(knee.z < line, "turn {turn}: knee behind the leg");
            }
            // The two pedals are always opposite.
            let (left, right) = (pedal_at(turn, -1.0), pedal_at(turn, 1.0));
            assert!((left.y + right.y - 2.0 * BRACKET.y).abs() < 1e-4);
        }
    }

    #[test]
    fn the_pedals_go_round_forward() {
        // A quarter turn after the top, the right pedal is at the front.
        let top = pedal_at(0.0, 1.0);
        let quarter = pedal_at(std::f32::consts::FRAC_PI_2, 1.0);
        assert!(top.y > quarter.y && quarter.z < top.z);
    }

    #[test]
    fn streets_are_found_from_anywhere() {
        assert_eq!(street_near(3.0, 0), 11.0);
        assert_eq!(street_near(-3.0, 0), -11.0);
        assert_eq!(street_near(12.0, 1), 33.0);
        assert_eq!(street_near(11.0, -1), -11.0);
    }
}
