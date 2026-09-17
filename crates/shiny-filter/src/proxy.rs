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

use axum::http::{
    header, HeaderMap, HeaderName, HeaderValue, Method, Request, Response, StatusCode, Uri,
};
use bytes::Bytes;
use http_body_util::BodyExt;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use tokio::io::copy_bidirectional;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;

use wreq_util::emulate::{Emulation, Profile};

use crate::classify::classify_request;
use crate::engine::{AdFilter, Verdict};
use crate::inject::{head_injections, insert_after_head, Injections};
use crate::metrics::Metrics;
use crate::rewrite::{rewrite_css, rewrite_html, strip_base_tag};
use crate::urls::{decode_target, is_proxied_path};

/// The browser the proxy impersonates, and the User-Agent that must match it.
///
/// These are pinned together on purpose. `wreq` emulates the TLS (JA3/JA4)
/// and HTTP/2 fingerprint of the browser named here; if the outgoing
/// `User-Agent` were taken from the running platform webview instead, the
/// request would claim Safari while presenting Chrome's fingerprint, and that
/// mismatch is itself a bot signal. Keep the two in step.
pub const EMULATION: Profile = Emulation::Chrome136;

/// User-Agent sent when the client does not provide one.
///
/// Matches [`EMULATION`] (Chrome 136), so the header and the TLS fingerprint
/// always agree. See the note above.
pub const BROWSER_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) \
AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36";

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
    /// Cookie names that belong to the app hosting the browser and must never
    /// be forwarded upstream.
    ///
    /// The app and this proxy share the host `127.0.0.1` (cookies ignore the
    /// port), so the browser sends the app's own session cookie on proxied
    /// requests too. Forwarding it would hand the app's credential to every
    /// site the user browses. Site cookies are stored under `/p/<scheme>/<host>`
    /// (see `rewrite_set_cookie`), but the app's cookie is `Path=/`, so it
    /// cannot be excluded by path — only by name.
    pub app_cookie_names: Vec<String>,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            bind: SocketAddr::from(([127, 0, 0, 1], 0)),
            public_base: None,
            max_rewrite_bytes: 12 * 1024 * 1024,
            filter_loopback: false,
            app_cookie_names: vec!["shiny_token".to_string()],
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
    client: wreq::Client,
    metrics: Metrics,
    base: String,
    max_rewrite_bytes: usize,
    /// See [`ProxyConfig::filter_loopback`].
    filter_loopback: bool,
    /// See [`ProxyConfig::app_cookie_names`].
    app_cookie_names: Vec<String>,
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

