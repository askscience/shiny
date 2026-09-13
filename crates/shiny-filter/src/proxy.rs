//! The filtering HTTP proxy.
//!
//! Supports both shapes a browser uses:
//!
//! * **Absolute-URI requests** — a webview configured with an HTTP proxy sends
//!   `GET http://example.com/a.js HTTP/1.1`. The request target names the
//!   upstream directly, so it is filtered and forwarded as-is.
//! * **Absolute-path requests** — when the proxy is used as an *origin* (the
//!   in-app plugin's iframe), the browser sends `GET /p/https/example.com/a.js`.
//!   [`crate::urls`] decodes that back to the real URL.
//! * **`CONNECT`** — HTTPS tunnels. See the module note in [`crate::lib`]:
//!   inside a tunnel the bytes are encrypted, so only the hostname can be
//!   filtered. The tunnel is still proxied (and counted) rather than refused,
//!   so HTTPS works.
//!
//! Document responses additionally get the rewriter and the injection block;
//! CSS responses get their `url()` references rewritten. Everything else is
//! streamed through untouched.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use axum::http::{header, HeaderMap, HeaderName, Method, Request, Response, StatusCode, Uri};
use bytes::Bytes;
use http_body_util::BodyExt;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use tokio::io::copy_bidirectional;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;

use crate::classify::classify_request;
use crate::engine::{AdFilter, Verdict};
use crate::inject::{head_injections, insert_after_head, Injections};
use crate::metrics::Metrics;
use crate::rewrite::{rewrite_css, rewrite_html, strip_base_tag};
use crate::urls::{decode_target, is_proxied_path};

/// User-Agent used when the client does not send one.
///
/// Pinned (rather than taken from the running platform webview) so a proxied
/// page renders identically whether it was fetched by the native shell, by the
/// in-app window, or by an AI tool.
pub const BROWSER_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15";

/// How to run the proxy.
#[derive(Clone, Debug)]
pub struct ProxyConfig {
    /// Bind address. The proxy is a local component: default to loopback so it
    /// is not an open relay on the LAN.
    pub bind: SocketAddr,
    /// Scheme+authority the proxy advertises when rewriting documents, e.g.
    /// `http://127.0.0.1:8899`. When `None` it is derived from the bound port
    /// as `http://127.0.0.1:<port>`.
    pub public_base: Option<String>,
    /// Refuse documents larger than this rather than buffering them in memory
    /// to rewrite. Bundles and downloads stream through instead.
    pub max_rewrite_bytes: usize,
    /// Apply the filter to loopback targets too.
    ///
    /// Default `false`. Shiny serves its own UI and this proxy from
    /// `127.0.0.1`, and a user's filter lists must not be able to break the
    /// app that hosts the browser. Tests and benchmarks that exercise the
    /// filter against a loopback fixture set this to `true`.
    pub filter_loopback: bool,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            bind: SocketAddr::from(([127, 0, 0, 1], 0)),
            public_base: None,
            max_rewrite_bytes: 12 * 1024 * 1024,
            filter_loopback: false,
        }
    }
}

impl ProxyConfig {
    pub fn on_port(port: u16) -> Self {
        Self {
            bind: SocketAddr::from(([127, 0, 0, 1], port)),
            public_base: Some(format!("http://127.0.0.1:{port}")),
            ..Self::default()
        }
    }
}

/// A running proxy.
pub struct ProxyHandle {
    addr: SocketAddr,
    base: String,
    shutdown: Arc<Notify>,
    task: tokio::task::JoinHandle<()>,
    pub filter: AdFilter,
    pub metrics: Metrics,
    paused: Arc<std::sync::atomic::AtomicBool>,
}

impl ProxyHandle {
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Scheme+authority to use when rewriting documents or pointing a webview
    /// at this proxy, e.g. `http://127.0.0.1:8899`.
    pub fn base(&self) -> &str {
        &self.base
    }

    pub fn metrics(&self) -> &Metrics {
        &self.metrics
    }

    /// Turn filtering off (or back on) for every request from now on.
    ///
    /// Deliberately *not* per-site and not persisted: the switch is a
    /// "this page is broken, let it load" escape hatch, and coming back
    /// enabled on the next navigation is what a user expects.
    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, std::sync::atomic::Ordering::Relaxed);
    }

    /// Whether filtering is currently paused.
    pub fn is_paused(&self) -> bool {
        self.paused.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Ask the proxy to stop accepting connections.
    pub fn shutdown(&self) {
        self.shutdown.notify_waiters();
    }

    /// Await the server task (useful in tests and for graceful teardown).
    pub async fn join(self) {
        self.task.await.ok();
    }
}

