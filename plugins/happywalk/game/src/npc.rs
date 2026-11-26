//! `NpcPlugin`: passers-by. Each walks round the pavement of its lot at its
//! own pace, waits when the character is in the way, and says something from
//! a short list when walked into or greeted. They are the same figure as the
//! character in other clothes, drawn in three pieces each.
//!
//! Who lives where comes from the city's seed, so a lot has the same people
//! every time; they exist only in the chunks next to the player.

use std::collections::HashMap;
use std::f32::consts::{FRAC_PI_2, PI};

use bevy::prelude::*;

use crate::city::{CitySeed, PAVED};
use crate::figure::Look;
use crate::gen::{self, SolidKind};
use crate::hud::OtherSaid;
use crate::lines::{passer_by, Remark};
use crate::movers::{Mover, Movers};
use crate::player::{Bumped, Player, PlayerSet, Waved};
use crate::vox::VoxLibrary;

/// Passers-by live in chunks this near the player.
const NEAR: i32 = 1;
/// How far from the middle of a lot its people walk: along the kerb.
const TRACK: f32 = PAVED / 2.0 - 0.28;

#[derive(Component, Debug)]
pub struct Npc {
    lot: Vec2,
    /// Metres along the lot's kerb, from its `-x -z` corner, clockwise.
    along: f32,
    /// +1 or -1: which way round.
    way: f32,
    speed: f32,
    /// Seconds it stands still for (someone in the way, or just spoken to).
    pause: f32,
    stride: f32,
    hop: f32,
}

/// Where a walker is on a lot's kerb, and which way it faces (radians, 0
/// looks down `-z`), `along` metres from the `-x -z` corner going clockwise
/// seen from above (`+x` first).
pub fn on_track(lot: Vec2, along: f32, way: f32) -> (Vec2, f32) {
    let side = TRACK * 2.0;
    let along = along.rem_euclid(side * 4.0);
    let (leg, t) = ((along / side) as usize % 4, along % side);
    // Clockwise seen from above with z towards the viewer: +x, +z, -x, -z.
    let (at, heading) = match leg {
        0 => (Vec2::new(-TRACK + t, -TRACK), -FRAC_PI_2),
        1 => (Vec2::new(TRACK, -TRACK + t), PI),
        2 => (Vec2::new(TRACK - t, TRACK), FRAC_PI_2),
        _ => (Vec2::new(-TRACK, TRACK - t), 0.0),
    };
    (lot + at, if way > 0.0 { heading } else { heading + PI })
}

/// A passer-by as the seed makes it: where on the kerb, which way round,
/// how fast, and what it looks like.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Resident {
    pub along: f32,
    pub way: f32,
    pub speed: f32,
    pub look: Look,
}

/// The passers-by of a chunk: none, one or two, from the seed. Each is a
/// person of its own (`figure.rs`): the same lot has the same people.
pub fn residents(seed: u64, chunk: IVec2) -> Vec<Resident> {
    let count = gen::chance(seed, chunk, 700) % 3;
    (0..count)
        .map(|i| {
            let roll = |salt: u32| gen::chance(seed, chunk, 710 + i * 10 + salt);
            Resident {
                along: (roll(0) % 1000) as f32 / 1000.0 * TRACK * 8.0,
                way: if roll(1) % 2 == 0 { 1.0 } else { -1.0 },
                speed: 0.9 + (roll(2) % 100) as f32 / 100.0 * 0.9,
                look: Look::random(((roll(3) as u64) << 32) | roll(4) as u64),
            }
        })
        .collect()
}

#[derive(Component)]
struct NpcLeg {
    side: f32,
    /// Where the hip is, and how long the leg.
    hip: Vec3,
    length: f32,
}

#[derive(Component)]
struct NpcBody;

#[derive(Resource, Default)]
struct Housed(HashMap<IVec2, Vec<Entity>>);

pub struct NpcPlugin;

impl Plugin for NpcPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Housed>()
            .add_systems(Update, (populate, stroll).chain().before(PlayerSet))
            .add_systems(Update, (react, animate).chain().after(PlayerSet));
    }
}