/// The upstream client every browser surface shares.
///
/// It presents the impersonated browser's TLS/HTTP2 fingerprint *and* the
/// matching `User-Agent`, and never follows redirects on its own (the proxy
/// rewrites `Location` itself so hops stay inside the filter).
///
/// Exposed so the in-app plugin's own fetches (`browser_read`, the news shelf)
/// use the same identity as the proxy instead of a library UA that sites treat
/// as a bot.
pub fn impersonated_client_builder() -> wreq::ClientBuilder {
    wreq::Client::builder()
        // Present a real Chrome fingerprint upstream. Without this the client's
        // own TLS stack is a bot signal many sites block outright, and no
        // header can hide it (see the module note on `EMULATION`).
        .emulation(EMULATION)
        // The client's own `User-Agent` is forwarded when it sends one. This is
        // the fallback for clients that send none — an HTML fetch made by a
        // tool, for instance — because a bare library UA is refused or served
        // degraded content by a lot of the web.
        .user_agent(BROWSER_USER_AGENT)
        .redirect(wreq::redirect::Policy::none())
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

    let client = impersonated_client_builder()
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
        app_cookie_names: config.app_cookie_names,
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

    // Where the request came from, for `$third-party` and `$domain` matching —
    // and for the same headers upstream, see below. The client sends the
    // *proxy* URL in `Referer`/`Origin` (that is the document's address in its
    // world), so both are translated back to the real site before anything
    // looks at them. Un-translated, every origin saw our loopback URL and every
    // third-party rule compared the site against itself, so nothing on a page
    // was ever third-party.
    let referer_url = referer_of(&headers);
    let source_url = referer_url.clone().unwrap_or_else(|| target.clone());

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
        // the client parses into `Host: <proxy>`), so copying it upstream
        // misroutes the virtual host. Origins that validate `Host` — DuckDuckGo's
        // nginx, most CDNs — answer 400; laxer origins silently serve the wrong
        // site. The client sets the correct `Host` from `fetch_url`; leave it alone.
        if name == header::HOST {
            continue;
        }
        // `Accept-Encoding` must NOT be forwarded either: the impersonating
        // client sets the one that matches the emulated browser (gzip/br/zstd)
        // and decodes the response before it reaches us. Forwarding the
        // webview's value and then also decoding would double-handle the body.
        if name == header::ACCEPT_ENCODING {
            continue;
        }
        // `User-Agent` is pinned to the impersonated browser, never taken from
        // the client. A webview UA (or `Peakd/…`) over Chrome's TLS fingerprint
        // is a mismatch that is itself a bot signal — the two must agree.
        if name == header::USER_AGENT {
            continue;
        }
        // Client hints are part of the same identity. The webview's hints
        // (Safari's, or another Chrome build's) would contradict the Chrome UA
        // and TLS we present, so the emulation profile's own hints are used.
        if matches!(
            name.as_str(),
            "sec-ch-ua"
                | "sec-ch-ua-mobile"
                | "sec-ch-ua-platform"
                | "sec-ch-ua-full-version"
                | "sec-ch-ua-full-version-list"
                | "sec-ch-ua-platform-version"
                | "sec-ch-ua-arch"
                | "sec-ch-ua-bitness"
                | "sec-ch-ua-model"
        ) {
            continue;
        }
        // The app and the proxy share `127.0.0.1`, and cookies ignore the port,
        // so the browser sends the app's own session cookie here too. Forwarding
        // it would hand the app's credential to every site; drop those names.
        // Site cookies survive (they are the ones this proxy needs to pass on
        // for sessions and bot challenges).
        if name == header::COOKIE {
            if let Some(filtered) =
                filter_cookie_header(value, &state.app_cookie_names)
            {
                upstream = upstream.header(name, filtered);
            }
            continue;
        }
        // Provenance headers describe the *proxied* document. Forwarding them
        // verbatim tells the origin the request came from `127.0.0.1:<port>`,
        // which breaks Referer-based hotlink protection, `Origin`-checked CORS
        // and CSRF checks — and leaks the proxy's address to every site.
        if name == header::REFERER {
            if let Some(real) = &referer_url {
                if let Ok(translated) = HeaderValue::from_str(real) {
                    upstream = upstream.header(name, translated);
                }
            }
            continue;
        }
        if name == header::ORIGIN {
            if let Some(real) = &referer_url {
                // An origin is a scheme://authority triple, not a full URL.
                if let Some(origin) = origin_of(real) {
                    if let Ok(translated) = HeaderValue::from_str(&origin) {
                        upstream = upstream.header(name, translated);
                    }
                }
            }
            continue;
        }
        upstream = upstream.header(name, value);
    }
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

    let mut body = response.bytes().await.map_err(|e| e.to_string())?;
    let src_len = body.len() as u64;

    // Classify *after* the body is in hand, because the declared type is not
    // always enough: an extensionless CMS route or a misconfigured origin
    // serves a real document as `application/octet-stream` or with no type at
    // all. Treated as "not a document", such a page would pass through
    // unfiltered *and* be refused rendering by a strict-MIME browser. Sniffing
    // the bytes is what makes those pages work at all.
    let declared = mime_of(&content_type);
    let generic_type = is_generic_type(&declared);
    let is_html = declared == "text/html"
        || declared == "application/xhtml+xml"
        || (generic_type && looks_like_html(&body));
    let is_css = declared == "text/css" || target.ends_with(".css");

    // Rewrite *every* document, including error pages. A 404 page is still a
    // page: its `href="/"` has to mean the site's home, and if it is left alone
    // the browser resolves it against the proxy's own origin — sending the user
    // to the app instead of the site, out of the filter, on the first page that
    // does not exist. The old `status.is_success()` gate did exactly that.
    //
    // A bodyless status (204/304) carries nothing to rewrite, so it is skipped
    // structurally rather than by status.
    let has_body = !matches!(status.as_u16(), 204 | 304) && !body.is_empty();
    let needs_rewrite = (is_html || is_css) && has_body;

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

    Ok(build_response(
        status,
        &resp_headers,
        body,
        needs_rewrite,
        is_html,
        &state.base,
        &fetch_url,
    ))
}

