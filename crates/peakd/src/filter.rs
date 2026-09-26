//! In-process ad blocking for the native browser child views.
//!
//! The proxy is gone (origin rewriting broke anti-bot cookies), so the shell
//! filters with a `QWebEngineUrlRequestInterceptor`: for every request QtWebEngine
//! makes, the C++ shim calls [`is_blocked`] and blocks the request when the
//! engine says so. The engine itself is compiled by the server
//! (`shiny_filter::load`) and restored here from the shared on-disk cache, so
//! this crate links only `shiny-filter-core` — no HTTP client, no tokio.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::OnceLock;

use shiny_filter_core::classify::build_adblock_request;
use shiny_filter_core::{AdFilter, CacheMeta, RequestKind, Verdict};

/// The User-Agent sent to Google's sign-in hosts.
///
/// Google blocks QtWebEngine-based browsers as "embedded"; the proven workaround
/// (qutebrowser v3.3, 2024) is a desktop Firefox UA for the accounts host. Kept
/// in one place so it can be refreshed as Google's checks change.
pub const FIREFOX_UA: &str =
    "Mozilla/5.0 (X11; Linux x86_64; rv:131.0) Gecko/20100101 Firefox/131.0";

/// Hosts that get the Firefox User-Agent.
///
/// Deliberately a small explicit list, not a `.google.com` suffix: spoofing
/// `www.google.com`/`drive.google.com` would break those sites' UA checks.
pub const GOOGLE_SIGNIN_HOSTS: &[&str] = &[
    "accounts.google.com",
    "accounts.youtube.com",
    "myaccount.google.com",
    "oauth2.googleapis.com",
];

/// The UA override for `host`, if any.
pub fn user_agent_override(host: &str) -> Option<&'static str> {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    GOOGLE_SIGNIN_HOSTS
        .iter()
        .any(|h| host == *h)
        .then_some(FIREFOX_UA)
}

/// Live filter state: the engine (if one could be loaded), the user's toggle,
/// and a counter for the window's shield badge.
pub struct FilterState {
    engine: Option<AdFilter>,
    enabled: AtomicBool,
    blocked: AtomicU64,
}

static FILTER: OnceLock<FilterState> = OnceLock::new();

fn state() -> Option<&'static FilterState> {
    FILTER.get()
}

/// Load the compiled engine from `<dir>/engine.dat` + `engine.json`.
///
/// Never fatal: a missing or unreadable cache leaves the shell unfiltered (the
/// user can still browse), exactly like the old "fail open" rule.
pub fn install(adfilter_dir: &Path) {
    let engine = load_engine(adfilter_dir);
    if let Some(e) = &engine {
        println!(
            "peakd: ad filter ready ({} rules from {})",
            e.rule_count(),
            adfilter_dir.display()
        );
    } else {
        println!(
            "peakd: no ad filter cache in {} — running unfiltered",
            adfilter_dir.display()
        );
    }
    let enabled = engine.is_some();
    let _ = FILTER.set(FilterState {
        engine,
        enabled: AtomicBool::new(enabled),
        blocked: AtomicU64::new(0),
    });
}

fn load_engine(dir: &Path) -> Option<AdFilter> {
    let meta_raw = std::fs::read_to_string(dir.join("engine.json")).ok()?;
    let meta: CacheMeta = serde_json::from_str(&meta_raw).ok()?;
    let bytes = std::fs::read(dir.join("engine.dat")).ok()?;
    match AdFilter::from_serialized(&bytes, meta.rules) {
        Ok(filter) => Some(filter),
        Err(err) => {
            eprintln!("peakd: ad filter cache unusable: {err}");
            None
        }
    }
}

/// Turn blocking on or off (the window's shield toggle).
pub fn set_enabled(on: bool) {
    if let Some(s) = state() {
        s.enabled.store(on, Ordering::Relaxed);
    }
}

/// `(enabled, blocked_count, rule_count)` for the shield state.
pub fn stats() -> (bool, u64, usize) {
    match state() {
        Some(s) => (
            s.enabled.load(Ordering::Relaxed),
            s.blocked.load(Ordering::Relaxed),
            s.engine.as_ref().map(AdFilter::rule_count).unwrap_or(0),
        ),
        None => (false, 0, 0),
    }
}

