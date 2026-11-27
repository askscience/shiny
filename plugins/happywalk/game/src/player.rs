//! `PlayerPlugin`: the character. A voxel figure in separate blocks (head,
//! body, arms, legs) that walks where it is told, swings its limbs as it
//! goes, and stops with a jolt when it walks into something.

use std::f32::consts::PI;

use bevy::prelude::*;

use crate::bridge::{LookChosen, Wardrobe};
use crate::city::{SolidKind, Solids, START};
use crate::control::{Order, Steer, DEAD};
use crate::figure::Look;
use crate::movers::Movers;
use crate::vox::VoxLibrary;

/// The character walked into something.
#[derive(Message, Clone, Copy, Debug)]
pub struct Bumped {
    pub into: SolidKind,
    /// The model of what was hit (see `brain::describe`).
    pub what: &'static str,
    /// Where the character was.
    pub at: Vec2,
    /// How fast it was going.
    pub pace: Pace,
}

/// The character waved.
#[derive(Message, Clone, Copy, Debug)]
pub struct Waved {
    pub at: Vec2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pace {
    Slow,
    Walk,
    Run,
}

impl Pace {
    /// Metres a second.
    pub fn speed(self) -> f32 {
        match self {
            Pace::Slow => 1.5,
            Pace::Walk => 3.2,
            Pace::Run => 6.2,
        }
    }
}

/// Half the side of the square the character takes on the ground.
pub const RADIUS: f32 = 0.38;
/// How long the jolt of a bump lasts, in seconds.
const JOLT: f32 = 0.5;
/// Metres a second walking backwards.
const BACK_SPEED: f32 = 1.4;
/// Radians a second.
const TURN_RATE: f32 = 7.0;
/// Seconds a glance at the camera lasts.
const GLANCE: f32 = 2.2;
/// A turn wider than this (radians) is finished before the next step.
const ABOUT_FACE: f32 = 1.2;

#[derive(Component, Debug)]
pub struct Player {
    /// Where the character is turning to, and where it faces now (radians
    /// around the vertical; 0 looks down `-z`).
    pub heading: f32,
    pub facing: f32,
    pub walking: bool,
    pub pace: Pace,
    /// Seconds of jolt left after a bump, and which way it throws.
    pub jolt: f32,
    thrown: Vec2,
    /// The model of the last thing walked into.
    pub hit_model: &'static str,
    /// Seconds of waving left.
    pub waving: f32,
    /// Seconds left of looking over its shoulder at the camera.
    pub glance: f32,
    /// Walking backwards (a key or the joystick held back).
    pub backing: bool,
    /// Stopped by something last step: a bump is told once, not every frame
    /// it is pushed against.
    blocked: bool,
    /// Where the limbs are in their swing, and how wide it is (0..1).
    stride: f32,
    swing: f32,
}

impl Default for Player {
    fn default() -> Self {
        Self {
            heading: 0.0,
            facing: 0.0,
            walking: false,
            pace: Pace::Walk,
            jolt: 0.0,
            thrown: Vec2::ZERO,
            waving: 0.0,
            hit_model: "",
            glance: 0.0,
            backing: false,
            blocked: false,
            stride: 0.0,
            swing: 0.0,
        }
    }
}

impl Player {
    /// Follow the held controls: turn while they turn, walk while they
    /// push. A joystick points the way to walk, seen from `view` (the way
    /// the camera looked, as a facing, when it was first pushed).
    pub fn drive(&mut self, steer: &Steer, view: f32) {
        if let Some(stick) = steer.stick {
            // Right on the screen turns clockwise seen from above.
            let towards = view - stick.x.atan2(stick.y);
            // The nearest way round to it from where it faces.
            let turn = (towards - self.facing + PI).rem_euclid(std::f32::consts::TAU) - PI;
            self.heading = self.facing + turn;
            self.walking = true;
            self.backing = false;
            self.pace = if steer.run { Pace::Run } else { Pace::Walk };
            return;
        }
        // A heading a little ahead of where it faces: it turns at its own
        // rate, as long as the control is held.
        self.heading = self.facing - steer.turn * 0.6;
        self.walking = steer.forward.abs() > DEAD;
        self.backing = steer.forward < -DEAD;
        self.pace = if steer.run { Pace::Run } else { Pace::Walk };
    }

    /// The direction the character faces, on the ground (`x`, `z`).
    pub fn forward(&self) -> Vec2 {
        Vec2::new(-self.facing.sin(), -self.facing.cos())
    }

