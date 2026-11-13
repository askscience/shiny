//! The planet's own time. The simulation runs on planet time (every
//! duration in the catalog is planet time); the clock turns the wall clock
//! into it: [`ACTIVE_RATE`] times faster while someone is playing, and
//! [`AWAY_RATE_HALVES`]/2 as fast while nobody is. The plugin marks the player
//! present (`present_until`) whenever the window reports activity.
//!
//! The clock counts planet time in half milliseconds, so every rate is a
//! whole number per wall millisecond: advancing it from A to C in one call
//! or stopping at B on the way gives the same time, exactly.

use serde::{Deserialize, Serialize};

/// Planet time per wall time while someone plays: a real minute is an hour.
pub const ACTIVE_RATE: i64 = 60;
/// Planet time per wall time while nobody plays, in halves: half speed.
pub const AWAY_RATE_HALVES: i64 = 1;
/// How long one report of activity keeps the planet in play.
pub const PRESENCE_MS: i64 = 45_000;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Clock {
    /// The wall time the clock last moved to (0: never used).
    pub wall_ms: i64,
    /// Planet time at `wall_ms`, in half milliseconds.
    pub planet_halves: i64,
    /// Someone is playing until this wall time.
    pub present_until: i64,
}

impl Clock {
    /// A clock whose planet time equals the wall time now.
    pub fn starting(wall_ms: i64) -> Clock {
        Clock {
            wall_ms,
            planet_halves: wall_ms * 2,
            present_until: 0,
        }
    }

    /// Planet time (ms) at the clock's last wall time.
    pub fn planet_ms(&self) -> i64 {
        self.planet_halves.div_euclid(2)
    }

    /// Move the clock to `wall_ms` and return the planet time then. A clock
    /// never used starts at the wall time (planets from before the clock
    /// keep their timeline). Earlier wall times change nothing.
    pub fn tick(&mut self, wall_ms: i64) -> i64 {
        if self.wall_ms == 0 && self.planet_halves == 0 {
            *self = Clock::starting(wall_ms);
        }
        if wall_ms > self.wall_ms {
            let present_end = self.present_until.clamp(self.wall_ms, wall_ms);
            let present = present_end - self.wall_ms;
            let away = wall_ms - present_end;
            self.planet_halves += present * ACTIVE_RATE * 2 + away * AWAY_RATE_HALVES;
            self.wall_ms = wall_ms;
        }
        self.planet_ms()
    }

    /// Someone is playing at `wall_ms`: keep the planet in play a little.
    pub fn present(&mut self, wall_ms: i64) {
        self.tick(wall_ms);
        self.present_until = self.present_until.max(wall_ms + PRESENCE_MS);
    }

    /// Is someone playing at the clock's last wall time?
    pub fn is_present(&self) -> bool {
        self.present_until > self.wall_ms
    }

    /// Planet milliseconds per wall millisecond right now.
    pub fn rate(&self) -> f64 {
        if self.is_present() {
            ACTIVE_RATE as f64
        } else {
            AWAY_RATE_HALVES as f64 / 2.0
        }
    }

    /// Testing: jump the planet forward by `planet_ms`.
    pub fn skip(&mut self, planet_ms: i64) {
        self.planet_halves += planet_ms.max(0) * 2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn present_time_runs_fast_and_away_time_slow() {
        let mut c = Clock::starting(1_000_000);
        let start = c.planet_ms();
        c.present(1_000_000);
        // 45 s present, then 55 s away.
        let t = c.tick(1_100_000);
        assert_eq!(t - start, 45_000 * 60 + 55_000 / 2);
        assert!(!c.is_present());
    }

    #[test]
    fn ticking_in_pieces_equals_one_tick() {
        let mut whole = Clock::starting(10);
        whole.present(10);
        let mut pieces = whole.clone();
        whole.tick(100_003);
        for w in [11, 17, 30_001, 45_009, 45_011, 99_999, 100_003] {
            pieces.tick(w);
        }
        assert_eq!(whole, pieces);
    }

    #[test]
    fn a_clock_never_used_starts_at_the_wall_time() {
        let mut c = Clock::default();
        assert_eq!(c.tick(5_000), 5_000);
        assert_eq!(c.tick(4_000), 5_000, "never backwards");
    }
}