/// Keep the people of the chunks around the player on screen.
fn populate(
    mut commands: Commands,
    seed: Res<CitySeed>,
    library: Res<VoxLibrary>,
    mut meshes: ResMut<Assets<Mesh>>,
    players: Query<&Transform, With<Player>>,
    mut housed: ResMut<Housed>,
    mut here: Local<Option<IVec2>>,
) {
    let Ok(player) = players.single() else { return };
    let home = gen::chunk_of(Vec2::new(player.translation.x, player.translation.z));
    if *here == Some(home) {
        return;
    }
    *here = Some(home);
    let gone: Vec<IVec2> = housed
        .0
        .keys()
        .copied()
        .filter(|c| (*c - home).abs().max_element() > NEAR)
        .collect();
    for coord in gone {
        for entity in housed.0.remove(&coord).unwrap_or_default() {
            commands.entity(entity).despawn();
        }
    }
    for x in -NEAR..=NEAR {
        for z in -NEAR..=NEAR {
            let coord = home + IVec2::new(x, z);
            if housed.0.contains_key(&coord) {
                continue;
            }
            let lot = gen::centre(coord);
            let people = residents(seed.0, coord)
                .into_iter()
                .map(|resident| {
                    // Two meshes a person: everything above the hips, and a
                    // leg used twice.
                    let voxel = resident.look.voxel();
                    let (body, leg) = (resident.look.body(), resident.look.leg());
                    let size = leg.size.as_vec3() * voxel;
                    let hip = size.y - 0.02;
                    let piece = |mesh: Handle<Mesh>, at: Vec3| {
                        (
                            Mesh3d(mesh),
                            MeshMaterial3d(library.material.clone()),
                            Transform::from_translation(at).with_scale(Vec3::splat(voxel)),
                        )
                    };
                    let (body, leg) = (meshes.add(body.mesh()), meshes.add(leg.mesh()));
                    commands
                        .spawn((
                            Npc {
                                lot,
                                along: resident.along,
                                way: resident.way,
                                speed: resident.speed,
                                pause: 0.0,
                                stride: resident.along,
                                hop: 0.0,
                            },
                            Transform::default(),
                            Visibility::default(),
                        ))
                        .with_children(|figure| {
                            // The body's model is deeper behind (hair, a
                            // bag) than in front: its middle is not the
                            // figure's.
                            figure.spawn((NpcBody, piece(body, Vec3::new(0.0, hip, 3.0 * voxel))));
                            for side in [-1.0, 1.0] {
                                figure.spawn((
                                    NpcLeg {
                                        side,
                                        hip: Vec3::new(side * (size.x / 2.0 + 0.02), hip, -0.02),
                                        length: size.y,
                                    },
                                    piece(leg.clone(), Vec3::ZERO),
                                ));
                            }
                        })
                        .id()
                })
                .collect();
            housed.0.insert(coord, people);
        }
    }
}

/// Everyone takes a step, unless the character is right in front.
fn stroll(
    time: Res<Time>,
    players: Query<&Transform, (With<Player>, Without<Npc>)>,
    mut people: Query<(&mut Npc, &mut Transform)>,
    mut movers: ResMut<Movers>,
) {
    let dt = time.delta_secs().min(0.05);
    let player = players
        .single()
        .map(|t| Vec2::new(t.translation.x, t.translation.z))
        .unwrap_or(Vec2::splat(f32::MAX));
    movers.people.clear();
    for (mut npc, mut transform) in &mut people {
        let (at, facing) = on_track(npc.lot, npc.along, npc.way);
        let ahead = Vec2::new(-facing.sin(), -facing.cos());
        let to_player = player - at;
        let blocked = to_player.length() < 1.3 && to_player.dot(ahead) > 0.0;
        npc.pause = (npc.pause - dt).max(0.0);
        npc.hop = (npc.hop - dt * 2.5).max(0.0);
        if !blocked && npc.pause == 0.0 {
            npc.along += npc.way * npc.speed * dt;
            npc.stride += npc.speed * dt * 2.6;
        }
        let (at, facing) = on_track(npc.lot, npc.along, npc.way);
        // Stopped by the character or spoken to, it turns to look at them.
        let facing = if npc.pause > 0.0 {
            (-to_player.x).atan2(-to_player.y)
        } else {
            facing
        };
        transform.translation = Vec3::new(at.x, gen::PAVEMENT + (npc.hop * PI).sin() * 0.18, at.y);
        transform.rotation = transform
            .rotation
            .slerp(Quat::from_rotation_y(facing), (dt * 10.0).min(1.0));
        movers.people.push(Mover {
            at,
            half: Vec2::splat(0.28),
            kind: SolidKind::Person,
            model: "person",
        });
    }
}

