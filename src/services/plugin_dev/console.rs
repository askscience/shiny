//! What a plugin's window reported while it ran in the page: a module that
//! failed to load, an exception, a rejected promise.
//!
//! The page posts them (`web/js/pluginConsole.js`); the server keeps the last
//! few per plugin in memory, and `plugin_dev_console` hands them to the
//! assistant, so a window that stays blank can be fixed without anyone
//! opening the browser's own console.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::{Extension, Json};
use serde::{Deserialize, Serialize};

use crate::api::AppState;
use crate::errors::AppError;
use crate::models::Traveler;

const KEEP_PER_PLUGIN: usize = 40;
const KEEP_PLUGINS: usize = 64;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Entry {
    pub message: String,
    /// The file of the plugin's `web/` the error came from, when known.
    pub source: String,
    pub line: u64,
    pub at: String,
}

#[derive(Clone, Default)]
pub struct Console {
    entries: Arc<Mutex<HashMap<String, VecDeque<Entry>>>>,
}

impl Console {
    pub fn record(&self, plugin: &str, entry: Entry) {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        if !entries.contains_key(plugin) && entries.len() >= KEEP_PLUGINS {
            return;
        }
        let list = entries.entry(plugin.to_string()).or_default();
        // The same error again (a render loop) is one line, not forty.
        if list.back().is_some_and(|last| last.message == entry.message && last.source == entry.source && last.line == entry.line) {
            return;
        }
        list.push_back(entry);
        while list.len() > KEEP_PER_PLUGIN {
            list.pop_front();
        }
    }

    pub fn of(&self, plugin: &str) -> Vec<Entry> {
        let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.get(plugin).map(|list| list.iter().cloned().collect()).unwrap_or_default()
    }

    pub fn clear(&self, plugin: &str) {
        self.entries.lock().unwrap_or_else(|e| e.into_inner()).remove(plugin);
    }
}

#[derive(Deserialize)]
pub struct Report {
    pub plugin: String,
    pub message: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub line: u64,
}

fn bounded(value: &str, max: usize) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(max).collect()
}

/// `POST /api/plugins/dev/console`: the page reports an error of a plugin's
/// window. Only an installed plugin's name is kept.
pub async fn report(
    State(state): State<AppState>,
    Extension(_traveler): Extension<Traveler>,
    Json(body): Json<Report>,
) -> Result<Json<serde_json::Value>, AppError> {
    crate::plugins::loader::validate_plugin_name(&body.plugin)?;
    let known = state.plugins.is_installed(&body.plugin);
    if known && !body.message.trim().is_empty() {
        state.plugin_console.record(
            &body.plugin,
            Entry {
                message: bounded(&body.message, 600),
                source: bounded(&body.source, 200),
                line: body.line,
                at: chrono::Local::now().format("%H:%M:%S").to_string(),
            },
        );
    }
    Ok(Json(serde_json::json!({ "success": true, "data": { "recorded": known } })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(message: &str) -> Entry {
        Entry { message: message.into(), source: "plugin.js".into(), line: 3, at: String::new() }
    }

    #[test]
    fn the_last_errors_of_each_plugin_are_kept_without_repeats() {
        let console = Console::default();
        for _ in 0..5 {
            console.record("notes", entry("x is not defined"));
        }
        console.record("notes", entry("other"));
        console.record("tasks", entry("mine"));
        assert_eq!(console.of("notes").len(), 2, "a repeated error is one line");
        assert_eq!(console.of("tasks")[0].message, "mine");
        for index in 0..100 {
            console.record("notes", entry(&format!("error {index}")));
        }
        let kept = console.of("notes");
        assert_eq!(kept.len(), KEEP_PER_PLUGIN);
        assert_eq!(kept.last().unwrap().message, "error 99");
        console.clear("notes");
        assert!(console.of("notes").is_empty());
        assert_eq!(bounded("a\n  b\tc", 4), "a b ");
    }
}
