//! Request classification now lives in `shiny-filter-core` (it needs no HTTP
//! client). This module re-exports it so the proxy's `crate::classify::…`
//! paths keep working.

pub use shiny_filter_core::classify::*;
