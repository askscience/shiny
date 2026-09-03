//! Image plugin — an image editor built on `photon-rs`. Effects, filters and
//! transforms are applied server-side; the Image window and the agent tools
//! share the same operations engine.

pub mod ops;
pub mod plugin;
pub mod routes;
pub mod session;
pub mod tools;

pub use plugin::ImagePlugin;