/// The verdict for one request, as the C++ interceptor asks for it.
///
/// `resource_type` is Qt's `QWebEngineUrlRequestInfo::ResourceType` numeric
/// value; `method` is the HTTP verb. Fails open on anything unparseable.
pub fn is_blocked(url: &str, first_party: &str, resource_type: i32, method: &str) -> bool {
    let Some(s) = state() else { return false };
    if !s.enabled.load(Ordering::Relaxed) {
        return false;
    }
    let Some(engine) = s.engine.as_ref() else {
        return false;
    };
    let kind = kind_from_qt(resource_type);
    let Some(request) = build_adblock_request(url, first_party, kind, method) else {
        return false;
    };
    let blocked = matches!(engine.check(&request), Verdict::Block);
    if blocked {
        s.blocked.fetch_add(1, Ordering::Relaxed);
    }
    blocked
}

/// Cosmetic CSS for a document URL (host-specific rules only).
///
/// Wired to per-view injection in a follow-up; network blocking is the main
/// value and ships first (PLAN §3 B4).
#[allow(dead_code)]
pub fn cosmetic_css(url: &str) -> String {
    let Some(s) = state() else { return String::new() };
    if !s.enabled.load(Ordering::Relaxed) {
        return String::new();
    }
    s.engine
        .as_ref()
        .map(|e| e.cosmetic_css(url))
        .unwrap_or_default()
}

/// Cosmetic CSS including generic rules matching the document's class/id set.
#[allow(dead_code)]
pub fn cosmetic_css_for(url: &str, classes: &[String], ids: &[String]) -> String {
    let Some(s) = state() else { return String::new() };
    if !s.enabled.load(Ordering::Relaxed) {
        return String::new();
    }
    s.engine
        .as_ref()
        .map(|e| e.cosmetic_css_for_classes(url, classes, ids))
        .unwrap_or_default()
}

/// Scriptlet JS the lists want injected at document-start.
#[allow(dead_code)]
pub fn injected_script(url: &str) -> String {
    let Some(s) = state() else { return String::new() };
    if !s.enabled.load(Ordering::Relaxed) {
        return String::new();
    }
    s.engine
        .as_ref()
        .map(|e| e.injected_script(url))
        .unwrap_or_default()
}

/// Qt's `QWebEngineUrlRequestInfo::ResourceType` → the adblock vocabulary.
///
/// Values are the stable enum ordinals from Qt 6 (see the Qt documentation);
/// the C++ shim passes `static_cast<int>(info.resourceType())`.
fn kind_from_qt(value: i32) -> RequestKind {
    match value {
        0 | 19 | 20 => RequestKind::Document, // MainFrame, NavigationPreload*
        1 => RequestKind::Subdocument,        // SubFrame
        2 => RequestKind::Stylesheet,
        3 => RequestKind::Script,
        4 | 12 => RequestKind::Image, // Image, Favicon
        5 => RequestKind::Font,
        6 => RequestKind::Other,       // SubResource
        7 => RequestKind::Object,
        8 => RequestKind::Media,
        9 | 10 | 15 => RequestKind::Script, // Worker, SharedWorker, ServiceWorker
        13 | 21 => RequestKind::Xhr,   // Xhr, Json (Qt 6.8)
        14 => RequestKind::Ping,
        _ => RequestKind::Other,
    }
}

/// The default cache directory, resolved like the shell's data dir.
#[allow(dead_code)]
pub fn default_dir() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(root) = exe.parent().and_then(Path::parent).and_then(Path::parent) {
            if root.join("Cargo.toml").is_file() {
                return root.join("data").join("adfilter");
            }
        }
    }
    PathBuf::from("data/adfilter")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn google_hosts_get_firefox_but_others_do_not() {
        assert_eq!(
            user_agent_override("accounts.google.com"),
            Some(FIREFOX_UA)
        );
        assert_eq!(user_agent_override("ACCOUNTS.GOOGLE.COM."), Some(FIREFOX_UA));
        assert_eq!(user_agent_override("www.google.com"), None);
        assert_eq!(user_agent_override("drive.google.com"), None);
        assert_eq!(user_agent_override("evil-accounts.google.com"), None);
    }

    #[test]
    fn qt_resource_types_map_to_the_engine_vocabulary() {
        assert_eq!(kind_from_qt(0), RequestKind::Document);
        assert_eq!(kind_from_qt(3), RequestKind::Script);
        assert_eq!(kind_from_qt(4), RequestKind::Image);
        assert_eq!(kind_from_qt(13), RequestKind::Xhr);
        assert_eq!(kind_from_qt(999), RequestKind::Other);
    }

    #[test]
    fn absent_state_fails_open() {
        // No `install()` in this process → nothing is blocked.
        assert!(!is_blocked("https://ads.example.com/x.png", "https://x/", 4, "GET"));
    }
}
