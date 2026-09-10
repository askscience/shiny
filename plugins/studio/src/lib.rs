//! `studio` — a music studio plugin for Shiny.
//!
//! Compose rhythmic patterns (explicit `x..x` rhythms and Euclidean fills) and
//! arrangements, render them through the plugin's own DSP engine (see
//! [`dsp`]) to WAV, and expose them to the AI sphere via tools + REST routes
//! and to the user via the Studio window.

pub mod catalog;
pub mod dsp;
pub mod engine;
pub mod fx;
pub mod grid;
pub mod plugin;
pub mod routes;
pub mod store;
pub mod tools;
pub mod voices;
pub mod wav;
