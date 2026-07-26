//! Plugin system: tool registry, plugin loader, hot-reloadable router merge.

pub mod manager;
pub mod registry;
pub mod loader;
pub mod installer;
pub mod admin_api;

pub use manager::PluginManager;
pub use registry::ToolRegistry;