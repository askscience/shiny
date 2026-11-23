//! Structured progress of a running upgrade, for the window's progress bar and
//! per-package rows.
//!
//! Package managers that can report machine-readable progress (apt, through
//! `APT::Status-Fd`) turn their status lines into [`Event`]s via
//! [`crate::distro::PackageManager::progress_event`]; the job folds them into a
//! [`Progress`] that clients poll alongside the log. Managers without such a
//! channel simply never produce events and the window falls back to the
//! spinner + last log line.

use std::time::Instant;

use serde::Serialize;

use crate::model::DownloadItem;

/// The step a package is at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PkgState {
    /// Being unpacked (the new files are going in).
    Unpacking,
    /// Running its configuration scripts.
    Configuring,
    /// Installed and configured.
    Done,
    /// Being removed (apt full-upgrade may remove obsolete packages).
    Removing,
    /// Removed.
    Removed,
}

/// One parsed status line.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Downloading: overall percent and a human message ("Retrieving file 3 of 12").
    /// An empty message keeps the current one (a bare progress-bar tick).
    Download { percent: f32, message: String },
    /// A new step without a percent of its own ("Stopping Ollama…",
    /// "Installing ollama to /usr/local"). Also written to the job log.
    Step { message: String },
    /// Installing: overall percent, the package (if any) and its new state.
    Install {
        percent: f32,
        package: Option<String>,
        state: Option<PkgState>,
        message: String,
    },
    /// The package manager reported an error for a package.
    Error { package: Option<String>, message: String },
}

#[derive(Debug, Clone, Serialize)]
pub struct PkgProgress {
    pub name: String,
    pub state: PkgState,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Progress {
    /// `download` or `install`; `None` until the first event.
    pub phase: Option<&'static str>,
    /// Overall percent of the current phase, 0–100.
    pub percent: Option<f32>,
    /// The current status message.
    pub message: Option<String>,
    /// The package being worked on right now.
    pub current: Option<String>,
    /// Every package seen so far, in first-seen order, with its latest state.
    pub packages: Vec<PkgProgress>,
    /// Packages the manager reported an error for.
    pub failed: Vec<String>,

    /// What the job downloads, when known beforehand (`set_downloads`).
    pub downloads: Vec<DownloadItem>,
    /// The files that have arrived (by name).
    pub downloaded: Vec<String>,
    /// Bytes to download in all, and bytes downloaded so far.
    pub download_total: u64,
    pub download_done: u64,
    /// Download speed (bytes per second) and the time left (seconds), from
    /// the recent samples.
    pub speed: Option<f64>,
    pub eta: Option<u64>,
    /// The whole job, 0–100: downloading counts for 40 % when there is
    /// something to download, installing for the rest.
    pub overall: Option<f32>,
    /// Every package of the job with where it is and how far (`view`).
    pub items: Vec<ItemProgress>,

    #[serde(skip)]
    download_dir: Option<std::path::PathBuf>,
    #[serde(skip)]
    last_sample: Option<(Instant, u64)>,
    #[serde(skip)]
    last_scan: Option<Instant>,
    #[serde(skip)]
    install_percent: f32,
}

/// One package's way through the job, for the window and the AI tools.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ItemProgress {
    pub name: String,
    /// `queued`, `downloading`, `downloaded`, `unpacking`, `configuring`,
    /// `done`, `removing`, `removed` or `failed`.
    pub state: &'static str,
    /// How far this package is, 0–100: downloading takes it to 50, unpacking
    /// to 65, configuring to 85, done 100.
    pub percent: f32,
    /// Bytes it downloads, when known.
    pub size: Option<u64>,
}

/// The share of the whole job taken by downloading, when there is any.
const DOWNLOAD_WEIGHT: f32 = 0.4;

impl Progress {
    /// What this job will download (from the package manager's plan) and
    /// where finished files land.
    pub fn set_downloads(&mut self, items: Vec<DownloadItem>, dir: Option<&str>) {
        self.download_total = items.iter().map(|i| i.size).sum();
        self.downloads = items;
        self.download_dir = dir.map(std::path::PathBuf::from);
        self.scan_downloads(true);
    }

