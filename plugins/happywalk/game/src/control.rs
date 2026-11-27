//! `ControlPlugin`: how the player moves the character.
//!
//! * **Keys held**: arrows or WASD walk forward and back and turn; Shift
//!   runs. `H` waves, `C` keeps a clip.
//! * **A joystick** on a touch screen, drawn by the page (`hw_steer`): up
//!   and down walk, left and right turn, pushed to the edge it runs.
//! * **A tap or a click on the ground** (`hw_tap`): the character walks
//!   there by itself (`nav.rs`).
//!
//! Every way ends up in [`Steer`] (held controls) or [`TapTo`] (a place to
//! walk to); the player module turns them into steps.

use std::sync::Mutex;

use bevy::prelude::*;

use crate::camera::FollowCamera;

/// Something to do once.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Order {
    /// Greet whoever is near.
    Wave,
    /// Keep the last seconds of play.
    Clip,
}

/// The held controls: how much forward (-1 back .. 1), how much turning
/// (-1 left .. 1 right), and whether to run.
#[derive(Resource, Default, Clone, Copy, Debug, PartialEq)]
pub struct Steer {
    pub forward: f32,
    pub turn: f32,
    pub run: bool,
}

impl Steer {
    /// Is anything held?
    pub fn active(&self) -> bool {
        self.forward.abs() > DEAD || self.turn.abs() > DEAD
    }
}

/// Below this a joystick counts as at rest.
pub const DEAD: f32 = 0.15;

/// Walk to this point on the ground.
#[derive(Message, Clone, Copy, Debug, PartialEq)]
pub struct TapTo(pub Vec2);

/// The page's joystick, as last reported: forward, turn.
static JOYSTICK: Mutex<(f32, f32)> = Mutex::new((0.0, 0.0));
/// Where the page was tapped, in CSS pixels from the canvas's top left.
static TAPPED: Mutex<Option<Vec2>> = Mutex::new(None);

/// The joystick moved (from the page): forward and turn, each -1..1.
pub fn steer(forward: f32, turn: f32) {
    if let Ok(mut joystick) = JOYSTICK.lock() {
        *joystick = (forward.clamp(-1.0, 1.0), turn.clamp(-1.0, 1.0));
    }
}

/// The ground was tapped (from the page), at this point of the canvas.
pub fn tap(x: f32, y: f32) {
    if let Ok(mut tapped) = TAPPED.lock() {
        *tapped = Some(Vec2::new(x, y));
    }
}

pub struct ControlPlugin;

impl Plugin for ControlPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Steer>()
            .add_message::<Order>()
            .add_message::<TapTo>()
            .add_systems(PreUpdate, (held, taps));
    }
}

/// The keys and the joystick, as one set of held controls.
fn held(
    pressed: Res<ButtonInput<KeyCode>>,
    wardrobe: Res<crate::bridge::Wardrobe>,
    typing: Res<crate::bridge::Typing>,
    mut steer: ResMut<Steer>,
    mut orders: MessageWriter<Order>,
) {
    let mut next = Steer::default();
    // While the player types in the chat, letters are not commands.
    if !wardrobe.0 && !typing.0 {
        let any = |keys: &[KeyCode]| pressed.any_pressed(keys.iter().copied());
        if any(&[KeyCode::ArrowUp, KeyCode::KeyW]) {
            next.forward += 1.0;
        }
        if any(&[KeyCode::ArrowDown, KeyCode::KeyS]) {
            next.forward -= 1.0;
        }
        if any(&[KeyCode::ArrowLeft, KeyCode::KeyA]) {
            next.turn -= 1.0;
        }
        if any(&[KeyCode::ArrowRight, KeyCode::KeyD]) {
            next.turn += 1.0;
        }
        next.run = any(&[KeyCode::ShiftLeft, KeyCode::ShiftRight]);
        if pressed.just_pressed(KeyCode::KeyH) {
            orders.write(Order::Wave);
        }
        if pressed.just_pressed(KeyCode::KeyC) {
            orders.write(Order::Clip);
        }
    }
    let (forward, turn) = JOYSTICK.lock().map(|j| *j).unwrap_or_default();
    if !wardrobe.0 && (forward.abs() > DEAD || turn.abs() > DEAD) {
        next.forward = forward;
        next.turn = turn;
        next.run = Vec2::new(forward, turn).length() > 0.9;
    }
    if *steer != next {
        *steer = next;
    }
}

/// A tap on the canvas: the point of the ground under it.
fn taps(
    cameras: Query<(&Camera, &GlobalTransform), With<FollowCamera>>,
    wardrobe: Res<crate::bridge::Wardrobe>,
    mut to: MessageWriter<TapTo>,
) {
    let Some(screen) = TAPPED.lock().ok().and_then(|mut t| t.take()) else {
        return;
    };
    let Ok((camera, view)) = cameras.single() else {
        return;
    };
    if wardrobe.0 {
        return;
    }
    if let Some(point) = ground_under(camera, view, screen) {
        to.write(TapTo(point));
    }
}

/// Where a ray through a point of the screen meets the ground.
pub fn ground_under(camera: &Camera, view: &GlobalTransform, screen: Vec2) -> Option<Vec2> {
    let ray = camera.viewport_to_world(view, screen).ok()?;
    let distance = ray.intersect_plane(Vec3::ZERO, InfinitePlane3d::new(Vec3::Y))?;
    let point = ray.get_point(distance);
    Some(Vec2::new(point.x, point.z))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_joystick_at_rest_holds_nothing() {
        assert!(!Steer::default().active());
        assert!(!Steer {
            forward: 0.1,
            turn: -0.1,
            run: false
        }
        .active());
        assert!(Steer {
            forward: 0.5,
            ..default()
        }
        .active());
    }
}
