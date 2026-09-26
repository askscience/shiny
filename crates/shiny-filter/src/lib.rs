//! `shiny-filter` — the one ad-blocking engine behind Shiny's browser.
//!
//! Where it runs:
//!
//! * **the `browser` plugin** uses it **server-side**: the `browser_read` tool
//!   and the related-news shelf fetch through it, and link previews use its
//!   classifier (`src/fetch.rs`, `src/news.rs`, `src/preview.rs`).
//! * the kiosk shells (`crates/peakd` on Linux, `crates/peakd-mac` on macOS)
//!   render the open web in native child web views at the page's true origin —
//!   no proxy in the page path.
//!
//! The proxy machinery below is still the engine the plugin's fetch path uses,
//! and it is where the request classifier and rewriting rules live. It is not
//! how the Browser window renders pages: rendering through an origin-rewriting
//! proxy broke anti-bot systems (Cloudflare's challenge cookies were re-scoped
//! to the proxy origin), which is why the window moved to native child views.
//!
//! # Layout
//!
//! * [`engine`] — the `adblock::Engine` wrapper: filter lists → `Engine`,
//!   on-disk cache, and the block/allow verdict for one request.
//! * [`classify`] — HTTP request → adblock request type + source/origin plumbing.
//! * [`urls`] — the proxy URL scheme: `http://<proxy>/p/<scheme>/<host><path>`.
//! * [`rewrite`] — HTML/CSS rewriting so subresources keep flowing through us.
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

pub use classify::{classify_request, RequestKind};
pub use engine::{AdFilter, Verdict};
pub use metrics::{Metrics, MetricsSnapshot};
pub use proxy::{run_proxy, ProxyConfig, ProxyHandle};
pub use urls::{decode_target, encode_target, PROXY_PREFIX};
