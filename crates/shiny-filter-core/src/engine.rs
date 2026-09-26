//! The ad-blocking engine: filter lists in, per-request verdicts out.
//!
//! This is a thin, deliberate wrapper around `adblock::Engine` (Brave's
//! adblock-rust). It is the *light* half of the old `shiny-filter::engine`:
//! everything that needs no network client and no async runtime. The server
//! downloads and compiles the lists (`shiny_filter::load`); the `peakd` kiosk
//! shell restores the compiled engine from the shared cache and checks requests
//! in-process. Keeping this crate free of `wreq`/BoringSSL is what lets the
//! shell link the engine without dragging a TLS stack and a tokio runtime in.
//!
//! Two things it adds beyond the raw engine:
//!
//! 1. **A cache.** Parsing EasyList + EasyPrivacy costs hundreds of
//!    milliseconds; `Engine::serialize()` + `deserialize()` turns that into a
//!    memory-map-and-go on every start after the first. The benchmarks measure
//!    exactly this delta.
//! 2. **A verdict type.** Callers want "block / allow / rewrite to X / serve
//!    this resource instead", not a raw `BlockerResult` plus knowledge of
//!    adblock-rust's precedence rules.

use std::path::{Path, PathBuf};
use std::time::Duration;

use adblock::lists::{FilterSet, ParseOptions};
use adblock::request::Request;
use adblock::Engine;
use parking_lot::RwLock;
use std::sync::Arc;

/// What to do with one request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Forward it upsteam unmodified.
    Allow,
    /// Refuse it. The proxy answers with a synthetic empty response.
    Block,
    /// Forward the (possibly modified) URL instead of the original.
    Rewrite(String),
    /// Serve this resource body in place of the request, as produced by a
    /// `redirect=` / `redirect-rule=` filter. `body` is the decoded resource
    /// text when the engine had one available.
    Redirect { url: String, body: Option<String> },
}

impl Verdict {
    pub fn is_blocked(&self) -> bool {
        matches!(self, Verdict::Block)
    }

    /// The URL that should actually be fetched, if the request proceeds.
    pub fn effective_url<'a>(&'a self, requested: &'a str) -> &'a str {
        match self {
            Verdict::Rewrite(url) => url,
            _ => requested,
        }
    }
}

/// How to build the filter engine.
#[derive(Clone, Debug)]
pub struct FilterConfig {
    /// Remote filter lists, fetched on first run and cached.
    pub list_urls: Vec<String>,
    /// Where the compiled engine + list metadata are cached.
    pub cache_dir: PathBuf,
    /// Compile the engine with per-rule debug info (slower, more memory;
    /// only useful when investigating why something was blocked).
    pub debug: bool,
    /// Give up on a filter-list download after this long (used by the loader in
    /// `shiny-filter`; kept here so both halves share one config type).
    pub fetch_timeout: Duration,
    /// When true, never touch the network — use the cache, or an empty engine.
    pub offline: bool,
    /// Extra raw filter text appended after the remote lists (tests, rules the
    /// user pinned, a regional list shipped in the repo).
    pub extra_filters: Vec<String>,
    /// How many filter lines were loaded, for logs and metrics. `FilterSet`
    /// exposes no rule count, so this is counted as lists are parsed.
    pub filter_count: usize,
}

impl FilterConfig {
    pub fn new(cache_dir: impl Into<PathBuf>) -> Self {
        Self {
            list_urls: default_lists(),
            cache_dir: cache_dir.into(),
            debug: false,
            fetch_timeout: Duration::from_secs(20),
            offline: false,
            extra_filters: Vec::new(),
            filter_count: 0,
        }
    }

    pub fn offline(cache_dir: impl Into<PathBuf>) -> Self {
        Self {
            offline: true,
            list_urls: Vec::new(),
            ..Self::new(cache_dir)
        }
    }
}

/// The default list set: the two lists nearly every blocker ships.
pub fn default_lists() -> Vec<String> {
    vec![
        "https://easylist.to/easylist/easylist.txt".to_string(),
        "https://easylist.to/easylist/easyprivacy.txt".to_string(),
    ]
}

/// A shared, hot-swappable filter engine.
///
/// The engine sits behind an `RwLock` so a background list refresh can swap in
/// a freshly compiled engine while requests keep being served by the old one.
#[derive(Clone)]
pub struct AdFilter {
    engine: Arc<RwLock<Engine>>,
    /// Number of filter rules currently loaded (for logs and metrics).
    rules: Arc<RwLock<usize>>,
    /// Cosmetic selectors per hostname are requested per document; the engine
    /// already caches internally, so we only keep the config here.
    pub config: Arc<FilterConfig>,
}

