//! Happy Walk plugin: a voice-steered walk through a voxel city. The window
//! is a Bevy game built for the web (`game/`, framed by the shell:
//! `kind = "hybrid"`); this process is the game's way to the language model
//! the user chose in Settings, which plays the character's brain.

pub mod brain;
pub mod plugin;
pub mod routes;

pub use plugin::HappyWalkPlugin;
