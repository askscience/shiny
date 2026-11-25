//! The keyboard as a stand-in for the voice, for testing: the same commands,
//! through the same path.
//!
//! Arrows or WASD: forward, back, left, right. Space: stop. `+`/`-` (or
//! `E`/`Q`): faster, slower.

use bevy::prelude::*;

use crate::intent::Order;

pub struct KeyboardPlugin;

impl Plugin for KeyboardPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PreUpdate, keys);
    }
}

const KEYS: &[(KeyCode, Order)] = &[
    (KeyCode::ArrowUp, Order::Forward),
    (KeyCode::KeyW, Order::Forward),
    (KeyCode::ArrowDown, Order::Back),
    (KeyCode::KeyS, Order::Back),
    (KeyCode::ArrowLeft, Order::Left),
    (KeyCode::KeyA, Order::Left),
    (KeyCode::ArrowRight, Order::Right),
    (KeyCode::KeyD, Order::Right),
    (KeyCode::Space, Order::Stop),
    (KeyCode::KeyE, Order::Faster),
    (KeyCode::NumpadAdd, Order::Faster),
    (KeyCode::Equal, Order::Faster),
    (KeyCode::KeyQ, Order::Slower),
    (KeyCode::NumpadSubtract, Order::Slower),
    (KeyCode::Minus, Order::Slower),
];

fn keys(pressed: Res<ButtonInput<KeyCode>>, mut commands: MessageWriter<Order>) {
    for (key, command) in KEYS {
        if pressed.just_pressed(*key) {
            commands.write(*command);
        }
    }
}
