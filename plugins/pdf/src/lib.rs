//! PDF plugin — view and edit PDF documents.
//!
//! Two engines, on purpose: **pdf_oxide** renders pages and backs the structural
//! operations (create, merge, rotate, annotate, watermark), while **lopdf**
//! handles in-place text editing, which needs raw content-stream access. See
//! `stream_edit.rs` for why the split exists.

pub mod ops;
pub mod plugin;
pub mod routes;
pub mod stream_edit;
pub mod tools;

pub use plugin::PdfPlugin;
