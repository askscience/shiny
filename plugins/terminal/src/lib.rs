//! Terminal plugin — a real Linux login shell in its own window.
//!
//! The Rust half owns PTY sessions and streams their output over SSE; the web
//! half (`web/plugin.js`) renders it with xterm.js and posts keystrokes back.

pub mod plugin;
pub mod pty;
pub mod routes;

pub use plugin::TerminalPlugin;
