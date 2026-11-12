//! Oorbit plugin: a slow, real-time planet game. The cdylib owns the game's
//! state and rules (routes and agent tools); the window is a Godot web export
//! framed by the shell (`kind = "hybrid"` in `plugin.toml`).

pub mod plugin;
pub mod routes;
pub mod store;

pub use plugin::OorbitPlugin;
