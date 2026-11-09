//! Timing samples and CPU accounting for the engine's own report.

use std::time::Duration;

#[derive(Default, Clone)]
pub struct Samples(Vec<u32>);

impl Samples {
    pub fn push(&mut self, d: Duration) {
        self.0.push(d.as_micros().min(u32::MAX as u128) as u32);
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// `(p50, p95, max, mean)` in microseconds.
    pub fn summary(&self) -> (u32, u32, u32, u32) {
        if self.0.is_empty() {
            return (0, 0, 0, 0);
        }
        let mut v = self.0.clone();
        v.sort_unstable();
        let at = |q: f64| v[((v.len() - 1) as f64 * q).round() as usize];
        let mean = (v.iter().map(|&x| x as u64).sum::<u64>() / v.len() as u64) as u32;
        (at(0.5), at(0.95), *v.last().unwrap(), mean)
    }

    pub fn line(&self, label: &str) -> String {
        let (p50, p95, max, mean) = self.summary();
        format!(
            "{label:<10} n={:<6} p50={p50:>6}µs p95={p95:>6}µs max={max:>6}µs mean={mean:>6}µs",
            self.0.len()
        )
    }
}

/// User + system CPU time used by this process (`children = false`) or by
/// its waited-for children.
pub fn cpu_time(children: bool) -> Duration {
    // SAFETY: getrusage fills the struct we pass.
    let ru = unsafe {
        let mut ru: libc::rusage = std::mem::zeroed();
        libc::getrusage(
            if children {
                libc::RUSAGE_CHILDREN
            } else {
                libc::RUSAGE_SELF
            },
            &mut ru,
        );
        ru
    };
    let tv = |t: libc::timeval| Duration::new(t.tv_sec as u64, t.tv_usec as u32 * 1000);
    tv(ru.ru_utime) + tv(ru.ru_stime)
}

/// Peak resident memory of this process, in KiB.
pub fn peak_rss_kib() -> u64 {
    // SAFETY: as above.
    unsafe {
        let mut ru: libc::rusage = std::mem::zeroed();
        libc::getrusage(libc::RUSAGE_SELF, &mut ru);
        ru.ru_maxrss as u64
    }
}
