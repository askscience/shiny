//! `browser` plugin — an adblock-filtered web browser window inside Shiny.
//!
//! Modules:
//! * [`proxy`] — owns the filter proxy the window is rendered through.
//! * [`routes`] — the REST surface the window's chrome talks to.
//! * [`tools`] — the agent tools (`browser_open`, `browser_search`,
//!   `browser_read`).
//! * [`fetch`] — page → readable text, for the tools.
//! * [`history`] — best-effort browsing history.
//! * [`plugin`] — registration and the `shiny_plugin_entry` symbol.

pub mod fetch;
pub mod history;
pub mod plugin;
pub mod proxy;
pub mod routes;
pub mod tools;

pub use plugin::PeakdBrowserPlugin;
