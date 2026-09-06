//! `studio` — a trem-powered music studio plugin for Shiny.
//!
//! Compose rhythmic patterns (explicit `x..x` rhythms and Euclidean fills),
//! render them through trem audio graphs to WAV, and expose them to the AI
//! sphere via tools + REST routes and to the user via the Studio window.

pub mod engine;
pub mod fx;
pub mod grid;
pub mod plugin;
pub mod routes;
pub mod store;
pub mod tools;
pub mod voices;
pub mod wav;