impl AdFilter {
    /// An engine with no rules — never blocks anything.
    ///
    /// Useful as a safe fallback and as the baseline in benchmarks.
    pub fn empty() -> Self {
        Self::from_lists(Vec::<String>::new(), false)
    }

    /// Build synchronously from raw filter-list texts.
    ///
    /// Parsing is CPU-bound, so async callers should run this inside
    /// `tokio::task::spawn_blocking`.
    pub fn from_lists<I, S>(lists: I, debug: bool) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut set = FilterSet::new(debug);
        let mut rule_lines = 0usize;
        for list in lists {
            let text = list.into();
            // Count non-comment, non-blank lines; a cheap, honest proxy for
            // "how many rules are loaded".
            rule_lines += text
                .lines()
                .filter(|l| {
                    let t = l.trim();
                    !t.is_empty() && !t.starts_with('!') && !t.starts_with('[')
                })
                .count();
            set.add_filter_list(text, ParseOptions::default());
        }
        let engine = Engine::new_with_filter_set(set);
        Self {
            engine: Arc::new(RwLock::new(engine)),
            rules: Arc::new(RwLock::new(rule_lines)),
            config: Arc::new(FilterConfig::offline(".shiny-filter-cache")),
        }
    }

    /// Attach the config that produced this engine (used by the loader so the
    /// cache signature stays inspectable).
    pub fn with_config(mut self, config: FilterConfig) -> Self {
        self.config = Arc::new(config);
        self
    }

    /// Restore an engine from `Engine::serialize()` output.
    pub fn from_serialized(bytes: &[u8], rules: usize) -> Result<Self, String> {
        let mut engine = Engine::new_with_filter_set(FilterSet::new(false));
        engine
            .deserialize(bytes)
            .map_err(|err| format!("deserialize failed: {err}"))?;
        Ok(Self {
            engine: Arc::new(RwLock::new(engine)),
            rules: Arc::new(RwLock::new(rules)),
            config: Arc::new(FilterConfig::offline(".shiny-filter-cache")),
        })
    }

    pub fn serialize(&self) -> Result<Vec<u8>, String> {
        Ok(self.engine.read().serialize())
    }

    pub fn rule_count(&self) -> usize {
        *self.rules.read()
    }

    /// The verdict for one already-classified request.
    ///
    /// `Err` means the URL could not be parsed; callers must treat that as
    /// **allow** (fail open) — a network filter should never be the reason a
    /// page breaks in a way the user cannot diagnose.
    pub fn check(&self, request: &Request) -> Verdict {
        let engine = self.engine.read();
        let result = engine.check_network_request(request);

        // `important` matches are unconditional; otherwise an exception rule
        // wins over a blocking rule. This mirrors adblock-rust's own
        // `BlockerResult::should_block` precedence.
        if result.should_block() {
            return Verdict::Block;
        }

        if let Some(redirect) = result.redirect {
            // `redirect` carries the *body* of the replacement resource. Until
            // resource bundles are wired in we cannot synthesise a data URL
            // faithfully, so we only honour a redirect we can express.
            return Verdict::Redirect {
                url: result
                    .rewritten_url
                    .clone()
                    .unwrap_or_else(|| request.url.clone()),
                body: Some(redirect),
            };
        }

        if let Some(rewritten) = result.rewritten_url {
            if rewritten != request.url {
                return Verdict::Rewrite(rewritten);
            }
        }

        Verdict::Allow
    }

    /// Cosmetic-filter CSS for one document URL, plus any scriptlets the lists
    /// want injected into that document.
    ///
    /// `hide_selectors` here are the **hostname-specific** rules for this page.
    /// Generic `##.ad` rules are matched against the classes and ids actually
    /// present in the page (`hidden_class_id_selectors`), so they need the
    /// document's class/id set.
    pub fn cosmetic_css(&self, document_url: &str) -> String {
        let engine = self.engine.read();
        let resources = engine.url_cosmetic_resources(document_url);
        css_from_selectors(resources.hide_selectors.iter())
    }

    /// Cosmetic CSS including generic rules matching `classes` / `ids` found in
    /// the document.
    pub fn cosmetic_css_for_classes(
        &self,
        document_url: &str,
        classes: &[String],
        ids: &[String],
    ) -> String {
        let engine = self.engine.read();
        let resources = engine.url_cosmetic_resources(document_url);
        let mut css = css_from_selectors(resources.hide_selectors.iter());
        if !resources.generichide {
            let generic = engine.hidden_class_id_selectors(
                classes.iter().map(|s| s.as_str()),
                ids.iter().map(|s| s.as_str()),
                &resources.exceptions,
            );
            css.push_str(&css_from_selectors(generic.iter()));
        }
        css
    }

    /// Scriptlet/microscript JS the filter lists want injected into a document.
    pub fn injected_script(&self, document_url: &str) -> String {
        let engine = self.engine.read();
        engine.url_cosmetic_resources(document_url).injected_script
    }

    /// Convenience for tests and the benchmark harness: classify + check in one
    /// call, returning the verdict.
    pub fn check_url(&self, url: &str, source_url: &str, kind: &str, method: &str) -> Verdict {
        match Request::new(url, source_url, kind, method) {
            Ok(req) => self.check(&req),
            Err(_) => Verdict::Allow,
        }
    }
}

