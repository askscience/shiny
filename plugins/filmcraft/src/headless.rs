//! The headless FilmCraft engine.
//!
//! The window in the browser cannot open server-side files, and the browser
//! tab is not a place to run a two-hour render. So this module links
//! `filmcraft-engine` — the real engine, with FilmCraft's own H.264/HEVC/AV1/
//! ProRes/DNx decoders, renderer and exporters — and drives it on
//! server-side files.
//!
//! # Why a dedicated thread, not the plugin runtime
//!
//! `rt::bridge` runs plugin jobs on one **process-global, serial** worker
//! (`crates/shiny-plugin-sdk/src/rt.rs`). A tool that waited for a long export
//! there would stall every plugin in the process. So: nothing in this module
//! blocks the plugin runtime. Each call gets its own short-lived OS thread
//! (`run`), the session itself is behind a mutex (one session, serial use), and
//! exports are started non-blocking (`wait: false`) and polled.
//!
//! # Two engines, two worlds
//!
//! This session and the one inside the browser window are separate. They share
//! `.fcproj` files on disk and nothing else: the window keeps its project in
//! memory (OPFS), so an export here never disturbs an edit in progress there,
//! and an edit saved there is not seen by a session already open here. Use
//! `file.open` to (re)load a project before a batch run.

use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde_json::{json, Value};

use filmcraft_engine::Session;

/// Default ceiling for a single command on the headless session.
pub const CMD_TIMEOUT: Duration = Duration::from_secs(60);
/// Opening a project parses every container index, so it gets longer.
pub const OPEN_TIMEOUT: Duration = Duration::from_secs(120);

struct Engine {
    session: Mutex<Session>,
}

static ENGINE: OnceLock<Arc<Engine>> = OnceLock::new();

fn engine() -> Arc<Engine> {
    ENGINE
        .get_or_init(|| {
            let mut session = Session::default();
            // User export presets (and the other per-user libraries) from the
            // data directory, exactly like `filmcraft-cli` does.
            if let Some(dir) = filmcraft_engine::autosave::default_data_dir() {
                session.export_presets.set_dir(&dir);
            }
            Arc::new(Engine {
                session: Mutex::new(session),
            })
        })
        .clone()
}

/// Run `f` against the session on its own OS thread and wait up to `timeout`.
///
/// The thread exists so a command that takes real time (indexing a large MP4,
/// for instance) never sits on the plugin runtime's serial worker. A timeout
/// detaches the thread: it keeps the session lock until it finishes, so the
/// next caller simply waits its own turn rather than seeing a half-applied
/// project.
fn run<T>(label: &str, timeout: Duration, f: impl FnOnce(&mut Session) -> Result<T, String> + Send + 'static) -> Result<T, String>
where
    T: Send + 'static,
{
    let engine = engine();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name(format!("filmcraft-{label}"))
        .spawn(move || {
            let mut session = engine.session.lock().unwrap_or_else(|e| e.into_inner());
            let _ = tx.send(f(&mut session));
        })
        .map_err(|e| format!("cannot start the {label} worker: {e}"))?;
    rx.recv_timeout(timeout)
        .map_err(|_| format!("{label} timed out after {}s", timeout.as_secs()))?
}

/// Run any engine command on the headless session.
pub fn execute(command: &str, params: Value, timeout: Duration) -> Result<Value, String> {
    let command = command.to_string();
    run("command", timeout, move |session| {
        session
            .execute(&command, params)
            .map_err(|e| e.to_string())
    })
}

/// Open (or reopen) a project on the headless session.
pub fn open_project(path: &str) -> Result<Value, String> {
    let path = path.to_string();
    run("open", OPEN_TIMEOUT, move |session| {
        session
            .execute("file.open", json!({ "path": path }))
            .map_err(|e| e.to_string())
    })
}

/// Start a background export and return the engine's job description.
///
/// The project is opened fresh, so a batch export always works from the file
/// on disk. `params` is merged into the `file.exportMedia` parameters; `wait`
/// is forced to `false` because the engine runs the render on its own thread.
pub fn start_export(project: &str, params: Value, timeout: Duration) -> Result<Value, String> {
    // Validate before opening: a bad settings object should not cost the caller
    // a project load (and an engine lock) just to be rejected.
    let Value::Object(mut map) = params else {
        return Err("export settings must be an object".into());
    };
    // The render runs on the engine's own thread and is polled; never block the
    // caller for the whole export.
    map.insert("wait".to_string(), json!(false));
    let project = project.to_string();
    run("export", timeout, move |session| {
        session
            .execute("file.open", json!({ "path": project }))
            .map_err(|e| e.to_string())?;
        session
            .execute("file.exportMedia", Value::Object(map))
            .map_err(|e| e.to_string())
    })
}

/// Every job the headless session knows about (progress and results included).
pub fn jobs() -> Result<Value, String> {
    execute("jobs.list", json!({}), CMD_TIMEOUT)
}

/// Ask a background job to stop.
pub fn cancel(job: u64) -> Result<Value, String> {
    execute("jobs.cancel", json!({ "job": job }), CMD_TIMEOUT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_command_is_an_error_not_a_panic() {
        let err = execute("no.such.command", json!({}), Duration::from_secs(10)).unwrap_err();
        assert!(err.contains("no.such.command"), "{err}");
    }

    #[test]
    fn export_rejects_non_object_settings() {
        let err = start_export("/nonexistent/project.fcproj", json!("nope"), CMD_TIMEOUT).unwrap_err();
        assert!(err.contains("must be an object"), "{err}");
    }
}