//! The server's ad-filter engine: fetch + compile the lists, hand the compiled
//! cache to the `peakd` shell.
//!
//! The shell does the per-request blocking in-process (a
//! `QWebEngineUrlRequestInterceptor`), which needs the compiled engine but not
//! an HTTP client or a runtime. This module owns the network half and writes
//! `engine.dat` + `engine.json` into a shared directory the shell reads
//! (`PEAKD_ADFILTER_DIR`).

use std::path::PathBuf;
use std::sync::OnceLock;

use parking_lot::RwLock;
use shiny_filter::{AdFilter, FilterConfig};
use shiny_plugin_sdk::services::PluginCtx;

/// The shared compiled engine. `None` until the first successful load.
static FILTER: OnceLock<RwLock<Option<AdFilter>>> = OnceLock::new();

/// Serializes loads so a burst of requests does not compile the lists twice.
static LOAD_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn slot() -> &'static RwLock<Option<AdFilter>> {
    FILTER.get_or_init(|| RwLock::new(None))
}

/// Where the compiled cache lives.
///
/// `ADFILTER_DIR` wins; otherwise the per-user data dir
/// (`$XDG_DATA_HOME/shiny/adfilter`, defaulting to `~/.local/share/...`), which
/// is exactly what the shell resolves when `PEAKD_ADFILTER_DIR` is unset. Both
/// processes run as the same user, so they agree without any fixed path or
/// username.
pub fn cache_dir(_ctx: &PluginCtx) -> PathBuf {
    if let Ok(dir) = std::env::var("ADFILTER_DIR") {
        if !dir.trim().is_empty() {
            return PathBuf::from(dir);
        }
    }
    if let Some(data) = std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        return PathBuf::from(data).join("shiny").join("adfilter");
    }
    if let Some(home) = std::env::var_os("HOME").filter(|v| !v.is_empty()) {
        return PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("shiny")
            .join("adfilter");
    }
    PathBuf::from("data/adfilter")
}

/// The current engine, if one has been loaded.
pub fn get() -> Option<AdFilter> {
    slot().read().clone()
}

pub fn rules() -> usize {
    get().map(|f| f.rule_count()).unwrap_or(0)
}

/// Is this document URL blocked outright? Used by `browser_read`.
pub fn document_blocked(url: &str) -> bool {
    get()
        .map(|f| {
            matches!(
                f.check_url(url, url, "document", "GET"),
                shiny_filter::Verdict::Block
            )
        })
        .unwrap_or(false)
}

/// Load the engine once. Concurrent callers wait for the first load.
///
/// Never fails hard: a machine with no network gets an empty engine, and the
/// shell simply browses unfiltered.
pub async fn ensure_loaded(ctx: &PluginCtx) -> Option<AdFilter> {
    if let Some(filter) = get() {
        return Some(filter);
    }
    let _guard = LOAD_LOCK.lock().await;
    if let Some(filter) = get() {
        return Some(filter);
    }
    let config = FilterConfig::new(cache_dir(ctx));
    let filter = shiny_filter::engine::load(config).await;
    *slot().write() = Some(filter.clone());
    Some(filter)
}

/// Re-download and recompile the lists, replacing the live engine.
pub async fn reload(ctx: &PluginCtx) -> usize {
    let _guard = LOAD_LOCK.lock().await;
    let mut config = FilterConfig::new(cache_dir(ctx));
    // Ignore a cache written by the previous run so this really re-fetches.
    config.extra_filters.clear();
    let filter = shiny_filter::engine::load(config).await;
    let rules = filter.rule_count();
    *slot().write() = Some(filter);
    rules
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_dir_honours_the_env_override() {
        std::env::set_var("ADFILTER_DIR", "/srv/shiny/adfilter");
        let ctx = crate::history::tests_support::ctx_for("/tmp/browser-filter-test.db");
        assert_eq!(cache_dir(&ctx), PathBuf::from("/srv/shiny/adfilter"));
        std::env::remove_var("ADFILTER_DIR");
    }
}
