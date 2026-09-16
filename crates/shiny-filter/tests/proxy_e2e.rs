//! End-to-end tests for the filtering proxy.
//!
//! These stand up a real origin server, a real proxy, and a real HTTP client,
//! then assert on what a browser would actually observe. Unit tests can show
//! the rewriter is correct; only this can show the pipeline works.
//!
//! **Run with a reduced thread count.** Each test binds both a proxy and an
//! origin, and every request burns a client-side ephemeral port that lingers in
//! `TIME_WAIT`. On macOS the default 16k-port range is exhausted after a couple
//! of full runs, and the failures it produces (`Can't assign requested address`
//! → 502) look like bugs in the proxy. When the suite goes red for no code
//! reason, check `netstat -an | grep -c 127.0.0.1` first:
//!
//! ```text
//! cargo test -p shiny-filter --test proxy_e2e -- --test-threads=2
//! ```

use std::net::SocketAddr;

use axum::http::StatusCode;
use axum::routing::{any, get};
use axum::Router;
use shiny_filter::engine::AdFilter;
use shiny_filter::proxy::{run_proxy, ProxyConfig};

/// A tiny origin with a page, a script, a stylesheet and an "ad" path.
async fn spawn_origin() -> SocketAddr {
    let app = Router::new()
        .route(
            "/",
            get(|| async {
                (
                    [("content-type", "text/html; charset=utf-8")],
                    r##"<!doctype html><html><head><title>t</title></head>
<body class="page">
<img src="/logo.png" alt="">
<script src="https://cdn.example.com/app.js"></script>
<a href="/next?x=1">next</a>
</body></html>"##,
                )
            }),
        )
        .route(
            "/style.css",
            get(|| async {
                (
                    [("content-type", "text/css")],
                    "body{background:url(/bg.png)}",
                )
            }),
        )
        .route(
            "/track/ad.js",
            get(|| async { ([("content-type", "application/javascript")], "track()") }),
        )
        .route(
            "/moved",
            get(|| async {
                (
                    axum::http::StatusCode::MOVED_PERMANENTLY,
                    [("location", "https://www.example.com/landing")],
                    "",
                )
            }),
        )
        .route(
            "/moved-relative",
            get(|| async {
                (
                    axum::http::StatusCode::FOUND,
                    [("location", "/next")],
                    "",
                )
            }),
        )
        .route(
            "/blocked.png",
            get(|| async {
                (
                    [("content-type", "image/png"), ("x-frame-options", "DENY")],
                    "PNGDATA",
                )
            }),
        )
        .route(
            "/framed",
            get(|| async {
                (
                    [
                        ("content-type", "text/html"),
                        ("x-frame-options", "DENY"),
                        ("content-security-policy", "frame-ancestors 'none'"),
                    ],
                    "<html><body>framed</body></html>",
                )
            }),
        )
        // Documents served with a generic type, or none. Common on misconfigured
        // origins and extensionless CMS routes, and a browser refuses to run the
        // injected shim inside an `application/octet-stream` response.
        .route(
            "/no-type",
            get(|| async {
                (
                    [("content-type", "application/octet-stream")],
                    "<html><body>untyped</body></html>",
                )
            }),
        )
        .route(
            "/really-no-type",
            get(|| async { "<html><body>bare</body></html>" }),
        );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    addr
}

/// A filter that blocks scripts and images from this origin.
///
/// Note the rule shape: `||` anchors on the **hostname** only, so a rule may
/// not contain `host:port/path`. The `$script,image` options narrow it, and a
/// separate path rule can target one URL. Both shapes are exercised below.
/// A proxy config that *does* filter loopback, so the fixture origin can stand
/// in for a real website. The shipped default exempts loopback so filter lists
/// can never break Shiny itself.
fn filtering_config() -> ProxyConfig {
    ProxyConfig {
        filter_loopback: true,
        ..ProxyConfig::default()
    }
}

fn ad_filter() -> AdFilter {
    AdFilter::from_lists(["||127.0.0.1^$script,image", "/track/ad.js"], false)
}