/// Shared state for one proxy instance.
struct ProxyState {
    filter: AdFilter,
    client: reqwest::Client,
    metrics: Metrics,
    base: String,
    max_rewrite_bytes: usize,
    /// See [`ProxyConfig::filter_loopback`].
    filter_loopback: bool,
    /// While true, every request is allowed through untouched.
    ///
    /// Some sites genuinely need their ads and trackers to function (a login
    /// that hangs on a blocked analytics call, a video player with a blocked
    /// prebid script). The window exposes this as a per-session pause, the
    /// same affordance every browser blocker has, so the user is never stuck
    /// with a broken page and no way out.
    ///
    /// An `Arc` shared with [`ProxyHandle`]: the handle flips it and the
    /// request path reads it, so both must be the same flag.
    paused: Arc<std::sync::atomic::AtomicBool>,
}

/// Start the proxy. `filter` is shared: the native shell and the in-app plugin
/// can pass the same [`AdFilter`], which is what makes them one engine.
pub async fn run_proxy(config: ProxyConfig, filter: AdFilter) -> std::io::Result<ProxyHandle> {
    let listener = TcpListener::bind(config.bind).await?;
    let addr = listener.local_addr()?;
    let base = config
        .public_base
        .clone()
        .unwrap_or_else(|| format!("http://127.0.0.1:{}", addr.port()));

    let client = reqwest::Client::builder()
        // The client's own `User-Agent` is forwarded when it sends one. This is
        // the fallback for clients that send none — an HTML fetch made by a
        // tool, for instance — because a bare library UA is refused or served
        // degraded content by a lot of the web.
        .user_agent(BROWSER_USER_AGENT)
        .redirect(reqwest::redirect::Policy::none())
        .pool_max_idle_per_host(16)
        .build()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

    let metrics = Metrics::new();
    let paused = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let state = Arc::new(ProxyState {
        filter,
        client,
        metrics: metrics.clone(),
        base: base.clone(),
        max_rewrite_bytes: config.max_rewrite_bytes,
        filter_loopback: config.filter_loopback,
        paused: paused.clone(),
    });

    let shutdown = Arc::new(Notify::new());
    let shutdown_signal = shutdown.clone();
    let task_state = state.clone();
    let task = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = shutdown_signal.notified() => {
                    tracing::info!("shiny-filter: proxy shutting down");
                    break;
                }
                accepted = listener.accept() => {
                    let (stream, peer) = match accepted {
                        Ok(pair) => pair,
                        Err(err) => {
                            tracing::warn!("shiny-filter: accept failed: {err}");
                            continue;
                        }
                    };
                    let state = task_state.clone();
                    tokio::spawn(async move {
                        serve_connection(stream, peer, state).await;
                    });
                }
            }
        }
    });

    Ok(ProxyHandle {
        addr,
        base,
        shutdown,
        task,
        filter: state.filter.clone(),
        metrics,
        paused: paused.clone(),
    })
}

/// Serve one client connection: HTTP/1 with upgrade support (for `CONNECT`).
async fn serve_connection(stream: TcpStream, peer: SocketAddr, state: Arc<ProxyState>) {
    let io = TokioIo::new(stream);
    let service = service_fn(move |req: Request<Incoming>| {
        let state = state.clone();
        async move { handle_request(req, state).await }
    });

    let builder = hyper::server::conn::http1::Builder::new();
    if let Err(err) = builder.serve_connection(io, service).with_upgrades().await {
        // Client disconnects are normal (a page navigates away mid-response).
        let msg = err.to_string();
        if !msg.contains("connection closed") && !msg.contains("broken pipe") {
            tracing::debug!("shiny-filter: connection from {peer} ended: {msg}");
        }
    }
}

type BoxBody = http_body_util::combinators::BoxBody<Bytes, std::io::Error>;

fn full(bytes: impl Into<Bytes>) -> BoxBody {
    http_body_util::Full::new(bytes.into())
        .map_err(|never: Infallible| match never {})
        .boxed()
}

fn empty() -> BoxBody {
    full(Bytes::new())
}

