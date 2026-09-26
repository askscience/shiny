//! `shiny-filter` — the one ad-blocking engine behind Shiny's browser.
//!
//! Where it runs:
//!
//! * **the `browser` plugin** uses it **server-side** to fetch and compile the
//!   filter lists (EasyList/EasyPrivacy) and to fetch pages as text; the
//!   related-news shelf and link previews use its classifier
//!   (`src/filter.rs`, `src/fetch.rs`, `src/news.rs`, `src/preview.rs`). The
//!   compiled engine is written to a shared cache.
//! * **the kiosk shells** (`crates/peakd` on Linux) render the open web in
//!   native child web views at the page's true origin — no proxy in the page
//!   path — and block requests **in-process** with a
//!   `QWebEngineUrlRequestInterceptor` that queries [`shiny_filter_core`]
//!   restored from that cache. The shell links only `shiny-filter-core`, so it
//!   pulls no BoringSSL/tokio.
//!
//! The proxy machinery below is retained for its request classifier and
//! rewriting rules and for the benchmark harness, but it is **not** how the
//! Browser window renders pages: rendering through an origin-rewriting proxy
//! broke anti-bot systems (Cloudflare's challenge cookies were re-scoped to the
//! proxy origin), which is why the window moved to native child views.
//!
//! # Layout
//!
//! * [`engine`] — the network half: fetch lists, compile, write the shared
//!   cache. The engine type itself is re-exported from `shiny-filter-core`.
//! * `shiny-filter-core` — the light engine (no HTTP client, no runtime):
//!   `adblock::Engine` wrapper, cache format, verdicts, classification and
//!   cosmetic selectors. This is what the shell links.
//! * [`rewrite`] — HTML/CSS rewriting so subresources keep flowing through the
//!   proxy.
//! * [`inject`] — the document-start runtime shim and cosmetic-filter CSS.
//! * [`proxy`] — the HTTP proxy server (absolute-URI requests + `CONNECT`).
//! * [`metrics`] — counters that make the benchmarks meaningful.
//!
//! # The `CONNECT` limitation, stated plainly
//!
//! HTTPS traffic that arrives as a `CONNECT` tunnel is end-to-end encrypted:
//! this proxy can decide whether to open the tunnel (host-level blocking) but
//! cannot see, and therefore cannot filter, individual URLs inside it. That is
//! the same limit every browser-integrated blocker without a MITM certificate
//! has (uBlock Origin in Safari, Brave on iOS). MITM is deliberately **out of
//! scope**: it would break TLS integrity. In-app rendering goes through the
//! plain-HTTP path, where filtering *is* per-URL; that is the richer surface.

pub mod classify;
pub mod engine;
pub mod inject;
pub mod metrics;
pub mod proxy;
pub mod rewrite;
pub mod urls;

pub use classify::{build_adblock_request, classify_request, RequestKind};
pub use engine::{default_cache_dir, default_lists, AdFilter, FilterConfig, Verdict};
pub use metrics::{Metrics, MetricsSnapshot};
pub use proxy::{run_proxy, ProxyConfig, ProxyHandle};
pub use urls::{decode_target, encode_target, PROXY_PREFIX};
