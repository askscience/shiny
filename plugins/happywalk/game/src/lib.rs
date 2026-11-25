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
//! * [`scene`] — what is on screen (for now: proof that frames are drawn).
//! * [`hud`] — "I am listening", what was understood, and the timings.

pub mod bridge;
pub mod hud;
pub mod scene;
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
        scene::ScenePlugin,
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
