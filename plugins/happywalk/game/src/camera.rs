//! `CameraPlugin`: third person, from behind and above, following softly.
//! In a narrow street the camera comes closer instead of going through the
//! wall behind the character.

use bevy::prelude::*;

use crate::city::{SolidKind, Solids, START};
use crate::player::Player;

/// Behind and above the character, in its own frame.
const BEHIND: Vec3 = Vec3::new(0.0, 3.6, 6.6);
/// What the camera looks at: a little above the character's head, ahead.
const AIM: Vec3 = Vec3::new(0.0, 1.1, -2.0);

#[derive(Component)]
pub struct FollowCamera;

pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn)
            .add_systems(PostUpdate, follow.before(TransformSystems::Propagate));
    }
}

fn spawn(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        FollowCamera,
        Transform::from_translation(START + BEHIND).looking_at(START + AIM, Vec3::Y),
    ));
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
    time: Res<Time>,
    solids: Res<Solids>,
    mut room: Local<Option<f32>>,
    players: Query<(&Player, &Transform), Without<FollowCamera>>,
    mut cameras: Query<&mut Transform, With<FollowCamera>>,
) {
    let (Ok((player, at)), Ok(mut camera)) = (players.single(), cameras.single_mut()) else {
        return;
    };
    // Follow where the character is heading, not every wobble of its body.
    let frame = Quat::from_rotation_y(player.facing);
    let ground = Vec3::new(at.translation.x, 0.0, at.translation.z);
    let ease = 1.0 - (-4.0 * time.delta_secs()).exp();
    let back = frame * Vec3::new(BEHIND.x, 0.0, BEHIND.z);
    let clear = room_behind(
        &solids,
        Vec2::new(ground.x, ground.z),
        Vec2::new(back.x, back.z),
    );
    // Ease the distance too, so a doorway passing by does not make it jump.
    let room = room.get_or_insert(clear);
    *room += (clear - *room) * ease;
    // Closer means higher up the wall's side: look down a little more.
    let want = ground + back * *room + Vec3::Y * BEHIND.y * (0.75 + 0.25 * *room);
    camera.translation = camera.translation.lerp(want, ease);
    let aim = ground + frame * AIM;
    let look = camera.looking_at(aim, Vec3::Y).rotation;
    camera.rotation = camera.rotation.slerp(look, ease.max(0.2));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::city::{neighbourhood, solids, PITCH};

    #[test]
    fn the_camera_comes_closer_with_a_wall_behind() {
        let city = solids(&neighbourhood(), &crate::vox::sizes());
        // Looking up the street from the crossing: all the room there is.
        let open = room_behind(&city, Vec2::new(PITCH / 2.0, 0.0), Vec2::new(0.0, 6.6));
        assert_eq!(open, 1.0);
        // Facing a wall across the street, the wall behind is near.
        let tight = room_behind(&city, Vec2::new(PITCH / 2.0, 4.0), Vec2::new(6.6, 0.0));
        assert!(tight < 0.8, "{tight}");
        assert!(tight >= 0.3);
    }
}