    /// One step of `dt` seconds from `at`. Returns the new place and what
    /// was walked into, if anything.
    pub fn step(
        &mut self,
        at: Vec2,
        dt: f32,
        solids: &Solids,
        movers: &Movers,
    ) -> (Vec2, Option<SolidKind>) {
        let turn = self.heading - self.facing;
        let most = TURN_RATE * dt;
        self.facing += turn.clamp(-most, most);

        let mut at = at;
        if self.jolt > 0.0 {
            // Thrown back a little, away from what was hit (wherever the
            // character has turned to since), as long as there is room.
            let back = at + self.thrown * 1.6 * dt * (self.jolt / JOLT);
            if solids.hit(back, RADIUS).is_none() && movers.hit(back, RADIUS).is_none() {
                at = back;
            }
            self.jolt = (self.jolt - dt).max(0.0);
            return (at, None);
        }
        // It turns on the spot before it sets off the other way: walking
        // while still facing the wall it just hit would hit it again.
        if !self.walking || turn.abs() > ABOUT_FACE {
            return (at, None);
        }
        let speed = if self.backing {
            -BACK_SPEED
        } else {
            self.pace.speed()
        };
        let step = self.forward() * speed * dt;
        let hit = |p: Vec2| {
            solids
                .what(p, RADIUS)
                .map(|solid| (solid.kind, solid.model))
                .or_else(|| {
                    movers
                        .what(p, RADIUS)
                        .map(|mover| (mover.kind, mover.model))
                })
        };
        let Some((into, model)) = hit(at + step) else {
            self.blocked = false;
            return (at + step, None);
        };
        // Along the wall, if it is only grazed.
        for slide in [Vec2::new(step.x, 0.0), Vec2::new(0.0, step.y)] {
            if slide.length() > step.length() * 0.3 && hit(at + slide).is_none() {
                return (at + slide, None);
            }
        }
        self.hit_model = model;
        // Into a person: a jolt back. Into anything else it just stops.
        if into == SolidKind::Person && self.jolt == 0.0 {
            self.jolt = JOLT;
            self.thrown = -self.forward();
        }
        let first = !self.blocked;
        self.blocked = true;
        (at, first.then_some(into))
    }
}

/// A limb that swings from its joint; `side` is +1 or -1.
#[derive(Component)]
struct Limb {
    side: f32,
    reach: f32,
}

#[derive(Component)]
struct Body;

#[derive(Component)]
struct Head;

/// The systems that move the character; reactions to a bump run after.
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PlayerSet;

pub struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PlayerLook>()
            .add_message::<Bumped>()
            .add_message::<Waved>()
            .init_resource::<Movers>()
            .add_systems(Startup, spawn)
            .add_systems(
                Update,
                (choose, dress, walk, animate).chain().in_set(PlayerSet),
            );
    }
}

/// How the character looks. Changing it dresses the character again.
#[derive(Resource, Default, Clone, Copy, Debug)]
pub struct PlayerLook(pub Look);

/// A part of the figure hanging from the character: all of them are
/// replaced when the look changes.
#[derive(Component)]
struct FigurePart;

fn spawn(mut commands: Commands) {
    commands.spawn((
        Player::default(),
        Transform::from_translation(START),
        Visibility::default(),
    ));
}

/// The look the page chose, or kept from last time.
fn choose(mut chosen: MessageReader<LookChosen>, mut look: ResMut<PlayerLook>) {
    for choice in chosen.read() {
        if look.0 != choice.0 {
            look.0 = choice.0;
        }
    }
}