/// Walked into, or greeted: the nearest passer-by answers.
fn react(
    mut bumps: MessageReader<Bumped>,
    mut waves: MessageReader<Waved>,
    status: Res<crate::bridge::HostStatus>,
    mut people: Query<(&mut Npc, &Transform)>,
    mut said: MessageWriter<OtherSaid>,
    mut turn: Local<usize>,
) {
    let mut answer =
        |from: Vec2, within: f32, remark: Remark, people: &mut Query<(&mut Npc, &Transform)>| {
            let nearest = people
                .iter_mut()
                .map(|(npc, at)| {
                    (
                        Vec2::new(at.translation.x, at.translation.z).distance(from),
                        npc,
                        at.translation,
                    )
                })
                .filter(|(distance, ..)| *distance < within)
                .min_by(|a, b| a.0.total_cmp(&b.0));
            if let Some((_, mut npc, at)) = nearest {
                npc.pause = 2.6;
                npc.hop = 1.0;
                *turn += 1;
                said.write(OtherSaid {
                    text: passer_by(&status.lang, remark, *turn + npc.stride as usize).to_string(),
                    at: at + Vec3::Y * 1.75,
                });
            }
        };
    for bump in bumps.read() {
        if bump.into == SolidKind::Person {
            answer(bump.at, 1.6, Remark::WalkedInto, &mut people);
        }
    }
    for wave in waves.read() {
        answer(wave.at, 7.0, Remark::Greeted, &mut people);
    }
}

fn animate(people: Query<(&Npc, &Children)>, mut legs: Query<(&NpcLeg, &mut Transform)>) {
    for (npc, children) in &people {
        let swing = if npc.pause > 0.0 {
            0.0
        } else {
            npc.stride.sin() * 0.6
        };
        for child in children.iter() {
            if let Ok((leg, mut transform)) = legs.get_mut(child) {
                // Swung about the hip: the model's origin is its foot.
                let turn = Quat::from_rotation_x(leg.side * swing);
                transform.rotation = turn;
                transform.translation = leg.hip + turn * Vec3::new(0.0, -leg.length, 0.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_track_goes_round_the_lot_on_the_pavement() {
        let lot = Vec2::new(22.0, -44.0);
        let lap = TRACK * 8.0;
        let mut last = on_track(lot, 0.0, 1.0).0;
        let mut walked = 0.0;
        let mut along = 0.0;
        while along < lap {
            along += 0.25;
            let (at, _) = on_track(lot, along, 1.0);
            let off = (at - lot).abs();
            assert!((off.x.max(off.y) - TRACK).abs() < 1e-3, "{at:?}");
            walked += at.distance(last);
            last = at;
        }
        assert!((walked - lap).abs() < 0.5, "{walked} of {lap}");
        assert!(
            on_track(lot, lap, 1.0)
                .0
                .distance(on_track(lot, 0.0, 1.0).0)
                < 1e-3
        );
    }

    #[test]
    fn a_walker_faces_where_it_goes() {
        let lot = Vec2::ZERO;
        for way in [1.0, -1.0] {
            for along in [1.0, TRACK * 2.0 + 1.0, TRACK * 4.0 + 1.0, TRACK * 6.0 + 1.0] {
                let (at, facing) = on_track(lot, along, way);
                let (next, _) = on_track(lot, along + way * 0.1, way);
                let ahead = Vec2::new(-facing.sin(), -facing.cos());
                assert!(
                    (next - at).normalize().dot(ahead) > 0.99,
                    "along {along} way {way}"
                );
            }
        }
    }

    #[test]
    fn people_stay_clear_of_what_stands_on_the_pavement() {
        let sizes = crate::vox::sizes();
        for seed in [1u64, 2] {
            for x in -4..=4 {
                for z in -4..=4 {
                    let coord = IVec2::new(x, z);
                    let solids = gen::chunk_solids(&gen::chunk(seed, coord), &sizes);
                    let mut along = 0.0;
                    while along < TRACK * 8.0 {
                        let (at, _) = on_track(gen::centre(coord), along, 1.0);
                        assert!(
                            !solids.iter().any(|s| s.touches(at, 0.2)),
                            "seed {seed} {coord} at {at:?}"
                        );
                        along += 0.2;
                    }
                }
            }
        }
    }

    #[test]
    fn nobody_looks_like_anybody_else() {
        // Everyone in a wide stretch of city, for two seeds: no two alike.
        for seed in [3u64, 2026] {
            let people: Vec<Look> = (-30..30)
                .flat_map(|x| (-30..30).map(move |z| IVec2::new(x, z)))
                .flat_map(|chunk| residents(seed, chunk))
                .map(|r| r.look.visible())
                .collect();
            let different: std::collections::HashSet<&Look> = people.iter().collect();
            assert!(people.len() > 3000, "{} people", people.len());
            assert_eq!(
                different.len(),
                people.len(),
                "seed {seed}: {} alike among {}",
                people.len() - different.len(),
                people.len()
            );
        }
    }

    #[test]
    fn residents_come_from_the_seed() {
        assert_eq!(
            residents(3, IVec2::new(2, 5)),
            residents(3, IVec2::new(2, 5))
        );
        let total: usize = (0..100).map(|i| residents(3, IVec2::new(i, 0)).len()).sum();
        assert!((60..=140).contains(&total), "{total}");
    }
}
