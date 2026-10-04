//! Updates — Linux system + Ollama update manager.
//!
//! The plugin detects the running distribution and dispatches to a dedicated
//! per-package-manager module (`distro/apt.rs`, `distro/dnf.rs`, `distro/
//! pacman.rs`, …). Everything else (command execution, jobs, routes, tools,
//! the window) is distro-agnostic.
//!
//! Privileged operations never use passwordless sudo: the caller supplies the
//! account's sudo password once per request, it is piped to `sudo -S` over
//! stdin, and it is never stored or logged.

pub mod distro;
pub mod exec;
pub mod history;
pub mod jobs;
pub mod model;
pub mod ollama;
pub mod ops;
pub mod plugin;
pub mod routes;
pub mod status;
pub mod tools;

pub use plugin::UpdatesPlugin;
