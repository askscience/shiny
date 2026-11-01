//! Word plugin — a word processor for open .odt documents, with .docx/PDF
//! export and composed (cover, contents, header, page numbers) documents.

pub mod engine;

pub mod plugin;
pub mod routes;
pub mod tools;

pub use plugin::WordPlugin;