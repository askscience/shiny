//! Structured progress of a running upgrade, for the window's progress bar and
//! per-package rows.
//!
//! Package managers that can report machine-readable progress (apt, through
//! `APT::Status-Fd`) turn their status lines into [`Event`]s via
//! [`crate::distro::PackageManager::progress_event`]; the job folds them into a
//! [`Progress`] that clients poll alongside the log. Managers without such a
//! channel simply never produce events and the window falls back to the
//! spinner + last log line.

use serde::Serialize;

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
}

impl Progress {
    pub fn apply(&mut self, event: Event) {
        match event {
            Event::Download { percent, message } => {
                self.phase = Some("download");
                self.percent = Some(percent.clamp(0.0, 100.0));
                if !message.is_empty() {
                    self.message = Some(message);
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
