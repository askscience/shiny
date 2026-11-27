//! `SkyPlugin`: the light. A pastel sky, a soft sun and a slow turn from day
//! to evening and back, all from one number ([`Daylight`]).

use bevy::light::CascadeShadowConfigBuilder;
use bevy::prelude::*;

/// Where the day is: 0 is full day, 1 is evening.
#[derive(Resource, Clone, Copy, Debug)]
pub struct Daylight {
    pub evening: f32,
    /// Seconds for one whole day-evening-day turn; 0 stops the clock.
    pub cycle: f32,
}

impl Default for Daylight {
    fn default() -> Self {
        Self {
            evening: 0.0,
            cycle: 300.0,
        }
    }
}

/// How much evening there is `seconds` into a cycle: a long day, a slow
/// dusk, a while of evening, and back.
pub fn evening_at(seconds: f32, cycle: f32) -> f32 {
    if cycle <= 0.0 {
        return 0.0;
    }
    let t = (seconds / cycle).rem_euclid(1.0);
    let ramp = |from: f32, to: f32| ((t - from) / (to - from)).clamp(0.0, 1.0);
    let smooth = |x: f32| x * x * (3.0 - 2.0 * x);
    smooth(ramp(0.45, 0.65)) - smooth(ramp(0.85, 1.0))
}

/// How far from the camera shadows are drawn, in metres.
pub const SHADOWS_TO: f32 = 38.0;

#[derive(Component)]
struct Sun;

pub struct SkyPlugin;

impl Plugin for SkyPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Daylight>()
            .insert_resource(ClearColor(DAY.sky))
            // Shadows reach 38 m: a map half the default size is sharp
            // enough for blocky shapes, and a quarter of the work.
            .insert_resource(bevy::light::DirectionalLightShadowMap { size: 1024 })
            .add_systems(Startup, setup)
            .add_systems(Update, turn);
    }
}

struct Look {
    sky: Color,
    sun: Color,
    illuminance: f32,
    ambient: Color,
    brightness: f32,
    /// The sun's height above the horizon, in radians.
    height: f32,
}

// A warm sun and a cool, weaker sky light: faces in the sun are warm, faces
// in shade lean blue-violet, so every block reads as a volume.
const DAY: Look = Look {
    sky: Color::srgb(0.66, 0.84, 0.99),
    sun: Color::srgb(1.0, 0.92, 0.78),
    illuminance: 12500.0,
    ambient: Color::srgb(0.70, 0.76, 1.0),
    brightness: 640.0,
    height: 0.95,
};

const EVENING: Look = Look {
    sky: Color::srgb(0.98, 0.78, 0.74),
    sun: Color::srgb(1.0, 0.72, 0.52),
    illuminance: 7600.0,
    ambient: Color::srgb(0.76, 0.66, 1.0),
    brightness: 560.0,
    height: 0.32,
};

fn setup(mut commands: Commands) {
    commands.spawn((
        Sun,
        DirectionalLight {
            shadow_maps_enabled: true,
            ..default()
        },
        // Shadows are the dearest thing drawn (the city is rendered twice):
        // one cascade, and only as far as a shadow can be told from haze.
        CascadeShadowConfigBuilder {
            num_cascades: 1,
            maximum_distance: SHADOWS_TO,
            ..default()
        }
        .build(),
        Transform::default(),
    ));
    commands.insert_resource(GlobalAmbientLight {
        color: DAY.ambient,
        brightness: DAY.brightness,
        ..default()
    });
}

fn turn(
    time: Res<Time>,
    mut daylight: ResMut<Daylight>,
    mut sky: ResMut<ClearColor>,
    mut ambient: ResMut<GlobalAmbientLight>,
    mut suns: Query<(&mut DirectionalLight, &mut Transform), With<Sun>>,
    mut fogs: Query<&mut DistanceFog>,
    wardrobe: Res<crate::bridge::Wardrobe>,
    cameras: Query<&Transform, (With<crate::camera::FollowCamera>, Without<Sun>)>,
) {
    daylight.evening = evening_at(time.elapsed_secs(), daylight.cycle);
    let e = daylight.evening;
    let mix = |a: Color, b: Color| a.mix(&b, e);
    let lerp = |a: f32, b: f32| a + (b - a) * e;
    sky.0 = mix(DAY.sky, EVENING.sky);
    for mut fog in &mut fogs {
        fog.color = sky.0;
    }
    ambient.color = mix(DAY.ambient, EVENING.ambient);
    ambient.brightness = lerp(DAY.brightness, EVENING.brightness);
    if let Ok((mut sun, mut transform)) = suns.single_mut() {
        sun.color = mix(DAY.sun, EVENING.sun);
        sun.illuminance = lerp(DAY.illuminance, EVENING.illuminance);
        // The sun goes down towards the west as the evening comes.
        let height = lerp(DAY.height, EVENING.height);
        let around = lerp(0.6, 1.5);
        let from = Vec3::new(
            around.cos() * height.cos(),
            height.sin(),
            around.sin() * height.cos(),
        );
        *transform = Transform::from_translation(from * 50.0).looking_at(Vec3::ZERO, Vec3::Y);
        // In the wardrobe it lights the figure from where the camera looks,
        // a little from above: a studio light. (WebGL2 takes one directional
        // light well; a second one, even dark, breaks the sun's shadows.)
        if wardrobe.0 {
            if let Ok(camera) = cameras.single() {
                transform.rotation = camera.rotation * Quat::from_rotation_x(-0.35);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_day_turns_to_evening_and_back() {
        assert_eq!(evening_at(0.0, 100.0), 0.0);
        assert_eq!(evening_at(40.0, 100.0), 0.0);
        assert!((evening_at(55.0, 100.0) - 0.5).abs() < 0.01);
        assert_eq!(evening_at(75.0, 100.0), 1.0);
        assert!(evening_at(95.0, 100.0) < 0.5);
        assert_eq!(evening_at(100.0, 100.0), 0.0);
        assert_eq!(evening_at(42.0, 0.0), 0.0);
    }
}