    /// Note the files that have arrived (at most every 300 ms unless forced).
    fn scan_downloads(&mut self, force: bool) {
        let Some(dir) = self.download_dir.clone() else { return };
        if !force && self.last_scan.is_some_and(|t| t.elapsed().as_millis() < 300) {
            return;
        }
        self.last_scan = Some(Instant::now());
        for item in &self.downloads {
            if self.downloaded.contains(&item.name) {
                continue;
            }
            let arrived = std::fs::metadata(dir.join(&item.file)).is_ok_and(|m| m.len() >= item.size);
            if arrived {
                self.downloaded.push(item.name.clone());
            }
        }
    }

    /// Fold a new byte count into the speed and the time left.
    fn sample(&mut self, done: u64) {
        let now = Instant::now();
        match self.last_sample {
            Some((at, before)) if done > before => {
                let secs = now.duration_since(at).as_secs_f64();
                if secs >= 0.25 {
                    let rate = (done - before) as f64 / secs;
                    // A smoothed rate: one slow packet does not swing the ETA.
                    self.speed = Some(match self.speed {
                        Some(old) => old * 0.7 + rate * 0.3,
                        None => rate,
                    });
                    self.last_sample = Some((now, done));
                }
            }
            None => self.last_sample = Some((now, done)),
            _ => {}
        }
        self.eta = match self.speed {
            Some(v) if v > 1.0 && self.download_total > done => Some(((self.download_total - done) as f64 / v).ceil() as u64),
            _ => None,
        };
    }

    /// Fill `items` and `overall` from what is known now.
    pub fn view(&self) -> Progress {
        let mut p = self.clone();
        let in_install = p.phase == Some("install") || !p.packages.is_empty();
        // Bytes of the files not yet arrived, attributed in order to the
        // first one still on its way (apt fetches in plan order).
        let mut in_flight = p.download_done.saturating_sub(
            p.downloads.iter().filter(|d| p.downloaded.contains(&d.name)).map(|d| d.size).sum(),
        );
        let mut items: Vec<ItemProgress> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for d in &p.downloads {
            if !seen.insert(d.name.clone()) {
                continue;
            }
            let arrived = in_install || p.downloaded.contains(&d.name);
            let (state, percent) = if arrived {
                ("downloaded", 50.0)
            } else if in_flight > 0 {
                let got = in_flight.min(d.size);
                in_flight -= got;
                ("downloading", 50.0 * got as f32 / d.size.max(1) as f32)
            } else {
                ("queued", 0.0)
            };
            items.push(ItemProgress { name: d.name.clone(), state, percent, size: Some(d.size) });
        }
        for pkg in &p.packages {
            let (state, percent) = match pkg.state {
                PkgState::Unpacking => ("unpacking", 65.0),
                PkgState::Configuring => ("configuring", 85.0),
                PkgState::Done => ("done", 100.0),
                PkgState::Removing => ("removing", 70.0),
                PkgState::Removed => ("removed", 100.0),
            };
            match items.iter_mut().find(|i| i.name == pkg.name) {
                Some(i) => {
                    i.state = state;
                    i.percent = percent;
                }
                None => items.push(ItemProgress { name: pkg.name.clone(), state, percent, size: None }),
            }
        }
        for name in &p.failed {
            match items.iter_mut().find(|i| &i.name == name) {
                Some(i) => i.state = "failed",
                None => items.push(ItemProgress { name: name.clone(), state: "failed", percent: 0.0, size: None }),
            }
        }
        p.items = items;
        p.overall = match (p.phase, p.download_total > 0) {
            (None, _) => None,
            (Some("install"), true) => Some(DOWNLOAD_WEIGHT * 100.0 + (1.0 - DOWNLOAD_WEIGHT) * p.install_percent),
            (Some("install"), false) => Some(p.install_percent),
            (Some("download"), true) => {
                Some(DOWNLOAD_WEIGHT * 100.0 * p.download_done as f32 / p.download_total.max(1) as f32)
            }
            _ => p.percent,
        };
        p
    }

    /// The job is over: a package still mid-step (unpacking or configuring)
    /// did not finish, so it counts as failed — for the window and for the
    /// AI tools that read the same snapshot.
    pub fn end(&mut self) {
        for p in &self.packages {
            if matches!(p.state, PkgState::Unpacking | PkgState::Configuring) && !self.failed.contains(&p.name) {
                self.failed.push(p.name.clone());
            }
        }
        self.current = None;
        self.speed = None;
        self.eta = None;
    }

