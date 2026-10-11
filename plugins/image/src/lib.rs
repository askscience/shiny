//! Image plugin — a minimal photo editor built on `photon-rs`: tone and colour
//! adjustments, blur/sharpen/vignette, one-tap looks and 90° geometry, applied
//! server-side by the engine the editor window and the agent tools share.

pub mod adjust;
pub mod doc;
pub mod filter;
pub mod looks;
pub mod ops;
pub mod pixels;
pub mod plugin;
pub mod routes;
pub mod tools;

pub use plugin::ImagePlugin;
