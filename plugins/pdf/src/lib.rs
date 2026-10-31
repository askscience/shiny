//! PDF plugin — view and edit PDF documents.
//!
//! One engine on top of lopdf's object model. The browser renders pages with
//! pdf.js; the server owns everything that changes bytes:
//!
//! - `compose` — Markdown / HTML / plain text → a laid-out A4 PDF
//! - `stream_edit` — reading text runs and editing text in place
//! - `ops` — page operations and visible marks (highlight, notes, text, links,
//!   watermark), the API the routes and tools call
//! - `doc` — page tree, inherited attributes, resources, appended content
//! - `fonts` / `afm` — Base-14 metrics and WinAnsi encoding
//! - `store` — persistence with undo / redo

pub mod afm;
pub mod compose;
pub mod craft;
pub mod doc;
pub mod fonts;
pub mod ops;
pub mod plugin;
pub mod routes;
pub mod store;
pub mod stream_edit;
pub mod tools;

pub use plugin::PdfPlugin;