    pub fn apply(&mut self, event: Event) {
        match event {
            Event::Download { percent, message } => {
                self.phase = Some("download");
                let percent = percent.clamp(0.0, 100.0);
                self.percent = Some(percent);
                if !message.is_empty() {
                    self.message = Some(message);
                }
                if self.download_total > 0 {
                    let done = (self.download_total as f64 * percent as f64 / 100.0) as u64;
                    self.download_done = done.max(self.download_done);
                    self.scan_downloads(false);
                    self.sample(self.download_done);
                }
            }
            Event::Step { message } => {
                self.phase = Some("step");
                self.percent = None;
                self.current = None;
                self.message = Some(message);
            }
            Event::Install {
                percent,
                package,
                state,
                message,
            } => {
                self.phase = Some("install");
                self.percent = Some(percent.clamp(0.0, 100.0));
                self.install_percent = percent.clamp(0.0, 100.0);
                // Installing starts once every file is in.
                self.download_done = self.download_total;
                self.speed = None;
                self.eta = None;
                self.message = Some(message);
                self.current = package.clone();
                if let (Some(name), Some(state)) = (package, state) {
                    match self.packages.iter_mut().find(|p| p.name == name) {
                        Some(p) => p.state = state,
                        None => self.packages.push(PkgProgress { name, state }),
                    }
                }
            }
            Event::Error { package, message } => {
                self.message = Some(message);
                if let Some(name) = package {
                    if !self.failed.contains(&name) {
                        self.failed.push(name);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan() -> Vec<DownloadItem> {
        vec![
            DownloadItem { name: "bash".into(), file: "bash_5.2_amd64.deb".into(), size: 600 },
            DownloadItem { name: "curl".into(), file: "curl_8_amd64.deb".into(), size: 400 },
        ]
    }

    #[test]
    fn downloads_are_followed_per_package() {
        let mut p = Progress::default();
        p.set_downloads(plan(), None);
        assert_eq!(p.download_total, 1000);
        // 30 % of 1000 bytes: half of the first file.
        p.apply(Event::Download { percent: 30.0, message: String::new() });
        let v = p.view();
        assert_eq!(v.download_done, 300);
        assert_eq!(v.items[0], ItemProgress { name: "bash".into(), state: "downloading", percent: 25.0, size: Some(600) });
        assert_eq!(v.items[1].state, "queued");
        assert!((v.overall.unwrap() - 12.0).abs() < 1e-4);
        // Installing: every file is in, the install percent drives the rest.
        p.apply(Event::Install { percent: 50.0, package: Some("bash".into()), state: Some(PkgState::Configuring), message: String::new() });
        let v = p.view();
        assert_eq!((v.items[0].state, v.items[0].percent), ("configuring", 85.0));
        assert_eq!((v.items[1].state, v.items[1].percent), ("downloaded", 50.0));
        assert!((v.overall.unwrap() - 70.0).abs() < 1e-4);
        assert_eq!(v.download_done, 1000);
    }

    #[test]
    fn arrived_files_count_as_downloaded() {
        let dir = std::env::temp_dir().join(format!("updates-dl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("bash_5.2_amd64.deb"), vec![0u8; 600]).unwrap();
        let mut p = Progress::default();
        p.set_downloads(plan(), dir.to_str());
        assert_eq!(p.downloaded, vec!["bash".to_string()]);
        p.apply(Event::Download { percent: 80.0, message: String::new() });
        let v = p.view();
        assert_eq!(v.items[0].state, "downloaded");
        // 800 done − 600 arrived = 200 of curl's 400.
        assert_eq!((v.items[1].state, v.items[1].percent), ("downloading", 25.0));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_job_without_a_plan_reports_the_phase_percent() {
        let mut p = Progress::default();
        assert_eq!(p.view().overall, None);
        p.apply(Event::Install { percent: 40.0, package: None, state: None, message: String::new() });
        assert_eq!(p.view().overall, Some(40.0));
    }

    #[test]
    fn a_package_still_mid_step_at_the_end_failed() {
        let mut p = Progress::default();
        for (name, state) in [("a", PkgState::Done), ("b", PkgState::Unpacking), ("c", PkgState::Configuring), ("d", PkgState::Removed)] {
            p.apply(Event::Install { percent: 50.0, package: Some(name.into()), state: Some(state), message: String::new() });
        }
        p.apply(Event::Error { package: Some("c".into()), message: "boom".into() });
        p.end();
        assert_eq!(p.failed, vec!["c".to_string(), "b".to_string()]);
        assert!(p.current.is_none());
    }
}
