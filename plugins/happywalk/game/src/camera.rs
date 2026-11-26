//! `CameraPlugin`: third person, from behind and above, following softly.
//! In a narrow street the camera comes closer instead of going through the
//! wall behind the character.

use bevy::prelude::*;

use crate::bridge::Wardrobe;
use crate::city::{SolidKind, Solids, START};
use crate::player::{Bumped, Player};

/// Behind and above the character, in its own frame.
const BEHIND: Vec3 = Vec3::new(0.0, 3.6, 6.6);
/// What the camera looks at: a little above the character's head, ahead.
const AIM: Vec3 = Vec3::new(0.0, 1.1, -2.0);

#[derive(Component)]
pub struct FollowCamera;

/// What the camera is making of the moment.
#[derive(Resource, Default, Debug)]
pub struct Direction {
    /// Seconds left of the close, slowed look at a bump.
    pub impact: f32,
    /// Photo mode: the game stands still, the HUD is away, the camera goes
    /// slowly round the character.
    pub photo: bool,
    orbit: f32,
}

/// A soft light from the camera, on only in the wardrobe: there the camera
/// is in front of the figure, which is often the side away from the sun.
#[derive(Component)]
struct Fill;

/// How long the look at a bump lasts, and how slow the first part of it is.
const IMPACT: f32 = 1.1;
const SLOW: f32 = 0.35;

pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Direction>()
            .add_systems(Update, direct)
            .add_systems(Startup, spawn)
            .add_systems(PostUpdate, follow.before(TransformSystems::Propagate));
    }
}

fn spawn(mut commands: Commands) {
    commands.spawn((
        Fill,
        DirectionalLight {
            illuminance: 0.0,
            shadow_maps_enabled: false,
            color: Color::srgb(1.0, 0.97, 0.92),
            ..default()
        },
        Transform::default(),
    ));
    commands.spawn((
        Camera3d::default(),
        FollowCamera,
        // The city has no edge; the fog is where it stops being drawn
        // (`sky.rs` keeps it the colour of the sky).
        DistanceFog {
            falloff: FogFalloff::Linear {
                start: 30.0,
                end: 54.0,
            },
            ..default()
        },
        Transform::from_translation(START + BEHIND).looking_at(START + AIM, Vec3::Y),
    ));
}

/// Bumps get a close, slowed look; `P` is photo mode.
fn direct(
    real: Res<Time<Real>>,
    mut clock: ResMut<Time<Virtual>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut bumps: MessageReader<Bumped>,
    mut direction: ResMut<Direction>,
) {
    if bumps.read().count() > 0 {
        direction.impact = IMPACT;
    }
    if keys.just_pressed(KeyCode::KeyP) {
        direction.photo = !direction.photo;
    }
    let dt = real.delta_secs();
    direction.impact = (direction.impact - dt).max(0.0);
    if direction.photo {
        direction.orbit += dt * 0.35;
    } else {
        direction.orbit = 0.0;
    }
    // Slow for the first half of the look, then back up to speed.
    let speed = if direction.photo {
        0.0
    } else if direction.impact > IMPACT / 2.0 {
        SLOW
    } else {
        1.0
    };
    if (clock.relative_speed() - speed).abs() > 0.01 {
        if speed == 0.0 {
            clock.pause();
        } else {
            clock.unpause();
            clock.set_relative_speed(speed);
        }
    }
}

/// How far back the camera can sit (0..1 of [`BEHIND`]) before the line
/// from the character to it meets a building.
pub fn room_behind(solids: &Solids, from: Vec2, back: Vec2) -> f32 {
    const STEPS: u32 = 16;
    // Never closer than this: the character must stay in the picture.
    const CLOSEST: f32 = 0.3;
    let mut clear = CLOSEST;
    for step in 1..=STEPS {
        let t = step as f32 / STEPS as f32;
        if solids.hit(from + back * t, 0.6) == Some(SolidKind::Building) {
            break;
        }
        clear = clear.max(t);
    }
    clear
}

