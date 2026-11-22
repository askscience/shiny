//! @@TITLE@@ — @@DESCRIPTION@@
//!
//! - `plugin.rs`: what the plugin registers (tools, routes, skill text);
//! - `tools.rs`: the assistant's tools, one struct each;
//! - `routes.rs`: the HTTP routes the window calls;
//! - `store.rs`: the plugin's data, shared by tools and routes.

pub mod plugin;
pub mod routes;
pub mod store;
pub mod tools;

pub use plugin::@@TYPE@@;
