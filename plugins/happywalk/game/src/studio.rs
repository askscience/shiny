//! `StudioPlugin`: where the character is dressed.
//!
//! While the wardrobe is open the character stands on a podium in a studio
//! far from the city, before a painted backdrop of a pastel town
//! (`assets/backdrop.png`, drawn by `tools/make_backdrop.py`). The backdrop
//! hangs from the camera, so it always fills the view behind the figure,
//! whatever the camera does; it takes no light and no fog. When the wardrobe
//! closes the character goes back where it was.

use bevy::asset::RenderAssetUsages;
use bevy::image::{CompressedImageFormats, ImageSampler, ImageType};
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use crate::bridge::Wardrobe;
use crate::camera::FollowCamera;
use crate::player::Player;

/// Where the studio is: far beyond the fog of any city.
pub const STUDIO: Vec3 = Vec3::new(20_000.0, 0.0, 20_000.0);
/// How far behind the camera's view the backdrop hangs.
const BACKDROP_AT: f32 = 30.0;
/// The picture's own shape.
const BACKDROP_ASPECT: f32 = 16.0 / 9.0;

#[derive(Component)]
struct Backdrop;

#[derive(Component)]
struct Podium;

/// Where the character was before it went to be dressed.
#[derive(Resource, Default)]
struct Away(Option<(Vec3, Quat)>);

pub struct StudioPlugin;

impl Plugin for StudioPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Away>()
            .add_systems(PostStartup, build)
            .add_systems(Update, (go, fit).chain());
    }
}

/// The size of the backdrop that covers a view of `aspect` (width over
/// height) seen with a vertical field of view `fov`, `distance` away,
/// without changing the picture's shape (the edges are cut, never stretched).
pub fn cover(aspect: f32, fov: f32, distance: f32) -> Vec2 {
    let high = 2.0 * distance * (fov / 2.0).tan();
    let wide = high * aspect;
    if wide / high > BACKDROP_ASPECT {
        Vec2::new(wide, wide / BACKDROP_ASPECT)
    } else {
        Vec2::new(high * BACKDROP_ASPECT, high)
    }
}

fn build(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    cameras: Query<Entity, With<FollowCamera>>,
) {
    let picture = Image::from_buffer(
        include_bytes!("../assets/backdrop.png"),
        ImageType::Extension("png"),
        CompressedImageFormats::NONE,
        true,
        ImageSampler::linear(),
        RenderAssetUsages::RENDER_WORLD,
    );
    let picture = match picture {
        Ok(picture) => images.add(picture),
        Err(e) => {
            error!("the studio's backdrop could not be read: {e}");
            return;
        }
    };
    let backdrop = materials.add(StandardMaterial {
        base_color_texture: Some(picture),
        unlit: true,
        fog_enabled: false,
        ..default()
    });
    if let Ok(camera) = cameras.single() {
        commands.entity(camera).with_children(|view| {
            view.spawn((
                Backdrop,
                Mesh3d(meshes.add(Rectangle::new(1.0, 1.0))),
                MeshMaterial3d(backdrop),
                Transform::from_xyz(0.0, 0.0, -BACKDROP_AT),
                Visibility::Hidden,
                NotShadowCaster,
            ));
        });
    }
    // A round podium, two steps, in the game's pastels.
    let top = materials.add(StandardMaterial {
        base_color: Color::srgb_u8(253, 238, 200),
        perceptual_roughness: 0.9,
        ..default()
    });
    let step = materials.add(StandardMaterial {
        base_color: Color::srgb_u8(249, 205, 210),
        perceptual_roughness: 0.9,
        ..default()
    });
    commands.spawn((
        Podium,
        Mesh3d(meshes.add(Cylinder::new(0.8, 0.1))),
        MeshMaterial3d(step),
        Transform::from_translation(STUDIO + Vec3::Y * 0.05),
    ));
    commands.spawn((
        Podium,
        Mesh3d(meshes.add(Cylinder::new(0.62, 0.1))),
        MeshMaterial3d(top),
        Transform::from_translation(STUDIO + Vec3::Y * 0.15),
    ));
}

/// To the studio when the wardrobe opens, back when it closes.
fn go(
    wardrobe: Res<Wardrobe>,
    mut away: ResMut<Away>,
    mut players: Query<&mut Transform, With<Player>>,
    mut backdrops: Query<&mut Visibility, With<Backdrop>>,
) {
    if !wardrobe.is_changed() {
        return;
    }
    let Ok(mut player) = players.single_mut() else {
        return;
    };
    if wardrobe.0 {
        if away.0.is_none() {
            away.0 = Some((player.translation, player.rotation));
        }
        // On the podium, facing the camera's side of the studio.
        player.translation = STUDIO + Vec3::Y * 0.2;
    } else if let Some((at, turn)) = away.0.take() {
        player.translation = at;
        player.rotation = turn;
    }
    for mut visibility in &mut backdrops {
        *visibility = if wardrobe.0 {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
}

/// Keep the backdrop covering the view as the window changes shape.
fn fit(
    wardrobe: Res<Wardrobe>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<&Projection, With<FollowCamera>>,
    mut backdrops: Query<&mut Transform, With<Backdrop>>,
) {
    if !wardrobe.0 {
        return;
    }
    let (Ok(window), Ok(projection), Ok(mut backdrop)) =
        (windows.single(), cameras.single(), backdrops.single_mut())
    else {
        return;
    };
    let fov = match projection {
        Projection::Perspective(perspective) => perspective.fov,
        _ => std::f32::consts::FRAC_PI_4,
    };
    let aspect = window.width() / window.height().max(1.0);
    let size = cover(aspect, fov, BACKDROP_AT);
    backdrop.scale = Vec3::new(size.x, size.y, 1.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_backdrop_covers_any_window_without_stretching() {
        for aspect in [0.5, 1.0, 1.6, 16.0 / 9.0, 2.4] {
            let fov = std::f32::consts::FRAC_PI_4;
            let size = cover(aspect, fov, 10.0);
            let high = 2.0 * 10.0 * (fov / 2.0).tan();
            assert!(
                size.y >= high - 1e-3 && size.x >= high * aspect - 1e-3,
                "aspect {aspect}: {size:?}"
            );
            assert!((size.x / size.y - BACKDROP_ASPECT).abs() < 1e-3);
        }
    }

    #[test]
    fn the_studio_is_beyond_any_fog() {
        assert!(STUDIO.length() > 1000.0);
    }
}
