//! Happy Walk: a detective story in a pastel voxel city. The character is
//! the assistant of a detective who is never seen (the player): clients
//! come to the office, suspects are found in their shops and homes, and are
//! questioned in a real chat with the user's language model playing them.
//! This crate is the game; it is built for the web and runs in the
//! `happywalk` plugin's window, where the page (`web/host.js`) connects it to
//! LiteAura's language model and speech recognition.
//!
//! One Bevy plugin per module:
//!
//! * [`bridge`] — `LiteAuraBridge`: the page's calls in, requests to the
//!   language model out. Nothing here blocks a frame.
//! * [`telemetry`] — how long the model takes, and what is drawn.
//! * [`control`] — keys, a touch joystick, a tap on the ground.
//! * [`vox`] — voxel models: `.vox` files meshed greedily, found by name.
//! * [`gen`] — the city generator: a seed in, lots, props and places out.
//! * [`case`] — the mystery generator: a fair case for a city and a level.
//! * [`city`] — the chunks of the city around the player, and its solids.
//! * [`rooms`] — inside places, built when entered: the office, shops, homes.
//! * [`sky`] — the light: pastel sky, warm sun, day turning to evening.
//! * [`figure`] — the character and the passers-by, sculpted from a look.
//! * [`player`] — the character: walking, turning, bumping.
//! * [`camera`] — third person, following softly; into rooms from the front.
//! * [`studio`] — where the character is dressed: a podium before a painted town.
//! * [`movers`] — where everything that moves is, for whoever must avoid it.
//! * [`npc`] — passers-by on the pavements.
//! * [`traffic`] — cars and bikes that always stop for the character.
//! * [`nav`] — a tap on the ground: the way there, walked alone.
//! * [`mystery`] — the case in play: the office, the people, the chat, the accusation.
//! * [`hud`] — bubbles over whoever answers, notices, timings.

// Bevy's queries are long types and its systems take what they use.
#![allow(
    clippy::type_complexity,
    clippy::too_many_arguments,
    clippy::chunks_exact_to_as_chunks
)]

pub mod bridge;
pub mod camera;
pub mod case;
pub mod city;
pub mod control;
pub mod figure;
pub mod gen;
pub mod hud;
pub mod movers;
pub mod mystery;
pub mod nav;
pub mod npc;
pub mod player;
pub mod rooms;
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
        control::ControlPlugin,
        vox::VoxPlugin,
        sky::SkyPlugin,
        city::CityPlugin,
        player::PlayerPlugin,
        camera::CameraPlugin,
    ))
    // A tuple holds fifteen plugins at most: the rest in a second one.
    .add_plugins((
        studio::StudioPlugin,
        mystery::MysteryPlugin,
        npc::NpcPlugin,
        traffic::TrafficPlugin,
        nav::NavPlugin,
        rooms::RoomsPlugin,
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
