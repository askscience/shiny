//! YouTube ad blocking for the shell's views.
//!
//! The Browser plugin's pages are rendered by this shell, and YouTube's video
//! ads are not reachable from a filter list (the ad and the video share one
//! `googlevideo.com/videoplayback` shape), so the shell injects a script at
//! document creation instead. It is registered as `peakd:youtube` in
//! `main.rs`; the window's shield toggle drops it from every view's script
//! collection through `shim::set_filter_scripts`.
//!
//! The script itself is compiled in, like the benchmark instrumentation: there
//! is no path to get wrong at runtime.

/// The YouTube document-start scriptlet. See the file's header for what it does
/// and why it exists.
pub const YOUTUBE_ADS_JS: &str = include_str!("../js/youtube-ads.js");

/// The name the script is registered and gated under. `Shell::scriptSources`
/// (shim/peakd.cpp) matches on exactly this string.
pub const SCRIPT_NAME: &str = "peakd:youtube";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ships_the_scriptlet_with_its_guard_and_host_gate() {
        assert!(!YOUTUBE_ADS_JS.is_empty());
        // Runs once per document, and only where it can do anything.
        assert!(YOUTUBE_ADS_JS.contains("window.__peakdYoutubeAds"));
        assert!(YOUTUBE_ADS_JS.contains("youtube.com"));
        // The ad keys uBO renames (see uAssets filters/quick-fixes.txt).
        assert!(YOUTUBE_ADS_JS.contains("adPlacements"));
        assert!(YOUTUBE_ADS_JS.contains("adSlots"));
    }

    #[test]
    fn the_script_is_scoped_to_the_guard() {
        // The whole scriptlet body must sit inside the guarded IIFE: a bare
        // statement here would run at document creation on every site.
        assert!(YOUTUBE_ADS_JS.trim_start().starts_with("/*"));
        assert!(YOUTUBE_ADS_JS.trim_end().ends_with("})();"));
        assert_eq!(SCRIPT_NAME, "peakd:youtube");
    }
}
