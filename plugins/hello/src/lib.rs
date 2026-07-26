//! Demo "Hello" plugin — a single trivial tool to verify the plugin pipeline.
//!
//! Registers one tool: `hello`. The AI sphere can call:
//!   {"action":"hello","params":{"name":"world"}}
//! and gets back `Hello, world!`.

pub mod plugin;
pub mod tool;

pub use plugin::HelloPlugin;