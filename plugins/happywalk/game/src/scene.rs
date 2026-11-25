//! What is on screen. For now a ground, a light and one block that turns,
//! which is enough to see that frames keep coming while the host works.

use bevy::prelude::*;

pub struct ScenePlugin;

impl Plugin for ScenePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ClearColor(Color::srgb(0.80, 0.90, 0.96)))
            .add_systems(Startup, setup)
            .add_systems(Update, turn);
    }
}

#[derive(Component)]
struct Turning;

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(4.0, 4.5, 7.0).looking_at(Vec3::new(0.0, 0.8, 0.0), Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 9000.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(3.0, 8.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(24.0, 24.0))),
        MeshMaterial3d(materials.add(Color::srgb(0.74, 0.88, 0.76))),
    ));
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(1.2, 1.2, 1.2))),
        MeshMaterial3d(materials.add(Color::srgb(0.98, 0.76, 0.80))),
        Transform::from_xyz(0.0, 0.9, 0.0),
        Turning,
    ));
}

fn turn(time: Res<Time>, mut blocks: Query<&mut Transform, With<Turning>>) {
    for mut transform in &mut blocks {
        transform.rotate_y(time.delta_secs() * 0.8);
    }
}
