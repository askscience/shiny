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
//! * [`vox`] — voxel models: `.vox` files meshed greedily, found by name.
//! * [`gen`] — the city generator: a seed in, lots, props and places out.
//! * [`case`] — the mystery generator: a fair case for a city and a level.
//! * [`city`] — the chunks of the city around the player, and its solids.
//! * [`sky`] — the light: pastel sky, soft sun, day turning to evening.
//! * [`figure`] — the character and the passers-by, sculpted from a look.
//! * [`player`] — the character: walking, turning, bumping.
//! * [`camera`] — third person, following softly.
//! * [`studio`] — where the character is dressed: a podium before a painted town.
//! * [`movers`] — where everything that moves is, for whoever must avoid it.
//! * [`npc`] — passers-by on the pavements.
//! * [`traffic`] — cars and bikes that always stop for the character.
//! * [`brain`] — what the character says, asked of the user's language model.
//! * [`nav`] — "go to the bakery": routes along the streets, walked alone.
//! * [`quest`] — errands: small things to do, five to a round.
//! * [`lines`] — stand-in lines for when the model cannot answer.
//! * [`hud`] — the speech bubble, "I am listening", what was understood.

// Bevy's queries are long types and its systems take what they use.
#![allow(
    clippy::type_complexity,
    clippy::too_many_arguments,
    clippy::chunks_exact_to_as_chunks
)]

pub mod brain;
pub mod bridge;
pub mod camera;
pub mod case;
pub mod city;
pub mod figure;
pub mod gen;
pub mod hud;
pub mod intent;
pub mod keyboard;
pub mod lines;
pub mod movers;
pub mod nav;
pub mod npc;
pub mod player;
pub mod quest;
pub mod sky;
pub mod studio;
pub mod telemetry;
pub mod traffic;
pub mod vox;

use bevy::prelude::*;

/// The canvas the page provides for the game.
pub const CANVAS: &str = "#happywalk-canvas";

/// The most device pixels drawn per CSS pixel. A high-density screen would
/// otherwise have the game shade four or more times the pixels for a look
/// that is made of big flat squares anyway.
pub const MAX_PIXEL_RATIO: f32 = 1.5;

/// The game, in the city grown from `seed`, on a screen of `pixel_ratio`
/// device pixels per CSS pixel.
pub fn app(seed: u64, pixel_ratio: f32) -> App {
    let mut app = App::new();
    app.insert_resource(city::CitySeed(seed));
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Happy Walk".into(),
                    resolution: bevy::window::WindowResolution::default()
                        .with_scale_factor_override(pixel_ratio.clamp(1.0, MAX_PIXEL_RATIO)),
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
        vox::VoxPlugin,
        sky::SkyPlugin,
        city::CityPlugin,
        player::PlayerPlugin,
        camera::CameraPlugin,
    ))
    // A tuple holds fifteen plugins at most: the rest in a second one.
    .add_plugins((
        studio::StudioPlugin,
        npc::NpcPlugin,
        traffic::TrafficPlugin,
        nav::NavPlugin,
        quest::QuestPlugin,
        brain::BrainPlugin,
        hud::HudPlugin,
    ));
    app
}

/// Called by the page once the module is loaded, with the city's seed and
/// the screen's pixel ratio.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hw_start(seed: u32, pixel_ratio: f32) {
    app(seed as u64, pixel_ratio).run();
}
