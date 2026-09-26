//! Iroh remote access.
//!
//! Exposes the loopback HTTP server over a peer-to-peer QUIC endpoint. The real
//! implementation is behind the `iroh` cargo feature; without it a stub keeps
//! `AppState` and the `/api/remote/*` routes compiling and reporting disabled.

#[cfg(feature = "iroh")]
mod real;
#[cfg(feature = "iroh")]
pub use real::{IrohRemote, IrohStatus};

#[cfg(not(feature = "iroh"))]
mod stub;
#[cfg(not(feature = "iroh"))]
pub use stub::{IrohRemote, IrohStatus};
