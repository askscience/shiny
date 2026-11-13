//! Oorbit's simulation: a tiny planet whose society grows in real time.
//!
//! The crate is pure (no I/O, no clock): every function takes the current
//! time as an argument, and the outcome depends only on the planet, the
//! catalog, the time and the player's actions.
//!
//! # Time
//!
//! Time is cut into [`STEP_MS`] steps aligned to the Unix epoch, and the
//! world changes only at step boundaries. Each step draws its randomness from
//! a generator seeded with the planet seed and the step index. Advancing from
//! A to C therefore processes exactly the same steps, with the same random
//! draws, whether it happens in one call or stops at B on the way: a computer
//! switched off overnight catches up on opening with the same result as one
//! that stayed on.
//!
//! An action happens at a time `t`: the planet is first advanced to `t`
//! (every boundary up to and including `t`), then the action applies.

pub mod actions;
pub mod autoplay;
pub mod catalog;
pub mod clock;
pub mod cells;
pub mod neighbors;
pub mod orders;
pub mod planet;
pub mod rng;
pub mod step;
pub mod terrain;
pub mod view;

pub use actions::{apply, Action, ActionError};
pub use catalog::Catalog;
pub use planet::{Order, Planet};
pub use step::{advance, rates, Rates};

/// Length of one simulation step: ten minutes of wall-clock time.
pub const STEP_MS: i64 = 10 * 60 * 1000;

/// Steps in one real hour.
pub const STEPS_PER_HOUR: i64 = 6;

/// Fraction of an hour a step covers (rates are expressed per hour).
pub const STEP_HOURS: f64 = 1.0 / STEPS_PER_HOUR as f64;

/// The step index a wall-clock time falls in.
pub fn step_of(time_ms: i64) -> i64 {
    time_ms.div_euclid(STEP_MS)
}

/// Steps in a number of real hours (rounded up).
pub fn hours_to_steps(hours: f64) -> i64 {
    (hours * STEPS_PER_HOUR as f64).ceil().max(1.0) as i64
}