async fn handle_request(
    req: Request<Incoming>,
    state: Arc<ProxyState>,
) -> Result<Response<BoxBody>, Infallible> {
    let result = if req.method() == Method::CONNECT {
        handle_connect(req).await
    } else {
        handle_forward(req, &state).await
    };

    Ok(match result {
        Ok(resp) => resp,
        Err(err) => {
            state.metrics.record_failure();
            tracing::debug!("shiny-filter: error: {err}");
            error_response(StatusCode::BAD_GATEWAY, &err)
        }
    })
}

/// A synthetic empty response for a blocked request.
fn blocked_response(kind: &str) -> Response<BoxBody> {
    Response::builder()
        .status(StatusCode::NO_CONTENT)
        .header(header::CONTENT_LENGTH, "0")
        .header(header::CACHE_CONTROL, "no-store")
        .header("x-shiny-blocked", kind)
        .body(empty())
        .expect("static response builds")
}

fn error_response(status: StatusCode, message: &str) -> Response<BoxBody> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-store")
        .body(full(message.to_string()))
        .expect("static response builds")
}

// ── CONNECT ──────────────────────────────────────────────────────────────

/// Tunnel an HTTPS connection.
///
/// Only the `host:port` is visible here, so this is where host-level filter
/// rules apply. Once the tunnel is open the proxy is a dumb pipe — see the
/// `CONNECT` note in the crate docs.
async fn handle_connect(req: Request<Incoming>) -> Result<Response<BoxBody>, String> {
    let authority = req
        .uri()
        .authority()
        .map(|a| a.to_string())
        .ok_or_else(|| "CONNECT without authority".to_string())?;

    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse::<u16>().unwrap_or(443)),
        None => (authority.clone(), 443),
    };

    let on_upgrade = hyper::upgrade::on(req);
    tokio::spawn(async move {
        match on_upgrade.await {
            Ok(upgraded) => {
                let upstream = match TcpStream::connect((host.as_str(), port)).await {
                    Ok(s) => s,
                    Err(err) => {
                        tracing::debug!("shiny-filter: CONNECT {authority} upstream failed: {err}");
                        return;
                    }
                };
                let mut upgraded = TokioIo::new(upgraded);
                let mut upstream = upstream;
                if let Err(err) = copy_bidirectional(&mut upgraded, &mut upstream).await {
                    tracing::debug!("shiny-filter: CONNECT {authority} tunnel ended: {err}");
                }
            }
            Err(err) => tracing::debug!("shiny-filter: CONNECT upgrade failed: {err}"),
        }
    });

    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(empty())
        .expect("static response builds"))
}

// ── Forward proxy ────────────────────────────────────────────────────────

