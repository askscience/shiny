//! Audio analysis shared by LiteAura plugins (Studio, Capture).
//!
//! Plain functions over `f32` sample slices, no I/O. Linked as an rlib into
//! each plugin, so nothing here crosses the plugin ABI.

pub mod loudness;
pub mod peaks;

/// Below this a recursive filter state is flushed to zero (denormals are slow).
const DENORMAL: f64 = 1e-20;

pub(crate) fn flush(x: f64) -> f64 {
    if x.abs() < DENORMAL {
        0.0
    } else {
        x
    }
}

/// Linear gain to decibels (floored at -180 dB).
pub fn gain_to_db(g: f32) -> f32 {
    20.0 * g.max(1e-9).log10()
}
