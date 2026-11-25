//! How long every link of the voice chain takes.
//!
//! The chain is: the player stops speaking → the recogniser has the text →
//! the game has it in a frame → (the model answers → the voice starts). Each
//! link is measured where it happens and reported here as a [`Lap`]; this
//! plugin logs it and keeps the latest of each for the HUD.
//!
//! All times are milliseconds on the page's clock (`performance.now()`), so
//! the page and the game measure on the same axis.

use std::collections::BTreeMap;

use bevy::prelude::*;

/// One measured link.
#[derive(Message, Clone, Debug)]
pub struct Lap {
    /// Which link, e.g. `"speech→text"`.
    pub link: &'static str,
    pub ms: f64,
    /// What it was about (the phrase, the request), for the log line.
    pub about: String,
}

/// The latest and the average of each link.
#[derive(Resource, Default)]
pub struct Latency {
    links: BTreeMap<&'static str, Stat>,
}

#[derive(Clone, Copy, Default)]
pub struct Stat {
    pub last: f64,
    pub count: u32,
    pub total: f64,
    pub worst: f64,
}

impl Stat {
    pub fn mean(&self) -> f64 {
        if self.count == 0 {
            0.0
        } else {
            self.total / self.count as f64
        }
    }
}

impl Latency {
    pub fn record(&mut self, link: &'static str, ms: f64) {
        let stat = self.links.entry(link).or_default();
        stat.last = ms;
        stat.count += 1;
        stat.total += ms;
        stat.worst = stat.worst.max(ms);
    }

    pub fn iter(&self) -> impl Iterator<Item = (&'static str, Stat)> + '_ {
        self.links.iter().map(|(link, stat)| (*link, *stat))
    }
}

pub struct TelemetryPlugin;

impl Plugin for TelemetryPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Latency>()
            .add_message::<Lap>()
            .add_systems(Last, log_laps);
    }
}

fn log_laps(mut laps: MessageReader<Lap>, mut latency: ResMut<Latency>) {
    for lap in laps.read() {
        latency.record(lap.link, lap.ms);
        let line = format!("latency {} {:.0} ms ({})", lap.link, lap.ms, lap.about);
        info!("{line}");
        crate::bridge::host_log(&line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_last_mean_and_worst() {
        let mut latency = Latency::default();
        latency.record("speech→text", 100.0);
        latency.record("speech→text", 300.0);
        let (link, stat) = latency.iter().next().unwrap();
        assert_eq!(link, "speech→text");
        assert_eq!(stat.last, 300.0);
        assert_eq!(stat.mean(), 200.0);
        assert_eq!(stat.worst, 300.0);
    }
}
