//! Image plugin — a layered raster editor built on `photon-rs`. Tone and colour
//! adjustments, convolution and distortion filters, painting tools and
//! transforms are applied server-side; the editor window and the agent tools
//! share the same operations engine.

pub mod adjust;
pub mod blend;
pub mod composite;
pub mod document;
pub mod filter;
pub mod layers;
pub mod ops;
pub mod paint;
pub mod pixels;
pub mod plugin;
pub mod routes;
pub mod tools;

pub use plugin::ImagePlugin;
