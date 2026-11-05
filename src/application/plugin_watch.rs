//! Plugins that change on disk are loaded again while the server runs.
//!
//! A package upgrade, the Craft store, or a developer copying a rebuilt
//! library into the plugin directory all leave new files behind; nothing told
//! the server. This worker fingerprints every plugin directory every few
//! seconds (a `stat` per file) and, when a plugin's code differs from what is
//! loaded **and has stopped changing** (the same fingerprint twice in a row,
//! so a copy in progress is never loaded half way), runs the manager's rescan
//! and rebuilds the router.
//!
//! `PLUGIN_WATCH_SECS` sets the interval; `0` turns the worker off.

use crate::api::AppState;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

const DEFAULT_SECS: u64 = 10;

pub fn interval() -> Option<Duration> {
    let secs = std::env::var("PLUGIN_WATCH_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_SECS);
    (secs > 0).then(|| Duration::from_secs(secs))
}

pub fn start(
    state: AppState,
    rebuild: Arc<dyn Fn() + Send + Sync>,
) -> Option<tokio::task::JoinHandle<()>> {
    let every = interval()?;
    Some(tokio::spawn(async move {
        // Code digests seen on the previous tick, by plugin name.
        let mut previous: BTreeMap<String, String> = BTreeMap::new();
        loop {
            tokio::time::sleep(every).await;
            let plugins = state.plugins.clone();
            let disk = match tokio::task::spawn_blocking(move || plugins.disk_plugins()).await {
                Ok(disk) => disk,
                Err(_) => continue,
            };
            let current: BTreeMap<String, String> = disk
                .iter()
                .map(|(name, p)| (name.clone(), p.digest.code.clone()))
                .collect();
            let settled_change = current.iter().any(|(name, code)| {
                let loaded = state.plugins.digest_for(name).map(|d| d.code);
                loaded.as_deref() != Some(code) && previous.get(name) == Some(code)
            });
            previous = current;
            if !settled_change {
                continue;
            }
            let report = state.plugins.rescan(state.plugin_ctx()).await;
            if report.changed() {
                tracing::info!(
                    "plugins changed on disk: updated {:?}, added {:?}",
                    report.updated,
                    report.added
                );
                rebuild();
            }
            for (name, error) in &report.failed {
                tracing::warn!("plugin {name}: the new files could not be loaded: {error}");
            }
        }
    }))
}