async fn handle_forward(
    req: Request<Incoming>,
    state: &Arc<ProxyState>,
) -> Result<Response<BoxBody>, String> {
    let method = req.method().clone();
    let headers = req.headers().clone();

    let target = resolve_target(req.uri(), &headers)
        .ok_or_else(|| format!("cannot determine target for {}", req.uri()))?;

    // Where the request came from, for `$third-party` and `$domain` matching.
    let source_url = referer_of(&headers).unwrap_or_else(|| target.clone());

    // The proxy's own control endpoints.
    if target.starts_with(&state.base) && target.contains("/__shiny/") {
        return Ok(control_endpoint(&target, state));
    }

    let kind = classify_request(
        &target,
        header_str(&headers, "sec-fetch-dest"),
        header_str(&headers, header::ACCEPT.as_str()),
        header_str(&headers, header::CONTENT_TYPE.as_str()),
        method.as_str(),
    );

    // ── The filter decision (the one engine both surfaces share) ─────────
    //
    // Loopback is exempt: Shiny serves itself from 127.0.0.1, and a filter
    // list must never be able to break the app that hosts this browser.
    let started = Instant::now();
    let filtering_paused = state.paused.load(std::sync::atomic::Ordering::Relaxed);
    let verdict = if filtering_paused || (!state.filter_loopback && is_loopback_url(&target)) {
        Verdict::Allow
    } else {
        match crate::classify::build_adblock_request(&target, &source_url, kind, method.as_str()) {
            Some(adblock_request) => state.filter.check(&adblock_request),
            // Unparseable URL: fail open.
            None => Verdict::Allow,
        }
    };
    state
        .metrics
        .record_filter_time(started.elapsed().as_micros() as u64);

    if verdict.is_blocked() {
        state.metrics.record(kind, true, 0, 0);
        tracing::debug!("shiny-filter: BLOCK [{:?}] {target}", kind);
        return Ok(blocked_response(kind.as_adblock_type()));
    }

    let fetch_url = verdict.effective_url(&target).to_string();
    let body_bytes = read_body(req.into_body()).await?;

    // Build the upstream request.
    let mut upstream = state.client.request(method.clone(), &fetch_url);
    for (name, value) in headers.iter() {
        // Strip hop-by-hop and proxy-specific headers.
        if is_hop_by_hop(name) || is_proxy_header(name) {
            continue;
        }
        // `Host` must NOT be forwarded. A proxy client sends the *proxy's*
        // authority in `Host` (or the absolute URI in the request line, which
        // reqwest parses into `Host: <proxy>`), so copying it upstream
        // misroutes the virtual host. Origins that validate `Host` — DuckDuckGo's
        // nginx, most CDNs — answer 400; laxer origins silently serve the wrong
        // site. reqwest sets the correct `Host` from `fetch_url`; leave it alone.
        if name == header::HOST {
            continue;
        }
        if name == header::ACCEPT_ENCODING {
            // Ask for an identity body so we can rewrite text responses
            // faithfully without a decompress/recompress round trip.
            continue;
        }
        upstream = upstream.header(name, value);
    }
    upstream = upstream.header(header::ACCEPT_ENCODING, "identity");
    if !body_bytes.is_empty() {
        upstream = upstream.body(body_bytes);
    }

    let response = upstream.send().await.map_err(|e| e.to_string())?;
    let status = response.status();
    let resp_headers = response.headers().clone();
    let content_type = resp_headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let is_css = content_type.contains("text/css") || target.ends_with(".css");
    let is_html = content_type.contains("text/html")
        || content_type.contains("application/xhtml")
        || (content_type.is_empty() && kind.is_rewritable());

    let needs_rewrite = (is_html || is_css) && status.is_success();

    let mut body = response.bytes().await.map_err(|e| e.to_string())?;
    let src_len = body.len() as u64;

    if needs_rewrite && body.len() <= state.max_rewrite_bytes {
        let started = Instant::now();
        let text = decode_text(&body, &content_type);
        let (rewritten, cosmetic_count) = if is_html {
            // A `<base href>` would send every relative URL straight to the
            // origin, around the proxy.
            let without_base = strip_base_tag(&text);
            let doc = rewrite_html(&fetch_url, &state.base, &without_base);

            let cosmetic_css = state
                .filter
                .cosmetic_css_for_classes(&fetch_url, &doc.class_id.classes, &doc.class_id.ids);
            let cosmetic_count = cosmetic_css.matches("display:none").count() as u64;
            let block = head_injections(
                &fetch_url,
                &state.base,
                &Injections {
                    cosmetic_css,
                    scriptlets: state.filter.injected_script(&fetch_url),
                    shim: String::new(),
                },
            );
            (insert_after_head(&doc.html, &block), cosmetic_count)
        } else {
            (rewrite_css(&fetch_url, &state.base, &text), 0)
        };

        if cosmetic_count > 0 {
            state.metrics.record_cosmetic_rules(cosmetic_count);
        }
        state
            .metrics
            .record_rewrite(started.elapsed().as_micros() as u64);
        body = Bytes::from(rewritten.into_bytes());
    }

    let out_len = body.len() as u64;
    state.metrics.record(kind, false, src_len, out_len);

    Ok(build_response(status, &resp_headers, body, needs_rewrite, &state.base))
}

/// Copy the upstream response, dropping security headers that would stop the
/// rewritten document from rendering and keeping the rest intact.
fn build_response(
    status: reqwest::StatusCode,
    headers: &HeaderMap,
    body: Bytes,
    rewritten: bool,
    proxy_base: &str,
) -> Response<BoxBody> {
    let mut builder = Response::builder().status(status.as_u16());

    for (name, value) in headers.iter() {
        if is_hop_by_hop(name) || is_proxy_header(name) {
            continue;
        }
        // These four are the ones that would break a rewritten or framed page.
        match name.as_str() {
            "x-frame-options" | "content-security-policy" | "content-security-policy-report-only" => {
                continue
            }
            "content-encoding" => continue, // we hold a decoded body
            "content-length" => continue,    // recomputed below
            "strict-transport-security" if rewritten => continue,
            // A redirect must stay *inside* the proxy. Forwarding the
            // upstream `Location` verbatim sends the browser straight to the
            // origin, so every request after the first hop escapes filtering —
            // and an https target escapes it permanently, because that hop is
            // an encrypted CONNECT the filter cannot inspect.
            "location" => {
                let rewritten_location = value
                    .to_str()
                    .ok()
                    .and_then(|raw| resolve_redirect(raw, proxy_base));
                match rewritten_location {
                    Some(url) => {
                        builder = builder.header(name, url);
                    }
                    None => {
                        // Relative or unparseable: forward as-is rather than
                        // drop it (a dropped Location is a broken redirect).
                        builder = builder.header(name, value);
                    }
                }
                continue;
            }
            _ => {}
        }
        builder = builder.header(name, value);
    }

    builder = builder
        .header(header::CONTENT_LENGTH, body.len().to_string())
        // Documented behaviour for a local filtering proxy: never let a stale
        // page hide a filter-list change.
        .header(header::CACHE_CONTROL, "no-store");

    builder
        .body(full(body))
        .unwrap_or_else(|_| error_response(StatusCode::INTERNAL_SERVER_ERROR, "bad response"))
}

