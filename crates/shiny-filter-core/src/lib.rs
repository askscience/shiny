//! `shiny-filter-core` — the light ad-blocking engine.
//!
//! Everything here is synchronous and depends only on `adblock-rust` (plus
//! serde/parking_lot/tracing): parsing filter lists, the compiled-engine cache
//! format, per-request verdicts and cosmetic selectors. It deliberately carries
//! **no HTTP client and no async runtime**, so it can be linked into both the
//! server (`shiny`) and the kiosk shell (`peakd`) without dragging in
//! `wreq`/BoringSSL or tokio.
//!
//! The server (`crates/shiny-filter`) owns the network half: fetching the lists
//! and compiling/serializing the engine into the shared cache that the shell
//! restores from.
//!
//! # Layout
//!
//! * [`engine`] — `AdFilter`: filter lists → `Engine`, on-disk cache format,
//!   and the block/allow verdict for one request.
//! * [`classify`] — HTTP request → adblock request type.

pub mod classify;
pub mod engine;

pub use classify::{classify_request, build_adblock_request, RequestKind};
pub use engine::{
    default_cache_dir, default_lists, AdFilter, CacheMeta, FilterConfig, Verdict,
};
