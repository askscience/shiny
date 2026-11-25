//! Happy Walk: a third-person walk through a pastel voxel city, steered only
//! by voice. This crate is the game; it is built for the web and runs in the
//! `happywalk` plugin's window, where the page (`web/host.js`) connects it to
//! LiteAura's speech recognition, its language model and its voice.
//!
//! One Bevy plugin per module:
//!
//! * [`bridge`] — `LiteAuraBridge`: recognised speech in, requests to the
//!   language model and text to speak out. Nothing here blocks a frame.
//! * [`telemetry`] — the latency of every link of that chain, logged.
//! * [`intent`] — the known commands, straight from the recognised words.
//! * [`keyboard`] — the same commands from keys, for testing.
//! * [`city`] — the neighbourhood and everything in it that is solid.
//! * [`player`] — the character: walking, turning, bumping.
//! * [`camera`] — third person, following softly.
//! * [`brain`] — what the character says, asked of the user's language model.
//! * [`lines`] — stand-in lines for when the model cannot answer.
//! * [`hud`] — the speech bubble, "I am listening", what was understood.

// Bevy's queries are long types by nature.
#![allow(clippy::type_complexity)]

pub mod brain;
pub mod bridge;
pub mod camera;
pub mod city;
pub mod hud;
pub mod intent;
pub mod keyboard;
pub mod lines;
pub mod player;
pub mod telemetry;

use bevy::prelude::*;

/// The canvas the page provides for the game.
pub const CANVAS: &str = "#happywalk-canvas";

pub fn app() -> App {
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Happy Walk".into(),
                    canvas: Some(CANVAS.into()),
                    fit_canvas_to_parent: true,
                    // The page keeps its own shortcuts (and the shell its own).
                    prevent_default_event_handling: false,
                    ..default()
                }),
                ..default()
            })
            .set(bevy::log::LogPlugin {
                level: bevy::log::Level::INFO,
                ..default()
            }),
    )
    .add_plugins((
        telemetry::TelemetryPlugin,
        bridge::LiteAuraBridgePlugin,
        intent::IntentPlugin,
        keyboard::KeyboardPlugin,
        city::CityPlugin,
        player::PlayerPlugin,
        camera::CameraPlugin,
        brain::BrainPlugin,
        hud::HudPlugin,
    ));
    app
}

/// Called by the page once the module is loaded.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hw_start() {
    app().run();
}
