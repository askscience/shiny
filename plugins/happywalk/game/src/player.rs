//! `PlayerPlugin`: the character. A voxel figure in separate blocks (head,
//! body, arms, legs) that walks where it is told, swings its limbs as it
//! goes, and stops with a jolt when it walks into something.

use std::f32::consts::{FRAC_PI_2, PI};

use bevy::prelude::*;

use crate::bridge::{LookChosen, Wardrobe};
use crate::city::{SolidKind, Solids, START};
use crate::figure::Look;
use crate::intent::Order;
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

    fn faster(self) -> Self {
        match self {
            Pace::Slow => Pace::Walk,
            _ => Pace::Run,
        }
    }

    fn slower(self) -> Self {
        match self {
            Pace::Run => Pace::Walk,
            _ => Pace::Slow,
        }
    }
}

/// Half the side of the square the character takes on the ground.
pub const RADIUS: f32 = 0.38;
/// How long the jolt of a bump lasts, in seconds.
const JOLT: f32 = 0.5;
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
            stride: 0.0,
            swing: 0.0,
        }
    }
}

impl Player {
    /// Carry out a command.
    pub fn obey(&mut self, command: Order) {
        match command {
            Order::Forward => self.walking = true,
            Order::Back => {
                self.heading += PI;
                self.walking = true;
            }
            Order::Left => self.heading += FRAC_PI_2,
            Order::Right => self.heading -= FRAC_PI_2,
            Order::Stop => self.walking = false,
            Order::Faster => {
                // "Faster" to someone standing still means "get going".
                if self.walking {
                    self.pace = self.pace.faster();
                }
                self.walking = true;
            }
            Order::Slower => self.pace = self.pace.slower(),
            Order::Wave => self.waving = 1.4,
            // Where to go is the navigator's business.
            Order::GoTo(_) | Order::Clip => {}
        }
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
        let next = at + self.forward() * self.pace.speed() * dt;
        let hit = solids
            .what(next, RADIUS)
            .map(|solid| (solid.kind, solid.model))
            .or_else(|| {
                movers
                    .what(next, RADIUS)
                    .map(|mover| (mover.kind, mover.model))
            });
        match hit {
            Some((into, model)) => {
                self.hit_model = model;
                self.walking = false;
                self.jolt = JOLT;
                self.thrown = -self.forward();
                (at, Some(into))
            }
            None => (next, None),
        }
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
                (choose, dress, glance, walk, animate)
                    .chain()
                    .in_set(PlayerSet),
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
fn glance(mut said: MessageReader<crate::brain::Said>, mut players: Query<&mut Player>) {
    if said.read().count() > 0 {
        if let Ok(mut player) = players.single_mut() {
            player.glance = GLANCE;
        }
    }
}

fn walk(
    time: Res<Time>,
    solids: Res<Solids>,
    movers: Res<Movers>,
    wardrobe: Res<Wardrobe>,
    mut waves: MessageWriter<Waved>,
    mut orders: MessageReader<Order>,
    mut placed: MessageReader<crate::bridge::Placed>,
    mut bumps: MessageWriter<Bumped>,
    mut players: Query<(&mut Player, &mut Transform)>,
) {
    let Ok((mut player, mut transform)) = players.single_mut() else {
        return;
    };
    for order in orders.read() {
        player.obey(*order);
        if *order == Order::Wave {
            waves.write(Waved {
                at: Vec2::new(transform.translation.x, transform.translation.z),
            });
        }
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

    fn walk_until_bump(
        player: &mut Player,
        mut at: Vec2,
        solids: &Solids,
        steps: usize,
    ) -> (Vec2, Option<SolidKind>) {
        for _ in 0..steps {
            let (next, bumped) = player.step(at, 1.0 / 60.0, solids, &Movers::default());
            at = next;
            if bumped.is_some() || !player.walking {
                return (at, bumped);
            }
        }
        (at, None)
    }

    /// On the street beside a built lot, told to walk at its wall.
    fn facing_a_wall(solids: &Solids) -> (Player, Vec2) {
        let mut player = Player::default();
        let mut at = gen::centre(a_built_lot(SEED)) + Vec2::new(PITCH / 2.0, -1.5);
        player.obey(Order::Left);
        player.obey(Order::Forward);
        for _ in 0..30 {
            at = player.step(at, 1.0 / 60.0, solids, &Movers::default()).0;
        }
        (player, at)
    }

    #[test]
    fn forward_walks_up_the_street_without_end() {
        let solids = Solids::new(SEED);
        let mut player = Player::default();
        player.obey(Order::Forward);
        player.obey(Order::Faster);
        let start = Vec2::new(START.x, START.z);
        // Three minutes at a run: more than a kilometre, and no edge.
        let (at, bumped) = walk_until_bump(&mut player, start, &solids, 60 * 180);
        assert_eq!(bumped, None);
        assert!(player.walking);
        assert!(at.y < start.y - 1000.0, "{at:?}");
    }

    #[test]
    fn a_turn_into_the_lot_ends_in_a_building() {
        let solids = Solids::new(SEED);
        let (mut player, at) = facing_a_wall(&solids);
        let (_, bumped) = walk_until_bump(&mut player, at, &solids, 2000);
        assert_eq!(bumped, Some(SolidKind::Building));
        assert!(!player.walking);
        assert!(player.jolt > 0.0);
    }

    #[test]
    fn turning_back_after_a_bump_does_not_bump_again() {
        let solids = Solids::new(SEED);
        let (mut player, at) = facing_a_wall(&solids);
        let (mut at, bumped) = walk_until_bump(&mut player, at, &solids, 2000);
        assert_eq!(bumped, Some(SolidKind::Building));
        let wall = at;
        // Told to go back while still reeling: it turns round and walks
        // off, back to the street.
        player.obey(Order::Back);
        for _ in 0..120 {
            let (next, bumped) = player.step(at, 1.0 / 60.0, &solids, &Movers::default());
            assert_eq!(bumped, None);
            at = next;
        }
        assert!(player.walking);
        assert!(at.x > wall.x + 2.0, "{at:?}");
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

    #[test]
    fn a_passer_by_in_the_way_is_a_bump_too() {
        use crate::movers::Mover;
        let solids = Solids::new(SEED);
        let mut player = Player::default();
        player.obey(Order::Forward);
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
        let mut at = start;
        let mut bumped = None;
        for _ in 0..600 {
            let (next, hit) = player.step(at, 1.0 / 60.0, &solids, &movers);
            at = next;
            if hit.is_some() {
                bumped = hit;
                break;
            }
        }
        assert_eq!(bumped, Some(SolidKind::Person));
        assert!(at.y > start.y - 3.0);
    }

    #[test]
    fn orders() {
        let mut player = Player::default();
        player.obey(Order::Faster);
        assert!(player.walking);
        assert_eq!(player.pace, Pace::Walk);
        player.obey(Order::Faster);
        assert_eq!(player.pace, Pace::Run);
        player.obey(Order::Slower);
        player.obey(Order::Slower);
        assert_eq!(player.pace, Pace::Slow);
        player.obey(Order::Stop);
        assert!(!player.walking);
        player.obey(Order::Back);
        assert!(player.walking);
        assert!((player.heading - PI).abs() < 1e-6);
        player.obey(Order::Right);
        assert!((player.heading - FRAC_PI_2).abs() < 1e-6);
        player.obey(Order::Wave);
        assert!(player.waving > 0.0);
    }
}
