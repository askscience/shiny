//! Interface scale for the kiosk shell.
//!
//! The choice file, `auto` DPI snapping and the runtime report live in the
//! shared `peakd-core` crate (the macOS shell uses the same code). This shell
//! applies the resolved factor as Qt page zoom on its 100 ms pump.

pub use peakd_core::display::{auto_scale, modified, read_choice, write_runtime, Choice};
