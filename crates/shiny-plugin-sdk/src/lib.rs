//! shiny Plugin SDK
//!
//! Defines the trait surface a plugin implements, the types it exchanges with the
//! core AI sphere, and a small set of helpers (parsing, parameter extraction,
//! migration runner). Plugins compile against this crate only; the binary loads
//! them via `libloading` at startup or on install.
//!
//! See `PLUGINS.md` at the repo root for the full authoring guide.

pub mod errors;
pub mod services;
pub mod artifacts;
pub mod navigation;
pub mod context;
pub mod outcome;
pub mod manifest;
pub mod rt;
pub mod tools;
pub mod routes;
pub mod crons;
pub mod migrations;
pub mod plugin;

pub use errors::AppError;
pub use services::{OllamaClient, SearchService, SupertonicClient};
pub use artifacts::{Artifact, PlanDay, PlanDayItem, RouteMeta};
pub use navigation::NavigationSession;
pub use context::AgentContext;
pub use outcome::ActionOutcome;
pub use manifest::Manifest;
pub use tools::{bridged, BridgedTool, Tool, ToolRequest, RegistryBuilder, ParamHelpers, parse_actions, strip_action_blocks, normalize_action_name};
pub use routes::{RouteSpec, HttpMethod};
pub use crons::{CronSpec, CronEntry};
pub use plugin::{Plugin, PluginEntry, PLUGIN_ENTRY_SYMBOL};

/// The core API level. Plugins declare `api_level` in `plugin.toml`; the loader
/// refuses to load a plugin built against a newer API than the running core.
pub const CORE_API_LEVEL: u32 = 1;

/// Format string for the C entry symbol. Plugins export a function that takes
/// no arguments and returns a heap-allocated `*mut dyn Plugin`.
pub fn entry_symbol(name: &str) -> String {
    // Plugins built as cdylibs expose `shiny_plugin_entry`. We don't actually
    // need a per-plugin symbol because each plugin is its own .so/.dylib/.dll —
    // one symbol per library is enough.
    let _ = name;
    PLUGIN_ENTRY_SYMBOL.to_string()
}