//! Files plugin cdylib entry — see `plugin.rs` for the trait implementation.

pub mod fs_util;
pub mod plugin;
pub mod preview;
pub mod routes;
pub mod tools;

pub use plugin::FilesPlugin;