fn follow(
    time: Res<Time<Real>>,
    direction: Res<Direction>,
    wardrobe: Res<Wardrobe>,
    solids: Res<Solids>,
    mut room: Local<Option<f32>>,
    players: Query<(&Player, &Transform), Without<FollowCamera>>,
    mut cameras: Query<&mut Transform, With<FollowCamera>>,
    mut fills: Query<
        (&mut DirectionalLight, &mut Transform),
        (With<Fill>, Without<FollowCamera>, Without<Player>),
    >,
) {
    let (Ok((player, at)), Ok(mut camera)) = (players.single(), cameras.single_mut()) else {
        return;
    };
    // Follow where the character is heading, not every wobble of its body.
    // In photo mode it goes round; otherwise it stays behind.
    let frame = Quat::from_rotation_y(player.facing + direction.orbit);
    let ground = Vec3::new(at.translation.x, 0.0, at.translation.z);
    let ease = 1.0 - (-4.0 * time.delta_secs()).exp();
    // To or from the studio: a cut, not a flight across the map.
    let ease = if camera.translation.distance(at.translation) > 60.0 {
        1.0
    } else {
        ease
    };
    if let Ok((mut fill, mut aim)) = fills.single_mut() {
        let want = if wardrobe.0 { 6500.0 } else { 0.0 };
        if fill.illuminance != want {
            fill.illuminance = want;
        }
        if wardrobe.0 {
            // It shines the way the camera looks.
            aim.rotation = camera.rotation;
        }
    }
    if wardrobe.0 {
        // The wardrobe: from the front, close, the figure a little to one
        // side so the choices have the other, swaying slowly to show it
        // round.
        let sway = (time.elapsed_secs() * 0.5).sin() * 0.5;
        let frame = Quat::from_rotation_y(player.facing + sway);
        let ground = Vec3::new(at.translation.x, 0.0, at.translation.z);
        let want = ground + frame * Vec3::new(0.0, 1.25, -3.3);
        camera.translation = camera.translation.lerp(want, ease);
        let aside = Quat::from_rotation_y(player.facing) * Vec3::new(-0.75, 0.0, 0.0);
        let look = camera
            .looking_at(ground + Vec3::Y * 0.8 + aside, Vec3::Y)
            .rotation;
        camera.rotation = camera.rotation.slerp(look, ease.max(0.2));
        return;
    }
    // The look at a bump comes in close and low.
    let close = 1.0 - 0.42 * (direction.impact / IMPACT).min(1.0);
    let back = frame * Vec3::new(BEHIND.x, 0.0, BEHIND.z) * close;
    let clear = room_behind(
        &solids,
        Vec2::new(ground.x, ground.z),
        Vec2::new(back.x, back.z),
    );
    // Ease the distance too, so a doorway passing by does not make it jump.
    let room = room.get_or_insert(clear);
    *room += (clear - *room) * ease;
    // Closer means higher up the wall's side: look down a little more.
    let want = ground + back * *room + Vec3::Y * BEHIND.y * (0.75 + 0.25 * *room) * close;
    camera.translation = camera.translation.lerp(want, ease);
    let aim = ground + frame * AIM;
    let look = camera.looking_at(aim, Vec3::Y).rotation;
    camera.rotation = camera.rotation.slerp(look, ease.max(0.2));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::city::{a_built_lot, PITCH};
    use crate::gen;

    #[test]
    fn the_camera_comes_closer_with_a_wall_behind() {
        let seed = 4;
        let city = Solids::new(seed);
        // Looking up the street from the crossing: all the room there is.
        let open = room_behind(&city, Vec2::new(PITCH / 2.0, 0.0), Vec2::new(0.0, 6.6));
        assert_eq!(open, 1.0);
        // On the street with its back to a building: the wall is near.
        let lot = gen::centre(a_built_lot(seed));
        let tight = room_behind(
            &city,
            lot + Vec2::new(PITCH / 2.0, -1.5),
            Vec2::new(-6.6, 0.0),
        );
        assert!(tight < 0.8, "{tight}");
        assert!(tight >= 0.3);
    }
}