/// Build the figure from the look: at the start, and whenever it changes.
fn dress(
    mut commands: Commands,
    look: Res<PlayerLook>,
    library: Res<VoxLibrary>,
    mut meshes: ResMut<Assets<Mesh>>,
    players: Query<Entity, With<Player>>,
    old: Query<Entity, With<FigurePart>>,
) {
    if !look.is_changed() {
        return;
    }
    let Ok(player) = players.single() else { return };
    for part in &old {
        commands.entity(part).despawn();
    }
    let parts = look.0.parts();
    let voxel = look.0.voxel();
    // Everything is measured from the parts, so any look fits together.
    let size = |model: &crate::vox::VoxModel| model.size.as_vec3() * voxel;
    let (leg, torso, arm) = (size(&parts.leg), size(&parts.torso), size(&parts.arm));
    let hip = leg.y - 0.02;
    let shoulder = hip + torso.y - 0.1;
    let chest = torso.y - 0.06;
    // The body is two voxels narrower than its model (room for a skirt).
    let reach = (torso.x - 2.0 * voxel + arm.x) / 2.0 - 0.03;
    let apart = leg.x / 2.0 + 0.02;
    let material = library.material.clone();
    let mut piece = |model: &crate::vox::VoxModel, at: Vec3| {
        (
            Mesh3d(meshes.add(model.mesh())),
            MeshMaterial3d(material.clone()),
            Transform::from_translation(at).with_scale(Vec3::splat(voxel)),
        )
    };
    let (leg_piece, arm_piece) = (
        piece(&parts.leg, Vec3::new(0.0, -leg.y, -0.02)),
        piece(&parts.arm, Vec3::new(0.0, -arm.y, 0.0)),
    );
    let torso_piece = piece(&parts.torso, Vec3::ZERO);
    let head_piece = piece(&parts.head, Vec3::ZERO);
    let pack_piece = parts.backpack.as_ref().map(|pack| {
        let behind = (torso.z + size(pack).z) / 2.0 - 0.1;
        piece(pack, Vec3::new(0.0, 0.06, behind))
    });
    commands.entity(player).with_children(|figure| {
        // Legs swing from the hip, arms from the shoulder: each is a joint
        // with its part hanging below it.
        for side in [-1.0, 1.0] {
            figure
                .spawn((
                    FigurePart,
                    Limb { side, reach: 0.75 },
                    Transform::from_xyz(side * apart, hip, 0.0),
                    Visibility::default(),
                ))
                .with_children(|joint| {
                    joint.spawn(leg_piece.clone());
                });
            figure
                .spawn((
                    FigurePart,
                    Limb {
                        side: -side,
                        reach: 0.95,
                    },
                    Transform::from_xyz(side * reach, shoulder, 0.0),
                    Visibility::default(),
                ))
                .with_children(|joint| {
                    joint.spawn(arm_piece.clone());
                });
        }
        figure
            .spawn((
                FigurePart,
                Body,
                Transform::from_xyz(0.0, hip, 0.0),
                Visibility::default(),
            ))
            .with_children(|body| {
                body.spawn(torso_piece);
                if let Some(pack) = pack_piece {
                    body.spawn(pack);
                }
                body.spawn((
                    Head,
                    Transform::from_xyz(0.0, chest, 0.0),
                    Visibility::default(),
                ))
                .with_children(|head| {
                    head.spawn(head_piece);
                });
            });
    });
}

/// A line said after something went wrong comes with a look at the camera.
fn walk(
    time: Res<Time>,
    solids: Res<Solids>,
    movers: Res<Movers>,
    wardrobe: Res<Wardrobe>,
    mut waves: MessageWriter<Waved>,
    mut orders: MessageReader<Order>,
    steer: Res<Steer>,
    mut steering: Local<bool>,
    mut view: Local<Option<f32>>,
    cameras: Query<&Transform, (With<crate::camera::FollowCamera>, Without<Player>)>,
    mut route: ResMut<crate::nav::Route>,
    mut placed: MessageReader<crate::bridge::Placed>,
    mut bumps: MessageWriter<Bumped>,
    mut players: Query<(&mut Player, &mut Transform)>,
) {
    let Ok((mut player, mut transform)) = players.single_mut() else {
        return;
    };
    for order in orders.read() {
        if *order == Order::Wave {
            player.waving = 1.4;
            waves.write(Waved {
                at: Vec2::new(transform.translation.x, transform.translation.z),
            });
        }
    }
    // Held controls win over a walk set by a tap; letting go stops.
    if steer.active() {
        // The view the joystick is read in: as it was when first pushed.
        let seen = match (steer.stick, *view) {
            (Some(_), Some(seen)) => seen,
            (Some(_), None) => {
                // Which way the camera looks, as a facing.
                let look = cameras.single().map(|c| c.forward().as_vec3()).unwrap_or(Vec3::NEG_Z);
                let seen = if look.x.abs() + look.z.abs() > 1e-3 {
                    (-look.x).atan2(-look.z)
                } else {
                    player.facing
                };
                *view.insert(seen)
            }
            (None, _) => {
                *view = None;
                player.facing
            }
        };
        player.drive(&steer, seen);
        *route = crate::nav::Route::default();
        *steering = true;
    } else if *steering {
        *steering = false;
        *view = None;
        player.walking = false;
        player.backing = false;
    }
    for place in placed.read() {
        transform.translation = Vec3::new(place.at.x, 0.0, place.at.y);
        (player.heading, player.facing, player.walking, player.jolt) =
            (place.facing, place.facing, false, 0.0);
    }
    // A long frame must not carry the character through a wall.
    let dt = time.delta_secs().min(0.05);
    let at = Vec2::new(transform.translation.x, transform.translation.z);
    if wardrobe.0 {
        player.walking = false;
    }
    player.waving = (player.waving - dt).max(0.0);
    player.glance = (player.glance - dt).max(0.0);
    let (at, bumped) = player.step(at, dt, &solids, &movers);
    transform.translation.x = at.x;
    transform.translation.z = at.y;
    transform.rotation = Quat::from_rotation_y(player.facing);
    if let Some(into) = bumped {
        bumps.write(Bumped {
            into,
            what: player.hit_model,
            at,
            pace: player.pace,
        });
    }
    crate::bridge::publish_state(format!(
        r#"{{"x":{:.3},"z":{:.3},"facing":{:.3},"walking":{},"pace":"{:?}","jolt":{:.2}}}"#,
        at.x, at.y, player.facing, player.walking, player.pace, player.jolt
    ));
}

