//! SearchService and SearchResult live in the SDK now. Re-exported for
//! existing call sites.

pub use shiny_plugin_sdk::services::{SearchService, SearchResult, is_junk_search_row, is_aggregator_row, text_chunks, clean_text};