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
    /// Meshes in the world, meshes drawn, triangles drawn.
    pub drawn: (u32, u32, u64),
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
            .add_systems(Last, log_laps)
            .add_systems(Last, count_drawn);
    }
}

/// Count what is drawn, twice a second: meshes in the world, how many the
/// camera actually draws (after culling), and the triangles of the voxel
/// models among them (mesh data lives on the GPU, so they carry their own
/// count). The F3 panel
/// and `hw_render` show it.
fn count_drawn(
    time: Res<Time<Real>>,
    drawn: Query<(&ViewVisibility, Option<&crate::vox::Triangles>), With<Mesh3d>>,
    mut latency: ResMut<Latency>,
    mut since: Local<f32>,
) {
    *since += time.delta_secs();
    if *since < 0.5 {
        return;
    }
    *since = 0.0;
    let (mut all, mut seen, mut triangles) = (0u32, 0u32, 0u64);
    for (visibility, count) in &drawn {
        all += 1;
        if visibility.get() {
            seen += 1;
            triangles += count.map(|c| c.0 as u64).unwrap_or(0);
        }
    }
    latency.drawn = (all, seen, triangles);
    crate::bridge::publish_render(format!(
        r#"{{"meshes":{all},"visible":{seen},"triangles":{triangles}}}"#
    ));
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
