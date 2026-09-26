//! `browser` plugin — a web browser window inside Shiny.
//!
//! The page is rendered by the shell in a native child webview
//! (`crates/peakd`'s `browse` module), so the plugin owns only the window's
//! chrome, its sessions, its history and the news home surface.
//!
//! Modules:
//! * [`sessions`] — the window's tabs, as seen by the plugin.
//! * [`routes`] — the REST surface the window's chrome talks to.
//! * [`tools`] — the agent tools (`browser_open`, `browser_search`,
//!   `browser_read`).
//! * [`fetch`] — page → readable text, for the tools.
//! * [`history`] — best-effort browsing history, and the interest profile's
//!   raw material.
//! * [`settings`] — per-user Browser settings (the shield toggle).
//! * [`filter`] — the server half of the ad-filter engine: fetch + compile the
//!   lists and write the cache the shell reads.
//! * [`downloads`] — persisted download history for the Downloads panel.
//! * [`news`] — the related-news cards on the window's home surface, ranked
//!   from what the user searches for.
//! * [`plugin`] — registration and the `shiny_plugin_entry` symbol.

pub mod downloads;
pub mod fetch;
pub mod filter;
pub mod history;
pub mod news;
pub mod plugin;
pub mod preview;
pub mod routes;
pub mod sessions;
pub mod settings;
pub mod tools;

pub use plugin::BrowserPlugin;
