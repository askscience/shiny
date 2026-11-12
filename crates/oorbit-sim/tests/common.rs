//! Shared by the tests: times, and the steady player (`oorbit_sim::autoplay`).
#![allow(dead_code)]

pub use oorbit_sim::autoplay::play_session;

pub const HOUR: i64 = 3_600_000;
pub const DAY: i64 = 24 * HOUR;
/// 2026-11-12T08:00:00Z
pub const T0: i64 = 1_794_470_400_000;
