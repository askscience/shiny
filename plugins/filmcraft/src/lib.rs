//! The FilmCraft plugin.
//!
//! FilmCraft (https://github.com/storytold/filmcraft) is a non-linear video
//! editor written in Rust. It ships **two** front ends over one engine:
//!
//! * a native desktop app (`apps/filmcraft`), and
//! * a WebAssembly build of the *same* engine and the *same* egui UI
//!   (`apps/filmcraft-web`), which `packaging/web/README.md` documents as
//!   embeddable in an `<iframe>` and servable from any static path.
//!
//! This plugin hosts that WebAssembly build as an ordinary Shiny plugin
//! window: `web/` carries the built site (`filmcraft_web.js`,
//! `filmcraft_web_bg.wasm`) and `web/plugin.js` boots it on a canvas inside the
//! tile. The core already serves every plugin's `web/` directory at
//! `/plugins/<name>/` (see `src/api/mod.rs`), so no route is needed for the
//! app itself, and `script-src 'self' 'wasm-unsafe-eval'` in the core CSP
//! already permits the WebAssembly instantiation.
//!
//! # Two engines, two purposes
//!
//! | Surface | Where it runs | Used for |
//! |---|---|---|
//! | Window (browser tab) | the user's browser | editing, playback, colour, export the user sees |
//! | Headless engine | this process, a dedicated OS thread | batch work on server-side files: inspect, conform, long exports |
//!
//! The window is reached through [`relay`]: the assistant's tool calls are
//! queued per user, the window polls them and executes them with
//! `window.filmcraft.execute(...)` — the very same command ids FilmCraft's own
//! MCP server exposes.
//!
//! The headless engine ([`headless`]) never runs on the plugin runtime: the
//! plugin runtime worker is process-global and serial (`rt.rs`), so a ten-minute
//! export there would freeze every plugin in the process. Exports are started
//! and polled instead.

pub mod headless;
pub mod plugin;
pub mod relay;
pub mod routes;
pub mod tools;

pub use plugin::FilmCraftPlugin;