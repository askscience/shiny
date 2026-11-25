//! `CameraPlugin`: third person, from behind and above, following softly.

use bevy::prelude::*;

use crate::city::START;
use crate::player::Player;

/// Behind and above the character, in its own frame.
const BEHIND: Vec3 = Vec3::new(0.0, 4.6, 8.2);
/// What the camera looks at: a little above the character's head, ahead.
const AIM: Vec3 = Vec3::new(0.0, 1.5, -2.0);

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

fn follow(
    time: Res<Time>,
    players: Query<(&Player, &Transform), Without<FollowCamera>>,
    mut cameras: Query<&mut Transform, With<FollowCamera>>,
) {
    let (Ok((player, at)), Ok(mut camera)) = (players.single(), cameras.single_mut()) else {
        return;
    };
    // Follow where the character is heading, not every wobble of its body.
    let frame = Quat::from_rotation_y(player.facing);
    let ground = Vec3::new(at.translation.x, 0.0, at.translation.z);
    let want = ground + frame * BEHIND;
    let ease = 1.0 - (-4.0 * time.delta_secs()).exp();
    camera.translation = camera.translation.lerp(want, ease);
    let aim = ground + frame * AIM;
    let look = camera.looking_at(aim, Vec3::Y).rotation;
    camera.rotation = camera.rotation.slerp(look, ease.max(0.2));
}
