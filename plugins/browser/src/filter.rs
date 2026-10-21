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
/// Rules appended after the downloaded lists.
///
/// EasyList and EasyPrivacy already carry most of YouTube's ad and tracking
/// requests; these are the player's own endpoints, which the community lists
/// have covered only intermittently. The *video* ads are not reachable from
/// here at all — they are served from the same `googlevideo.com/videoplayback`
/// shape as the video — and that half is the shell's document-start scriptlet
/// (`crates/peakd/js/youtube-ads.js`).
fn extra_filters() -> Vec<String> {
    [
        "||youtube.com/api/stats/ads",
        "||youtube.com/pagead/",
        "||youtube.com/ptracking",
        "||youtube.com/get_midroll_info",
    ]
    .iter()
    .map(|rule| (*rule).to_string())
    .collect()
}

/// The config both loaders start from.
fn config_for(ctx: &PluginCtx) -> FilterConfig {
    let mut config = FilterConfig::new(cache_dir(ctx));
    config.extra_filters = extra_filters();
    config
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
    let config = config_for(ctx);
    let filter = shiny_filter::engine::load(config).await;
    *slot().write() = Some(filter.clone());
    Some(filter)
}

/// Re-download and recompile the lists, replacing the live engine.
pub async fn reload(ctx: &PluginCtx) -> usize {
    let _guard = LOAD_LOCK.lock().await;
    let mut config = config_for(ctx);
    // Ignore a cache written by the previous run so this really re-fetches. The
    // signature hashes the whole config, extra rules included, so the nonce is
    // what stops `load` from taking the cache fast path it now would match.
    config.extra_filters.push(format!(
        "! refresh {}",
        shiny_filter::engine::chrono_stamp()
    ));
    let filter = shiny_filter::engine::load(config).await;
    let rules = filter.rule_count();
    *slot().write() = Some(filter);
    rules
}

#[cfg(test)]
mod tests {
    use super::*;
    use shiny_filter::Verdict;

    #[test]
    fn cache_dir_honours_the_env_override() {
        std::env::set_var("ADFILTER_DIR", "/srv/shiny/adfilter");
        let ctx = crate::history::tests_support::ctx_for("/tmp/browser-filter-test.db");
        assert_eq!(cache_dir(&ctx), PathBuf::from("/srv/shiny/adfilter"));
        std::env::remove_var("ADFILTER_DIR");
    }

    /// The extra rules must catch the player's ad endpoints and nothing else —
    /// over-blocking here breaks the page the user is watching.
    #[test]
    fn the_youtube_rules_block_the_ad_endpoints_only() {
        let filter = AdFilter::from_lists(extra_filters(), false);
        let watch = "https://www.youtube.com/watch?v=abc";
        for url in [
            "https://www.youtube.com/api/stats/ads?ver=2&ns=yt",
            "https://www.youtube.com/pagead/id",
            "https://www.youtube.com/ptracking?video_id=abc",
            "https://www.youtube.com/get_midroll_info?cpn=abc",
        ] {
            assert_eq!(
                filter.check_url(url, watch, "xhr", "GET"),
                Verdict::Block,
                "should block {url}"
            );
        }
        for url in [
            // The player API itself carries no ads (the shell's scriptlet
            // rewrites its *body*), and the page and the media must load.
            "https://www.youtube.com/youtubei/v1/player?key=x",
            watch,
            "https://rr1---sn-x.googlevideo.com/videoplayback?expire=1",
        ] {
            assert_eq!(
                filter.check_url(url, watch, "xhr", "GET"),
                Verdict::Allow,
                "should allow {url}"
            );
        }
    }
}
