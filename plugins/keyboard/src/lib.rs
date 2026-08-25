//! Keyboard plugin — pure surface plugin: a virtual multi-language keyboard
//! bar at the bottom of the screen. It contributes NO skills, tools or
//! persona — the AI never knows it exists; the frontend owns the keyboard UI.

pub mod plugin;

pub use plugin::KeyboardPlugin;