/// A client that speaks to the proxy in absolute-URI (webview) form.
fn proxied_client(proxy: SocketAddr) -> reqwest::Client {
    reqwest::Client::builder()
        .proxy(reqwest::Proxy::http(format!("http://{proxy}")).unwrap())
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
}

#[tokio::test]
async fn blocks_a_filtered_subresource() {
    let origin = spawn_origin().await;
    // Block exactly the ad script, allow everything else.
    let filter = ad_filter();
    let proxy = run_proxy(filtering_config(), filter).await.unwrap();
    let client = proxied_client(proxy.addr());

    let res = client
        .get(format!("http://{origin}/track/ad.js"))
        .send()
        .await
        .unwrap();

    assert_eq!(
        res.status(),
        StatusCode::NO_CONTENT,
        "a blocked request must come back empty, not fail"
    );
    assert_eq!(res.headers().get("x-shiny-blocked").unwrap(), "script");

    let snapshot = proxy.metrics().snapshot();
    assert_eq!(snapshot.blocked, 1);
    assert_eq!(snapshot.requests, 1);
    proxy.shutdown();
}

/// The proxy must send the *target's* authority in `Host`, never the proxy's.
///
/// This was a real bug: the client's `Host` (the proxy address) was copied
/// upstream, so DuckDuckGo's nginx answered 400 and laxer origins served the
/// wrong virtual host. Asserting it needs an origin that echoes `Host` back.
#[tokio::test]
async fn forwards_the_target_host_not_the_proxys() {
    let app = Router::new().fallback(any(|req: axum::extract::Request| async move {
        let host = req
            .headers()
            .get("host")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("<missing>")
            .to_string();
        ([( "x-seen-host", host.clone())], host)
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let proxy = run_proxy(ProxyConfig::default(), AdFilter::empty())
        .await
        .unwrap();

    // Origin-form, exactly as the in-app window requests a page.
    let res = reqwest::get(format!("{}/p/http/{origin}/probe", proxy.base()))
        .await
        .unwrap();
    let seen = res
        .headers()
        .get("x-seen-host")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    assert_eq!(seen, origin.to_string(), "upstream Host was wrong: {seen}");
    assert!(
        !seen.contains(&proxy.addr().port().to_string()) || proxy.addr().port() == origin.port(),
        "the proxy's own authority leaked upstream: {seen}"
    );

    proxy.shutdown();
}

#[tokio::test]
async fn allows_an_unfiltered_subresource_and_passes_headers() {
    let origin = spawn_origin().await;
    let filter = AdFilter::empty();
    let proxy = run_proxy(ProxyConfig::default(), filter).await.unwrap();
    let client = proxied_client(proxy.addr());

    let res = client
        .get(format!("http://{origin}/style.css"))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    let body = res.text().await.unwrap();
    assert!(body.contains("/p/http/127.0.0.1"), "css not rewritten: {body}");
    assert!(body.contains("/bg.png"), "css path lost: {body}");

    proxy.shutdown();
}

#[tokio::test]
async fn rewrites_a_document_and_injects_the_shim() {
    let origin = spawn_origin().await;
    let filter = AdFilter::empty();
    let proxy = run_proxy(ProxyConfig::default(), filter).await.unwrap();
    let client = proxied_client(proxy.addr());

    let res = client.get(format!("http://{origin}/")).send().await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let html = res.text().await.unwrap();

    // Every subresource now points at the proxy.
    assert!(
        html.contains(&format!("/p/http/{origin}/logo.png")),
        "img not rewritten: {html}"
    );
    assert!(
        html.contains("/p/https/cdn.example.com/app.js"),
        "cross-origin script not rewritten: {html}"
    );
    // The document-start shim is present and bound to the right proxy.
    assert!(html.contains("data-shiny-filter=\"shim\""), "no shim: {html}");
    assert!(
        html.contains(proxy.base()),
        "shim not bound to the proxy base: {html}"
    );

    proxy.shutdown();
}

#[tokio::test]
async fn injects_cosmetic_css_for_the_document() {
    let origin = spawn_origin().await;
    let filter = AdFilter::from_lists(["127.0.0.1##.page"], false);
    let proxy = run_proxy(filtering_config(), filter).await.unwrap();
    let client = proxied_client(proxy.addr());

    let html = client
        .get(format!("http://{origin}/"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();

    assert!(
        html.contains("data-shiny-filter=\"cosmetic\""),
        "no cosmetic block: {html}"
    );
    assert!(html.contains(".page"), "cosmetic selector missing: {html}");
    assert!(html.contains("display:none"), "cosmetic css incomplete: {html}");

    assert!(proxy.metrics().snapshot().cosmetic_rules > 0);
    proxy.shutdown();
}

#[tokio::test]
async fn strips_headers_that_would_block_framing() {
    let origin = spawn_origin().await;
    let filter = AdFilter::empty();
    let proxy = run_proxy(ProxyConfig::default(), filter).await.unwrap();
    let client = proxied_client(proxy.addr());

    let res = client.get(format!("http://{origin}/framed")).send().await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(
        res.headers().get("x-frame-options").is_none(),
        "X-Frame-Options must be stripped so the in-app window can frame the page"
    );
    assert!(
        res.headers().get("content-security-policy").is_none(),
        "CSP frame-ancestors must be stripped"
    );

    proxy.shutdown();
}

#[tokio::test]
async fn origin_form_requests_work_for_the_plugin_iframe() {
    let origin = spawn_origin().await;
    let filter = AdFilter::empty();
    let proxy = run_proxy(ProxyConfig::default(), filter).await.unwrap();

    // This is what the in-app iframe does: the proxy *is* the origin.
    let res = reqwest::get(format!("{}/p/http/{origin}/", proxy.base()))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let html = res.text().await.unwrap();
    assert!(html.contains("<!doctype html>") || html.contains("<!DOCTYPE html>"));
    assert!(html.contains("/p/http/"), "subresources not proxied: {html}");

    proxy.shutdown();
}

#[tokio::test]
async fn metrics_endpoint_reports_the_run() {
    let origin = spawn_origin().await;
    let filter = ad_filter();
    let proxy = run_proxy(filtering_config(), filter).await.unwrap();
    let client = proxied_client(proxy.addr());

    client
        .get(format!("http://{origin}/track/ad.js"))
        .send()
        .await
        .unwrap();
    client
        .get(format!("http://{origin}/style.css"))
        .send()
        .await
        .unwrap();

    let raw = reqwest::get(format!("{}/__shiny/metrics", proxy.base()))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    let snapshot: serde_json::Value = serde_json::from_str(&raw).unwrap();

    assert_eq!(snapshot["requests"], 2);
    assert_eq!(snapshot["blocked"], 1);
    assert_eq!(snapshot["blocked_ratio"], 0.5);
    assert_eq!(snapshot["by_kind"]["scripts"], 1);

    proxy.shutdown();
}

#[tokio::test]
async fn unparseable_target_fails_open() {
    let filter = AdFilter::from_lists(["||example.com^"], false);
    let proxy = run_proxy(ProxyConfig::default(), filter).await.unwrap();

    // A request with no resolvable host must not be blocked or panic.
    let res = reqwest::get(format!("{}/not-a-proxied-path", proxy.base()))
        .await
        .unwrap();
    // It will fail to find an upstream (there is no Host match) — what matters
    // is that it is an error response, not a block, and that the proxy lives.
    assert!(res.status().is_client_error() || res.status().is_server_error());
    assert_eq!(proxy.metrics().snapshot().blocked, 0);

    proxy.shutdown();
}

#[tokio::test]
async fn redirects_stay_inside_the_proxy() {
    let origin = spawn_origin().await;
    let proxy = run_proxy(ProxyConfig::default(), AdFilter::empty())
        .await
        .unwrap();
    let client = proxied_client(proxy.addr());

    let res = client
        .get(format!("http://{origin}/moved"))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::MOVED_PERMANENTLY);
    let location = res.headers().get("location").unwrap().to_str().unwrap();
    // The whole point: a verbatim upstream Location would send the browser
    // straight to the origin, unfiltered and (for https) uninspectable.
    assert!(
        location.starts_with(proxy.base()),
        "Location escaped the proxy: {location}"
    );
    assert!(
        location.contains("/p/https/www.example.com/landing"),
        "Location not proxied correctly: {location}"
    );

    proxy.shutdown();
}

#[tokio::test]
async fn relative_redirects_are_left_alone() {
    let origin = spawn_origin().await;
    let proxy = run_proxy(ProxyConfig::default(), AdFilter::empty())
        .await
        .unwrap();
    let client = proxied_client(proxy.addr());

    let res = client
        .get(format!("http://{origin}/moved-relative"))
        .send()
        .await
        .unwrap();
    // A relative Location already resolves against the proxied origin, so
    // rewriting it would be wrong.
    assert_eq!(
        res.headers().get("location").unwrap().to_str().unwrap(),
        "/next"
    );

    proxy.shutdown();
}

#[tokio::test]
async fn loopback_is_never_filtered() {
    let origin = spawn_origin().await;
    // A filter that would block this very origin if it were applied.
    let filter = AdFilter::from_lists(["||127.0.0.1^"], false);
    let proxy = run_proxy(ProxyConfig::default(), filter).await.unwrap();
    let client = proxied_client(proxy.addr());

    let res = client
        .get(format!("http://{origin}/style.css"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        StatusCode::OK,
        "the app's own loopback origin must never be filtered"
    );

    proxy.shutdown();
}

#[tokio::test]
async fn pausing_stops_blocking_and_resuming_restores_it() {
    let origin = spawn_origin().await;
    let proxy = run_proxy(filtering_config(), ad_filter()).await.unwrap();
    let client = proxied_client(proxy.addr());

    // Baseline: the ad script is blocked.
    let blocked = client
        .get(format!("http://{origin}/track/ad.js"))
        .send()
        .await
        .unwrap();
    assert_eq!(blocked.status(), StatusCode::NO_CONTENT);
    assert_eq!(proxy.metrics().snapshot().blocked, 1);
    assert!(!proxy.is_paused());

    // Paused: the same request must now reach the origin, because some sites
    // genuinely cannot work with their trackers removed.
    proxy.set_paused(true);
    assert!(proxy.is_paused());
    let allowed = client
        .get(format!("http://{origin}/track/ad.js"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        allowed.status(),
        StatusCode::OK,
        "a paused filter must not block anything"
    );
    // The block counter must not move while paused.
    assert_eq!(proxy.metrics().snapshot().blocked, 1);

    // Resumed: blocking returns.
    proxy.set_paused(false);
    let blocked_again = client
        .get(format!("http://{origin}/track/ad.js"))
        .send()
        .await
        .unwrap();
    assert_eq!(blocked_again.status(), StatusCode::NO_CONTENT);
    assert_eq!(proxy.metrics().snapshot().blocked, 2);

    proxy.shutdown();
}

#[tokio::test]
async fn a_rewritten_document_is_always_declared_as_html() {
    // The injected shim is JavaScript. A browser only executes (and, in strict
    // MIME mode, only renders) it when the response is declared as HTML, so a
    // document that arrived with a generic or missing `Content-Type` must come
    // back with one — otherwise filtering silently stops working on exactly the
    // sites most likely to need it.
    let origin = spawn_origin().await;
    let proxy = run_proxy(filtering_config(), ad_filter()).await.unwrap();
    let client = proxied_client(proxy.addr());

    for path in ["/no-type", "/really-no-type"] {
        let res = client
            .get(format!("http://{origin}{path}"))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK, "{path}");
        let declared = res
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();
        assert!(
            declared.contains("text/html"),
            "{path} was declared as {declared:?}"
        );
        let body = res.text().await.unwrap();
        assert!(body.contains("data-shiny-filter=\"shim\""), "{path}: shim missing");
    }

    // A correct `Content-Type` is passed through untouched, charset included.
    let res = client
        .get(format!("http://{origin}/"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        res.headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or(""),
        "text/html; charset=utf-8"
    );

    proxy.shutdown();
}