/// The URL this request is really for.
///
/// Handles both shapes: an absolute-form request target from a proxied
/// webview, and the origin-form `/p/<scheme>/<host>/<path>` used when the
/// proxy is itself the origin (the in-app window).
fn resolve_target(uri: &Uri, headers: &HeaderMap) -> Option<String> {
    let raw = uri.to_string();

    // Absolute-URI: a webview configured with an HTTP proxy. Wins outright.
    if raw.starts_with("http://") || raw.starts_with("https://") {
        return Some(raw);
    }

    // Explicit proxy path: only produced by something that already rewrote a
    // document, so it beats an ambiguous `Host`.
    if is_proxied_path(&raw) {
        return decode_target(&raw);
    }

    // Origin-form: this proxy *is* the document's origin, so the upstream is
    // named by `Host`. `<img src="/logo.png">` on a proxied page arrives as
    // `/logo.png` with `Host: example.com`.
    let host = header_str(headers, header::HOST.as_str())?;
    Some(format!("http://{host}{raw}"))
}

fn referer_of(headers: &HeaderMap) -> Option<String> {
    header_str(headers, header::REFERER.as_str()).map(|r| r.to_string())
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

async fn read_body(body: Incoming) -> Result<Bytes, String> {
    body.collect().await.map(|c| c.to_bytes()).map_err(|e| e.to_string())
}

/// Decode a text body, honouring the charset when it is not UTF-8.
fn decode_text(body: &Bytes, content_type: &str) -> String {
    let lower = content_type.to_ascii_lowercase();
    if lower.contains("charset=iso-8859-1") || lower.contains("charset=latin1") {
        body.iter().map(|&b| b as char).collect()
    } else {
        // Lossy is the right default: a document with a few bad bytes must
        // still be rewritten, not dropped.
        String::from_utf8_lossy(body).into_owned()
    }
}

/// Rewrite a redirect target into the proxy's own path form.
///
/// Only absolute `http(s)` targets are rewritten; a relative `Location` is
/// already resolved against the proxied document's origin and therefore
/// already flows through the proxy.
fn resolve_redirect(location: &str, proxy_base: &str) -> Option<String> {
    let absolute = match location.split_once("://") {
        Some(_) => location.to_string(),
        None => return None,
    };
    let proxied = crate::urls::encode_target(&absolute)?;
    Some(format!("{proxy_base}{proxied}"))
}

/// True when a URL points at this machine.
///
/// Shiny's own UI is served from loopback, and it must never be subject to the
/// user's filter lists: doing so would let an EasyList rule break the app or
/// the plugin's own control endpoints.
fn is_loopback_url(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    let Some(host) = parsed.host_str() else {
        return false;
    };
    host.eq_ignore_ascii_case("localhost")
        || host == "127.0.0.1"
        || host == "::1"
        || host == "[::1]"
        || host.ends_with(".localhost")
}

fn is_hop_by_hop(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

fn is_proxy_header(name: &HeaderName) -> bool {
    matches!(name.as_str(), "proxy-connection")
}

/// `/__shiny/metrics` and friends — the surface the benchmarks and the shell
/// status bar read.
fn control_endpoint(target: &str, state: &Arc<ProxyState>) -> Response<BoxBody> {
    let snapshot = state.metrics.snapshot();
    if target.ends_with("/__shiny/metrics") {
        let body = serde_json::to_vec(&snapshot).unwrap_or_default();
        return Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .body(full(body))
            .expect("static response builds");
    }
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(full(snapshot.summary()))
        .expect("static response builds")
}
