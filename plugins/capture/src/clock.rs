//! A frame clock on absolute deadlines: tick `n` is due at `start + n·period`,
//! so sleep jitter never accumulates into drift.

use std::time::{Duration, Instant};

pub struct Ticker {
    start: Instant,
    period: Duration,
    next: u64,
}

impl Ticker {
    pub fn new(fps: u32) -> Self {
        Self {
            start: Instant::now(),
            period: Duration::from_secs(1) / fps.max(1),
            next: 0,
        }
    }

    pub fn period(&self) -> Duration {
        self.period
    }

    fn due(&self, n: u64) -> Instant {
        self.start + self.period * n as u32
    }

    /// Wait for the next tick. Returns how many ticks were missed (the caller
    /// repeats the last frame that many times to keep constant frame rate).
    pub fn wait(&mut self) -> u64 {
        let due = self.due(self.next);
        let now = Instant::now();
        if due > now {
            std::thread::sleep(due - now);
            self.next += 1;
            return 0;
        }
        // Late: catch up to the tick that is current now.
        let behind = ((now - self.start).as_nanos() / self.period.as_nanos()) as u64;
        let missed = behind.saturating_sub(self.next);
        self.next = behind + 1;
        missed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_missed_ticks() {
        let mut t = Ticker::new(1000);
        assert_eq!(t.wait(), 0);
        std::thread::sleep(Duration::from_millis(10));
        let missed = t.wait();
        assert!((7..=12).contains(&missed), "{missed}");
    }
}