/// Metadata the compiled cache is stamped with.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct CacheMeta {
    pub signature: String,
    pub rules: usize,
    pub compiled_at: String,
}

/// Build the `display:none` block for a set of cosmetic selectors.
///
/// A selector that is empty, or that is only whitespace, is skipped rather
/// than emitted: an empty key would produce `{display:none}` with no selector,
/// which is a CSS parse error that can invalidate the whole stylesheet.
fn css_from_selectors<'a, I: Iterator<Item = &'a String>>(selectors: I) -> String {
    let mut css = String::new();
    for selector in selectors {
        let trimmed = selector.trim();
        if trimmed.is_empty() {
            continue;
        }
        css.push_str(trimmed);
        css.push_str("{display:none !important;}\n");
    }
    css
}

/// Cache identity: the list set + debug flag + crate version. Changing any of
/// them invalidates the compiled cache, which is what stops a stale engine
/// from silently serving rules the user turned off.
pub fn signature_of(config: &FilterConfig) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    config.list_urls.hash(&mut hasher);
    config.debug.hash(&mut hasher);
    config.extra_filters.hash(&mut hasher);
    env!("CARGO_PKG_VERSION").hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

pub fn chrono_stamp() -> String {
    // Avoid a chrono dependency just for a log line.
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => format!("unix:{}", d.as_secs()),
        Err(_) => "unix:0".to_string(),
    }
}

/// Path helper so both surfaces agree on the cache location.
pub fn default_cache_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("adfilter")
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_RULES: &str = r#"
! test list
||ads.example.com^
||tracker.example.net^$third-party
example.com##.ad-banner
"#;

    fn test_filter() -> AdFilter {
        AdFilter::from_lists([TEST_RULES], false)
    }

    #[test]
    fn blocks_a_listed_host() {
        let filter = test_filter();
        assert_eq!(
            filter.check_url(
                "https://ads.example.com/banner.png",
                "https://news.example.org/",
                "image",
                "GET"
            ),
            Verdict::Block
        );
    }

    #[test]
    fn allows_an_unlisted_host() {
        let filter = test_filter();
        assert_eq!(
            filter.check_url(
                "https://cdn.example.org/logo.png",
                "https://news.example.org/",
                "image",
                "GET"
            ),
            Verdict::Allow
        );
    }

    #[test]
    fn third_party_only_rule_respects_party() {
        let filter = test_filter();
        // third-party from news.example.org -> blocked
        assert_eq!(
            filter.check_url(
                "https://tracker.example.net/p.gif",
                "https://news.example.org/",
                "image",
                "GET"
            ),
            Verdict::Block
        );
        // first-party from tracker.example.net itself -> allowed
        assert_eq!(
            filter.check_url(
                "https://tracker.example.net/p.gif",
                "https://tracker.example.net/",
                "image",
                "GET"
            ),
            Verdict::Allow
        );
    }

    #[test]
    fn cosmetic_selectors_are_reported() {
        let filter = test_filter();
        let css = filter.cosmetic_css("https://example.com/page");
        assert!(css.contains(".ad-banner"), "got: {css:?}");
        assert!(css.contains("display:none"));
    }

    #[test]
    fn empty_engine_never_blocks() {
        let filter = AdFilter::empty();
        assert_eq!(
            filter.check_url(
                "https://ads.example.com/x.png",
                "https://example.com/",
                "image",
                "GET"
            ),
            Verdict::Allow
        );
    }

    #[test]
    fn serialize_round_trip_preserves_rules() {
        let filter = test_filter();
        let bytes = filter.serialize().unwrap();
        let restored = AdFilter::from_serialized(&bytes, filter.rule_count()).unwrap();
        assert_eq!(
            restored.check_url(
                "https://ads.example.com/banner.png",
                "https://news.example.org/",
                "image",
                "GET"
            ),
            Verdict::Block
        );
    }

    #[test]
    fn unparseable_url_fails_open() {
        let filter = test_filter();
        assert_eq!(filter.check_url("not a url", "", "image", "GET"), Verdict::Allow);
    }
}
