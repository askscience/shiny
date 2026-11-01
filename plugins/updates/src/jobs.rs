//! Background jobs for long-running operations (refresh / apply / Ollama).
//!
//! A job runs on its own OS thread and appends log lines to shared state. HTTP
//! clients and agent tools poll [`Job::snapshot`] with a `from` cursor. The
//! job thread never touches SQLite — see `history.rs`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use serde::Serialize;

use crate::progress::{Event, Progress};
use crate::status::now_secs;

/// Maximum log lines retained per job.
const MAX_LINES: usize = 5000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Running,
    Done,
    Failed,
}

struct Inner {
    state: State,
    success: Option<bool>,
    error: Option<String>,
    lines: Vec<String>,
    progress: Progress,
}

pub struct Job {
    pub id: String,
    pub kind: String,
    pub started: i64,
    inner: Mutex<Inner>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Snapshot {
    pub lines: Vec<String>,
    pub done: bool,
    pub success: Option<bool>,
    pub error: Option<String>,
    pub progress: Progress,
}

impl Job {
    /// Fold a structured progress event into the job state.
    pub fn progress(&self, event: Event) {
        if let Ok(mut g) = self.inner.lock() {
            g.progress.apply(event);
        }
    }

    pub fn log(&self, line: impl AsRef<str>) {
        if let Ok(mut g) = self.inner.lock() {
            if g.lines.len() < MAX_LINES {
                g.lines.push(line.as_ref().to_string());
            }
        }
    }

    pub fn finish(&self, success: bool, error: Option<String>) {
        if let Ok(mut g) = self.inner.lock() {
            g.state = if success { State::Done } else { State::Failed };
            g.success = Some(success);
            g.error = error;
        }
    }

    /// Lines from `from` onward plus completion state.
    pub fn snapshot(&self, from: usize) -> Snapshot {
        match self.inner.lock() {
            Ok(g) => Snapshot {
                lines: if from < g.lines.len() {
                    g.lines[from..].to_vec()
                } else {
                    Vec::new()
                },
                done: g.state != State::Running,
                success: g.success,
                error: g.error.clone(),
                progress: g.progress.clone(),
            },
            Err(_) => Snapshot {
                lines: Vec::new(),
                done: true,
                success: Some(false),
                error: Some("job state lock poisoned".into()),
                progress: Progress::default(),
            },
        }
    }

    pub fn total(&self) -> usize {
        self.inner.lock().map(|g| g.lines.len()).unwrap_or(0)
    }

    pub fn state(&self) -> (bool, Option<bool>, Option<String>) {
        match self.inner.lock() {
            Ok(g) => (g.state != State::Running, g.success, g.error.clone()),
            Err(_) => (true, Some(false), Some("job state lock poisoned".into())),
        }
    }

    pub fn full_log(&self) -> String {
        self.inner
            .lock()
            .map(|g| g.lines.join("\n"))
            .unwrap_or_default()
    }
}

fn registry() -> &'static Mutex<HashMap<String, Arc<Job>>> {
    static JOBS: OnceLock<Mutex<HashMap<String, Arc<Job>>>> = OnceLock::new();
    JOBS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn get(id: &str) -> Option<Arc<Job>> {
    registry().lock().ok()?.get(id).cloned()
}

/// The job that is still running, if any. Package managers hold an exclusive
/// lock, so the plugin runs one job at a time; a reopened window uses this to
/// re-attach to an install already in progress.
pub fn running() -> Option<Arc<Job>> {
    registry()
        .lock()
        .ok()?
        .values()
        .filter(|j| !j.state().0)
        .max_by_key(|j| j.started)
        .cloned()
}

/// Create a job and run `work` on a dedicated thread. Returns immediately.
///
/// Refused while another job is still running: two package-manager runs at
/// once only fight over the dpkg/rpm lock, and an Ollama install on top of a
/// running upgrade is exactly the unsafe overlap `ops` tries to avoid.
pub fn start(
    kind: &str,
    work: impl FnOnce(Arc<Job>) + Send + 'static,
) -> Result<Arc<Job>, shiny_plugin_sdk::errors::AppError> {
    let job = Arc::new(Job {
        id: uuid::Uuid::new_v4().to_string(),
        kind: kind.to_string(),
        started: now_secs(),
        inner: Mutex::new(Inner {
            state: State::Running,
            success: None,
            error: None,
            lines: Vec::new(),
            progress: Progress::default(),
        }),
    });

    {
        let mut g = registry()
            .lock()
            .map_err(|_| shiny_plugin_sdk::errors::AppError::Internal("job registry lock poisoned".into()))?;
        if g.values().any(|j| !j.state().0) {
            return Err(shiny_plugin_sdk::errors::AppError::BadRequest(
                "An update is already running; wait for it to finish".into(),
            ));
        }
        // Opportunistic GC: never let finished jobs accumulate without bound.
        if g.len() > 64 {
            g.retain(|_, j| !j.state().0);
        }
        g.insert(job.id.clone(), job.clone());
    }

    let thread_job = job.clone();
    std::thread::spawn(move || work(thread_job));
    Ok(job)
}
