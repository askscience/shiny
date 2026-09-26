//! The Browser's download manager (shell side).
//!
//! Every download from every site and every profile is captured by the Qt shim
//! (`QWebEngineProfile::downloadRequested`), which assigns an id, saves into the
//! directory the window reported, and forwards lifecycle events here over
//! [`crate::shim::DownloadCallback`]. This module keeps the live list (the
//! window's source of truth), relays each event to the window's downloads panel
//! through `window.__peakdViewEvent`, and issues pause/resume/cancel commands
//! back to Qt.
//!
//! Persistence to the user's history is the *window's* job: it receives each
//! event and POSTs it to `/api/browser/downloads/event`. Keeping the file I/O
//! and the DB in separate processes means a broken DB can never stall a
//! download.

use std::collections::BTreeMap;
use std::path::PathBuf;

use parking_lot::Mutex;
use serde_json::{json, Value};

use crate::shim;

/// Live downloads, keyed by the shell id (`d1`, `d2`, …). A `BTreeMap` keeps
/// insertion order by id, which for `d<n>` is chronological.
static DOWNLOADS: Mutex<BTreeMap<String, Value>> = Mutex::new(BTreeMap::new());

/// Where the window asked downloads to land (the user's `Downloads` folder in
/// whichever home mode the server resolved). `None` until the page reports it.
static DOWNLOAD_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Set the destination directory (from `peakd:settings`).
pub fn set_dir(dir: &str) {
    if dir.trim().is_empty() {
        return;
    }
    let path = PathBuf::from(dir);
    let _ = std::fs::create_dir_all(&path);
    shim::set_download_dir(&path.to_string_lossy());
    *DOWNLOAD_DIR.lock() = Some(path);
}

/// Current destination, if the window has told us.
#[allow(dead_code)]
pub fn dir() -> Option<PathBuf> {
    DOWNLOAD_DIR.lock().clone()
}

/// One download lifecycle event from the shim.
///
/// `payload` is a JSON object from C++ (`{id, url, host, file, path, mime,
/// total, received, state, error}`); `kind` is a coarse tag for logging.
pub fn on_event(id: &str, kind: &str, payload: &str) {
    let value: Value = match serde_json::from_str(payload) {
        Ok(v) => v,
        Err(_) => json!({ "id": id, "state": kind }),
    };
    // `removed` deletes; everything else upserts.
    if kind == "removed" {
        DOWNLOADS.lock().remove(id);
    } else {
        DOWNLOADS.lock().insert(id.to_string(), value.clone());
    }
    let event = json!({ "type": "download", "kind": kind, "download": value });
    let script = format!(
        "window.__peakdViewEvent && window.__peakdViewEvent({event})"
    );
    shim::main_run_js(&script, 0);
}

/// Send the full current list to the window (answer to `peakd:downloads:list`).
pub fn push_list() {
    let list: Vec<Value> = DOWNLOADS.lock().values().cloned().collect();
    let payload = serde_json::to_string(&list).unwrap_or_else(|_| "[]".into());
    let script = format!("window.__peakdDownloads && window.__peakdDownloads({payload})");
    shim::main_run_js(&script, 0);
}

/// Pause / resume / cancel / retry / remove one download.
pub fn action(id: &str, action: &str) {
    match action {
        // `retry` is a fresh navigation handled by the window (it re-opens the
        // URL); the shell only cancels the failed entry here.
        "pause" | "resume" | "cancel" => shim::download_action(id, action),
        "remove" => {
            DOWNLOADS.lock().remove(id);
            shim::download_action(id, "forget");
        }
        "clear" => {
            DOWNLOADS
                .lock()
                .retain(|_, v| !is_finished(v.get("state").and_then(Value::as_str)));
        }
        _ => {}
    }
}

fn is_finished(state: Option<&str>) -> bool {
    matches!(state, Some("completed") | Some("cancelled") | Some("interrupted"))
}
