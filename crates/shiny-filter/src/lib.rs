//! `shiny-filter` — the one ad-blocking engine shared by both browser surfaces.
//!
//! Shiny ships two ways to browse:
//!
//! * **`peakd`**, a native shell (tao + wry) whose webview is pointed at
//!   this crate's proxy.
//! * **the `browser` plugin**, whose window in the Shiny desktop embeds an
//!   iframe whose *origin is this same proxy*.
//!
//! Neither the native webview nor an iframe can filter requests on its own —
//! wry exposes no HTTP interception hook, and `WKWebView`/`WebKitGTK` have no
//! `onBeforeRequest` equivalent. So filtering has to live in a proxy, and this
//! crate is that proxy. Both surfaces therefore run through *exactly* the same
//! `adblock::Engine`, the same request classifier, and the same injection
//! pipeline — which is what makes them "the same engine" in the only sense
//! that survives contact with the platforms.
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