/// Copy the upstream response, dropping security headers that would stop the
/// rewritten document from rendering and keeping the rest intact.
fn build_response(
    status: StatusCode,
    headers: &HeaderMap,
    body: Bytes,
    rewritten: bool,
    rewritten_html: bool,
    proxy_base: &str,
    fetch_url: &str,
) -> Response<BoxBody> {
    let mut builder = Response::builder().status(status.as_u16());
    // Whether the upstream actually sent a `Content-Type` we forwarded.
    let mut sent_content_type = false;

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
            // A response is classified as a document from its declared type,
            // its extension, or `Sec-Fetch-Dest`. When the first two are absent
            // the forwarded `Content-Type` is empty or a generic
            // `application/octet-stream`, and a browser will refuse to execute
            // the injected shim in it — and, in strict-MIME mode, may refuse to
            // render the page at all. Re-declare it as HTML: the body we are
            // returning genuinely *is* the rewritten HTML document.
            "content-type" => {
                let lower = value.to_str().unwrap_or("").to_ascii_lowercase();
                let is_declared_html = lower.contains("html") || lower.contains("xhtml");
                if rewritten_html && !is_declared_html {
                    builder = builder.header(name, "text/html; charset=utf-8");
                } else {
                    builder = builder.header(name, value);
                }
                sent_content_type = true;
                continue;
            }
            // Cookies must be re-scoped to the proxy origin or the browser
            // drops them: the document's address is `http://127.0.0.1:<port>`,
            // so a cookie marked `Domain=.example.com` or `Secure` is refused
            // outright. Without this, every session cookie — including
            // Cloudflare's `__cf_bm` / `cf_clearance`, which a JS challenge
            // depends on — is lost and the challenge loops forever. See
            // [`rewrite_set_cookie`].
            "set-cookie" => {
                match value.to_str() {
                    Ok(raw) => builder = builder.header(name, rewrite_set_cookie(raw, fetch_url)),
                    Err(_) => builder = builder.header(name, value),
                }
                continue;
            }
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

    // A rewritten document with no `Content-Type` at all still needs one, for
    // the same MIME reason as above.
    if rewritten_html && !sent_content_type {
        builder = builder.header(header::CONTENT_TYPE, "text/html; charset=utf-8");
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
    // Empty and unparseable values are dropped rather than forwarded.
    header_str(headers, header::REFERER.as_str())
        .filter(|raw| !raw.trim().is_empty())
        .map(|raw| decode_proxy_url(raw.trim()))
}

/// Turn a URL the client used, which may be a proxy path, back into the real
/// one: `http://proxy/p/https/example.com/a` → `https://example.com/a`.
///
/// Returns the input unchanged when it is already a real URL (or is not one at
/// all), so this is safe to apply to anything.
fn decode_proxy_url(url: &str) -> String {
    if let Some(rest) = url.split_once("/p/").map(|(_, rest)| rest) {
        if let Some((scheme, remainder)) = rest.split_once('/') {
            if scheme == "http" || scheme == "https" {
                return format!("{scheme}://{remainder}");
            }
        }
    }
    url.to_string()
}

/// The `scheme://authority` of a URL, for an `Origin` header.
fn origin_of(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    Some(match parsed.port() {
        Some(port) => format!("{}://{host}:{port}", parsed.scheme()),
        None => format!("{}://{host}", parsed.scheme()),
    })
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

/// A `Content-Type` that names nothing: missing, generic, or `text/plain`.
///
/// `text/plain` counts as "no document type" on purpose. A framework that
/// returns a bare string without setting a type emits `text/plain; charset=utf-8`,
/// and that is exactly the shape of an untyped HTML route. Only bodies that
/// sniff as markup are promoted, so a real plain-text response is untouched.
fn is_generic_type(declared: &str) -> bool {
    matches!(
        declared,
        "" | "application/octet-stream" | "text/plain" | "binary/octet-stream"
    )
}

/// The MIME type of a `Content-Type` header, lowercased and without parameters.
///
/// `substring` checks on the raw header (what this used to do) mis-read
/// `text/html-ish` and cannot see through `; charset=…`.
fn mime_of(content_type: &str) -> String {
    content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
}

/// Whether a body with no useful declared type looks like an HTML document.
///
/// Deliberately strict and cheap: skip leading whitespace and BOM, then look
/// for a doctype or an `<html` tag in the first bytes. A document that starts
/// with comments or a `<head>` still gets caught by the `<html` check in its
/// first kilobyte, and a video or a JSON body never matches.
fn looks_like_html(body: &Bytes) -> bool {
    let window = &body[..body.len().min(1024)];
    let text = String::from_utf8_lossy(window);
    let trimmed = text.trim_start_matches(['\u{feff}', ' ', '\t', '\r', '\n']);
    let lower = trimmed.to_ascii_lowercase();
    lower.starts_with("<!doctype html")
        || lower.starts_with("<html")
        || lower.contains("<html")
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

/// Re-scope a `Set-Cookie` from the origin onto the proxy path.
///
/// The window's origin is the proxy (`http://127.0.0.1:<port>`), so the cookie
/// attributes an origin sends cannot be honoured as-is:
///
/// * `Domain=…` never matches that origin, so it is dropped (the cookie becomes
///   host-only for the proxy).
/// * `Secure` can never be satisfied over plain `http`, so it is dropped.
/// * `SameSite=None` is only legal with `Secure`; the proxy is a single origin,
///   so `Lax` is both valid and permissive enough.
/// * `Partitioned` needs a partitioned (top-level, third-party) context the
///   proxy does not have, so it is dropped.
/// * `Path` is rewritten to sit under the site's proxy prefix
///   (`/p/<scheme>/<host>`). That is what keeps one site's cookies from
///   reaching another: the browser scopes them by path, so a `Path=/` cookie
///   from `evil.com` is stored under `/p/https/evil.com/` and is never sent to
///   `bank.com`. An origin cannot widen it either, because the result is always
///   prefixed with the site the cookie came from.
///
/// Getting this wrong is not cosmetic: without it every session cookie is
/// dropped, and Cloudflare's `__cf_bm` / `cf_clearance` (which its JS challenge
/// depends on) never sticks, so the challenge repeats forever.
fn rewrite_set_cookie(raw: &str, fetch_url: &str) -> String {
    let raw = raw.trim();
    if raw.is_empty() {
        return String::new();
    }
    let Some(prefix) = site_prefix_of(fetch_url) else {
        return raw.to_string();
    };

    let mut out = String::with_capacity(raw.len() + prefix.len() + 8);
    let mut had_path = false;
    for (i, attr) in raw.split(';').map(str::trim).filter(|a| !a.is_empty()).enumerate() {
        if i == 0 {
            out.push_str(attr); // Name=Value
            continue;
        }
        let (name, value) = match attr.split_once('=') {
            Some((n, v)) => (n.trim(), v.trim()),
            None => (attr, ""),
        };
        match name.to_ascii_lowercase().as_str() {
            // Host-only for the proxy origin; `Secure`/`Partitioned` cannot be
            // satisfied over plain http from a single origin.
            "domain" | "secure" | "partitioned" => {}
            "path" => {
                had_path = true;
                out.push_str("; Path=");
                out.push_str(&scope_cookie_path(&prefix, value));
            }
            "samesite" => {
                let value = if value.eq_ignore_ascii_case("none") {
                    "Lax"
                } else {
                    value
                };
                out.push_str("; SameSite=");
                out.push_str(value);
            }
            _ => {
                out.push_str("; ");
                out.push_str(attr);
            }
        }
    }

    // Even a cookie with no `Path` has to be scoped: the browser's *default*
    // path is the directory of the request URL, which for a bare
    // `/p/http/host` is `/p/http` — shared by every plain-http site.
    if !had_path {
        out.push_str("; Path=");
        out.push_str(&prefix);
        out.push('/');
    }
    out
}

/// Drop the app's own cookies from a `Cookie` header before it goes upstream.
///
/// Returns the surviving pairs, or `None` when nothing is left (so no empty
/// `Cookie:` header is sent).
fn filter_cookie_header(value: &HeaderValue, deny: &[String]) -> Option<String> {
    let raw = value.to_str().ok()?;
    let kept: Vec<&str> = raw
        .split(';')
        .map(str::trim)
        .filter(|pair| !pair.is_empty())
        .filter(|pair| {
            let name = pair.split('=').next().unwrap_or("").trim();
            !deny.iter().any(|d| d == name)
        })
        .collect();
    if kept.is_empty() {
        None
    } else {
        Some(kept.join("; "))
    }
}

/// `/p/<scheme>/<authority>` for the document a cookie came from.
fn site_prefix_of(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    let authority = match parsed.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    };
    Some(format!("/p/{}/{authority}", parsed.scheme()))
}

/// Put a server-supplied cookie path under the site prefix, dropping `.`/`..`
/// so it cannot climb out of it.
fn scope_cookie_path(prefix: &str, raw: &str) -> String {
    let mut cleaned = String::new();
    for seg in raw.split('/') {
        if seg.is_empty() || seg == "." || seg == ".." {
            continue;
        }
        cleaned.push('/');
        cleaned.push_str(seg);
    }
    if cleaned.is_empty() {
        format!("{prefix}/")
    } else {
        format!("{prefix}{cleaned}")
    }
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

#[cfg(test)]
mod cookie_tests {
    use super::*;

    #[test]
    fn strips_domain_and_secure_and_scopes_the_path() {
        let out = rewrite_set_cookie(
            "sid=abc123; Domain=.example.com; Path=/; Secure; HttpOnly",
            "https://example.com/account",
        );
        assert_eq!(out, "sid=abc123; Path=/p/https/example.com/; HttpOnly");
    }

    #[test]
    fn a_missing_path_is_still_scoped() {
        // The browser's default path for `/p/http/host` is `/p/http`, which is
        // shared by every plain-http site, so a scoped Path must be added.
        let out = rewrite_set_cookie("a=b; HttpOnly", "http://127.0.0.1:8080/x");
        assert_eq!(out, "a=b; HttpOnly; Path=/p/http/127.0.0.1:8080/");
    }

    #[test]
    fn samesite_none_becomes_lax() {
        let out = rewrite_set_cookie(
            "__cf_bm=x; Path=/; SameSite=None; Secure",
            "https://site.example/",
        );
        assert_eq!(out, "__cf_bm=x; Path=/p/https/site.example/; SameSite=Lax");
    }

    #[test]
    fn a_server_cannot_escape_its_own_site() {
        // A malicious origin must not be able to plant a cookie that the
        // browser would send to another site.
        let out = rewrite_set_cookie(
            "x=1; Path=/p/https/bank.example/",
            "https://evil.example/",
        );
        assert_eq!(
            out,
            "x=1; Path=/p/https/evil.example/p/https/bank.example"
        );
        assert!(out.starts_with("x=1; Path=/p/https/evil.example/"));
    }

    #[test]
    fn traversal_segments_are_dropped() {
        let out = rewrite_set_cookie("x=1; Path=/../../", "https://a.example/");
        assert_eq!(out, "x=1; Path=/p/https/a.example/");
    }

    #[test]
    fn other_attributes_survive() {
        let out = rewrite_set_cookie(
            "t=v; Path=/app; Max-Age=3600; Expires=Wed, 21 Oct 2026 07:28:00 GMT; SameSite=Strict",
            "https://a.example/app",
        );
        assert!(out.contains("Path=/p/https/a.example/app"));
        assert!(out.contains("Max-Age=3600"));
        assert!(out.contains("Expires=Wed, 21 Oct 2026 07:28:00 GMT"));
        assert!(out.contains("SameSite=Strict"));
    }

    #[test]
    fn port_is_part_of_the_site_prefix() {
        assert_eq!(
            site_prefix_of("http://localhost:9000/a").as_deref(),
            Some("/p/http/localhost:9000")
        );
    }

    #[test]
    fn the_app_cookie_is_not_forwarded_but_site_cookies_are() {
        let deny = vec!["shiny_token".to_string()];
        let kept = filter_cookie_header(
            &HeaderValue::from_static("shiny_token=secret; cf_clearance=abc; session=1"),
            &deny,
        );
        assert_eq!(kept.as_deref(), Some("cf_clearance=abc; session=1"));
    }

    #[test]
    fn a_lone_app_cookie_leaves_no_header() {
        let deny = vec!["shiny_token".to_string()];
        assert_eq!(
            filter_cookie_header(&HeaderValue::from_static("shiny_token=secret"), &deny),
            None
        );
    }
}

