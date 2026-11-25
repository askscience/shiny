//! `PlayerPlugin`: the character. A voxel figure in separate blocks (head,
//! body, arms, legs) that walks where it is told, swings its limbs as it
//! goes, and stops with a jolt when it walks into something.

use std::f32::consts::{FRAC_PI_2, PI};

use bevy::prelude::*;

use crate::city::{SolidKind, Solids, START};
use crate::intent::Order;

/// The character walked into something.
#[derive(Message, Clone, Copy, Debug)]
pub struct Bumped {
    pub into: SolidKind,
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
        }
    }

    /// The direction the character faces, on the ground (`x`, `z`).
    pub fn forward(&self) -> Vec2 {
        Vec2::new(-self.facing.sin(), -self.facing.cos())
    }

    /// One step of `dt` seconds from `at`. Returns the new place and what
    /// was walked into, if anything.
    pub fn step(&mut self, at: Vec2, dt: f32, solids: &Solids) -> (Vec2, Option<SolidKind>) {
        let turn = self.heading - self.facing;
        let most = TURN_RATE * dt;
        self.facing += turn.clamp(-most, most);

        let mut at = at;
        if self.jolt > 0.0 {
            // Thrown back a little, away from what was hit (wherever the
            // character has turned to since), as long as there is room.
            let back = at + self.thrown * 1.6 * dt * (self.jolt / JOLT);
            if solids.hit(back, RADIUS).is_none() {
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
        match solids.hit(next, RADIUS) {
            Some(into) => {
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
        app.add_message::<Bumped>()
            .add_systems(Startup, spawn)
            .add_systems(Update, (walk, animate).chain().in_set(PlayerSet));
    }
}

fn spawn(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mut matte = |r: f32, g: f32, b: f32| {
        materials.add(StandardMaterial {
            base_color: Color::srgb(r, g, b),
            perceptual_roughness: 0.9,
            reflectance: 0.1,
            ..default()
        })
    };
    let skin = matte(0.99, 0.84, 0.74);
    let hair = matte(0.45, 0.33, 0.30);
    let shirt = matte(0.99, 0.70, 0.45);
    let trousers = matte(0.45, 0.56, 0.80);
    let shoes = matte(0.96, 0.96, 0.98);
    let dark = matte(0.20, 0.20, 0.26);
    let pack = matte(0.55, 0.80, 0.72);
    let unit = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    let part = |at: Vec3, size: Vec3, material: &Handle<StandardMaterial>| {
        (
            Mesh3d(unit.clone()),
            MeshMaterial3d(material.clone()),
            Transform::from_translation(at).with_scale(size),
        )
    };

    commands
        .spawn((
            Player::default(),
            Transform::from_translation(START),
            Visibility::default(),
        ))
        .with_children(|figure| {
            // Legs swing from the hip, arms from the shoulder: each is a
            // joint with its blocks hanging below it.
            for side in [-1.0, 1.0] {
                figure
                    .spawn((
                        Limb { side, reach: 0.75 },
                        Transform::from_xyz(side * 0.19, 0.82, 0.0),
                        Visibility::default(),
                    ))
                    .with_children(|leg| {
                        leg.spawn(part(
                            Vec3::new(0.0, -0.36, 0.0),
                            Vec3::new(0.30, 0.72, 0.32),
                            &trousers,
                        ));
                        leg.spawn(part(
                            Vec3::new(0.0, -0.75, -0.04),
                            Vec3::new(0.32, 0.14, 0.42),
                            &shoes,
                        ));
                    });
                figure
                    .spawn((
                        Limb {
                            side: -side,
                            reach: 0.95,
                        },
                        Transform::from_xyz(side * 0.47, 1.56, 0.0),
                        Visibility::default(),
                    ))
                    .with_children(|arm| {
                        arm.spawn(part(
                            Vec3::new(0.0, -0.22, 0.0),
                            Vec3::new(0.22, 0.44, 0.26),
                            &shirt,
                        ));
                        arm.spawn(part(
                            Vec3::new(0.0, -0.58, 0.0),
                            Vec3::new(0.20, 0.30, 0.22),
                            &skin,
                        ));
                    });
            }
            figure
                .spawn((
                    Body,
                    Transform::from_xyz(0.0, 0.82, 0.0),
                    Visibility::default(),
                ))
                .with_children(|body| {
                    body.spawn(part(
                        Vec3::new(0.0, 0.40, 0.0),
                        Vec3::new(0.70, 0.80, 0.40),
                        &shirt,
                    ));
                    body.spawn(part(
                        Vec3::new(0.0, 0.42, 0.30),
                        Vec3::new(0.50, 0.56, 0.22),
                        &pack,
                    ));
                    body.spawn((
                        Head,
                        Transform::from_xyz(0.0, 0.82, 0.0),
                        Visibility::default(),
                    ))
                    .with_children(|head| {
                        head.spawn(part(
                            Vec3::new(0.0, 0.32, 0.0),
                            Vec3::new(0.62, 0.62, 0.60),
                            &skin,
                        ));
                        head.spawn(part(
                            Vec3::new(0.0, 0.60, 0.04),
                            Vec3::new(0.68, 0.20, 0.68),
                            &hair,
                        ));
                        head.spawn(part(
                            Vec3::new(0.0, 0.40, 0.26),
                            Vec3::new(0.68, 0.44, 0.20),
                            &hair,
                        ));
                        // The face is on the `-z` side: that is where it walks.
                        for side in [-1.0, 1.0] {
                            head.spawn(part(
                                Vec3::new(side * 0.15, 0.34, -0.31),
                                Vec3::new(0.09, 0.12, 0.03),
                                &dark,
                            ));
                        }
                        head.spawn(part(
                            Vec3::new(0.0, 0.17, -0.31),
                            Vec3::new(0.18, 0.04, 0.03),
                            &dark,
                        ));
                    });
                });
        });
}

fn walk(
    time: Res<Time>,
    solids: Res<Solids>,
    mut orders: MessageReader<Order>,
    mut bumps: MessageWriter<Bumped>,
    mut players: Query<(&mut Player, &mut Transform)>,
) {
    let Ok((mut player, mut transform)) = players.single_mut() else {
        return;
    };
    for order in orders.read() {
        player.obey(*order);
    }
    // A long frame must not carry the character through a wall.
    let dt = time.delta_secs().min(0.05);
    let at = Vec2::new(transform.translation.x, transform.translation.z);
    let (at, bumped) = player.step(at, dt, &solids);
    transform.translation.x = at.x;
    transform.translation.z = at.y;
    transform.rotation = Quat::from_rotation_y(player.facing);
    if let Some(into) = bumped {
        bumps.write(Bumped { into });
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
        transform.rotation = Quat::from_rotation_x(limb.side * swing * limb.reach + surprise);
    }
    if let Ok(mut body) = bodies.single_mut() {
        body.rotation = Quat::from_rotation_x(-0.08 * player.swing + jolt * 0.45);
    }
    if let Ok(mut head) = heads.single_mut() {
        head.rotation = Quat::from_rotation_z((jolt * 22.0).sin() * jolt * 0.35);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::city::{solids, PITCH};

    fn walk_until_bump(
        player: &mut Player,
        mut at: Vec2,
        solids: &Solids,
    ) -> (Vec2, Option<SolidKind>) {
        for _ in 0..10_000 {
            let (next, bumped) = player.step(at, 1.0 / 60.0, solids);
            at = next;
            if bumped.is_some() || !player.walking {
                return (at, bumped);
            }
        }
        (at, None)
    }

    #[test]
    fn forward_walks_up_the_street_to_the_hedge() {
        let solids = Solids(solids());
        let mut player = Player::default();
        player.obey(Order::Forward);
        let start = Vec2::new(START.x, START.z);
        let (at, bumped) = walk_until_bump(&mut player, start, &solids);
        assert_eq!(bumped, Some(SolidKind::Edge));
        assert!(at.y < start.y - 40.0, "{at:?}");
        assert!(!player.walking);
        assert!(player.jolt > 0.0);
    }

    #[test]
    fn a_turn_into_the_lot_ends_in_a_building() {
        let solids = Solids(solids());
        let mut player = Player::default();
        // From the crossing, a few metres up the street, then left: a wall.
        let mut at = Vec2::new(PITCH / 2.0, 4.0);
        player.obey(Order::Left);
        player.obey(Order::Forward);
        for _ in 0..60 {
            at = player.step(at, 1.0 / 60.0, &solids).0;
        }
        let (_, bumped) = walk_until_bump(&mut player, at, &solids);
        assert_eq!(bumped, Some(SolidKind::Building));
    }

    #[test]
    fn the_jolt_ends_and_the_character_stands_still() {
        let solids = Solids(solids());
        let mut player = Player {
            jolt: JOLT,
            ..default()
        };
        let mut at = Vec2::new(START.x, START.z);
        for _ in 0..120 {
            at = player.step(at, 1.0 / 60.0, &solids).0;
        }
        assert_eq!(player.jolt, 0.0);
        let still = player.step(at, 1.0 / 60.0, &solids).0;
        assert_eq!(still, at);
    }

    #[test]
    fn turning_back_after_a_bump_does_not_bump_again() {
        let solids = Solids(solids());
        let mut player = Player::default();
        let mut at = Vec2::new(PITCH / 2.0, 4.0);
        player.obey(Order::Left);
        player.obey(Order::Forward);
        for _ in 0..60 {
            at = player.step(at, 1.0 / 60.0, &solids).0;
        }
        let (mut at, bumped) = walk_until_bump(&mut player, at, &solids);
        assert_eq!(bumped, Some(SolidKind::Building));
        // Told to go back while still reeling: it turns round and walks
        // off, back to the street.
        player.obey(Order::Back);
        for _ in 0..120 {
            let (next, bumped) = player.step(at, 1.0 / 60.0, &solids);
            assert_eq!(bumped, None);
            at = next;
        }
        assert!(player.walking);
        assert!(at.x > PITCH / 2.0 - 2.0, "{at:?}");
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
    }
}