fn animate(
    time: Res<Time>,
    mut players: Query<
        (&mut Player, &mut Transform),
        (Without<Limb>, Without<Body>, Without<Head>),
    >,
    mut limbs: Query<(&Limb, &mut Transform), (Without<Player>, Without<Body>, Without<Head>)>,
    mut bodies: Query<&mut Transform, (With<Body>, Without<Player>, Without<Limb>, Without<Head>)>,
    mut heads: Query<&mut Transform, (With<Head>, Without<Player>, Without<Limb>, Without<Body>)>,
) {
    let Ok((mut player, mut root)) = players.single_mut() else {
        return;
    };
    let dt = time.delta_secs();
    let moving = player.walking && player.jolt == 0.0;
    let wide = if moving { 1.0 } else { 0.0 };
    player.swing += (wide - player.swing) * (dt * 10.0).min(1.0);
    if moving {
        player.stride += player.pace.speed() * dt * 2.4;
    }
    let swing = player.stride.sin() * player.swing;
    let jolt = player.jolt / JOLT;

    root.translation.y =
        (player.stride * 2.0).sin().abs() * 0.05 * player.swing + jolt * (jolt * PI).sin() * 0.25;
    for (limb, mut transform) in &mut limbs {
        // Thrown up in surprise when it bumps.
        let surprise = if limb.reach > 0.9 { -jolt * 2.2 } else { 0.0 };
        // A wave: one arm up, swinging from the shoulder.
        if limb.reach > 0.9 && limb.side < 0.0 && player.waving > 0.0 {
            let up = (player.waving / 0.25).min(1.0) * ((1.4 - player.waving) / 0.25).min(1.0);
            transform.rotation = Quat::from_rotation_z((player.waving * 14.0).sin() * 0.35 * up)
                * Quat::from_rotation_x(-2.7 * up);
            continue;
        }
        transform.rotation = Quat::from_rotation_x(limb.side * swing * limb.reach + surprise);
    }
    if let Ok(mut body) = bodies.single_mut() {
        body.rotation = Quat::from_rotation_x(-0.08 * player.swing + jolt * 0.45);
    }
    if let Ok(mut head) = heads.single_mut() {
        // Saying its line, it looks back at whoever is steering.
        let turn =
            (player.glance / 0.3).min(1.0) * ((GLANCE - player.glance) / 0.3).clamp(0.0, 1.0);
        head.rotation = Quat::from_rotation_y(2.2 * turn)
            * Quat::from_rotation_z((jolt * 22.0).sin() * jolt * 0.35);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::city::{a_built_lot, PITCH};
    use crate::gen;

    const SEED: u64 = 9;

    fn held(forward: f32, turn: f32, run: bool) -> Steer {
        Steer {
            forward,
            turn,
            run,
            ..default()
        }
    }

    /// Steps of 1/60 s with the controls held, until it is stopped.
    fn walk_held(
        player: &mut Player,
        mut at: Vec2,
        steer: Steer,
        solids: &Solids,
        movers: &Movers,
        steps: usize,
    ) -> (Vec2, Option<SolidKind>) {
        for _ in 0..steps {
            player.drive(&steer, player.facing);
            let (next, bumped) = player.step(at, 1.0 / 60.0, solids, movers);
            if bumped.is_some() {
                return (next, bumped);
            }
            at = next;
        }
        (at, None)
    }

    #[test]
    fn held_forward_walks_up_the_street_without_end() {
        let solids = Solids::new(SEED);
        let mut player = Player::default();
        let start = Vec2::new(START.x, START.z);
        // Three minutes at a run: more than a kilometre, and no edge.
        let (at, bumped) = walk_held(
            &mut player,
            start,
            held(1.0, 0.0, true),
            &solids,
            &Movers::default(),
            60 * 180,
        );
        assert_eq!(bumped, None);
        assert_eq!(player.pace, Pace::Run);
        assert!(at.y < start.y - 1000.0, "{at:?}");
    }

    #[test]
    fn turning_while_held_goes_round() {
        let mut player = Player::default();
        for _ in 0..30 {
            player.drive(&held(0.0, 1.0, false), 0.0);
            player.step(
                Vec2::ZERO,
                1.0 / 60.0,
                &Solids::new(SEED),
                &Movers::default(),
            );
        }
        // Right turns clockwise seen from above: the facing goes down.
        assert!(player.facing < -1.0, "{}", player.facing);
        assert!(!player.walking);
    }

    #[test]
    fn back_walks_backwards_without_turning() {
        let solids = Solids::new(SEED);
        let mut player = Player::default();
        let start = Vec2::new(START.x, START.z);
        let (at, _) = walk_held(
            &mut player,
            start,
            held(-1.0, 0.0, false),
            &solids,
            &Movers::default(),
            60,
        );
        assert!(at.y > start.y + 1.0, "{at:?}");
        assert!(player.facing.abs() < 1e-6);
    }

    #[test]
    fn a_building_stops_it_once_and_without_a_jolt() {
        let solids = Solids::new(SEED);
        let mut player = Player {
            facing: std::f32::consts::FRAC_PI_2,
            heading: std::f32::consts::FRAC_PI_2,
            ..default()
        };
        let mut at = gen::centre(a_built_lot(SEED)) + Vec2::new(PITCH / 2.0, -1.5);
        let steer = held(1.0, 0.0, false);
        let mut bumps = 0;
        for _ in 0..600 {
            player.drive(&steer, player.facing);
            player.heading = player.facing;
            let (next, bumped) = player.step(at, 1.0 / 60.0, &solids, &Movers::default());
            bumps += usize::from(bumped.is_some());
            at = next;
        }
        assert_eq!(bumps, 1, "told once, however long it pushes");
        assert_eq!(player.jolt, 0.0);
    }

    #[test]
    fn a_passer_by_in_the_way_is_a_bump_with_a_jolt() {
        use crate::movers::Mover;
        let solids = Solids::new(SEED);
        let mut player = Player::default();
        let start = Vec2::new(START.x, START.z);
        let movers = Movers {
            people: vec![Mover {
                at: start + Vec2::new(0.0, -3.0),
                half: Vec2::splat(0.28),
                kind: SolidKind::Person,
                model: "person",
            }],
            vehicles: Vec::new(),
        };
        let (at, bumped) = walk_held(
            &mut player,
            start,
            held(1.0, 0.0, false),
            &solids,
            &movers,
            600,
        );
        assert_eq!(bumped, Some(SolidKind::Person));
        assert!(player.jolt > 0.0);
        assert!(at.y > start.y - 3.0);
    }

    #[test]
    fn a_joystick_walks_the_way_it_is_pushed_without_circling() {
        let solids = Solids::new(SEED);
        let mut player = Player::default();
        let start = Vec2::new(START.x, START.z);
        let mut at = start;
        // Pushed right and held: the view stays the one it was pushed in,
        // so the character settles on one way (to the screen's right, +x)
        // and walks it straight.
        let steer = Steer {
            stick: Some(Vec2::X),
            ..default()
        };
        for _ in 0..120 {
            player.drive(&steer, 0.0);
            at = player.step(at, 1.0 / 60.0, &solids, &Movers::default()).0;
        }
        assert!(at.x > start.x + 4.0, "{at:?}");
        assert!((at.y - start.y).abs() < 1.0, "{at:?}");
        assert!(
            (player.facing + std::f32::consts::FRAC_PI_2).abs() < 0.05,
            "{}",
            player.facing
        );
    }

    #[test]
    fn the_jolt_ends_and_the_character_stands_still() {
        let solids = Solids::new(SEED);
        let mut player = Player {
            jolt: JOLT,
            ..default()
        };
        let mut at = Vec2::new(START.x, START.z);
        for _ in 0..120 {
            at = player.step(at, 1.0 / 60.0, &solids, &Movers::default()).0;
        }
        assert_eq!(player.jolt, 0.0);
        let still = player.step(at, 1.0 / 60.0, &solids, &Movers::default()).0;
        assert_eq!(still, at);
    }
}
