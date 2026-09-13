//! REST routes for the browser window.
//!
//! The window is a client of the plugin, not a peer of it: every action the
//! user takes (go, back, reload, search) round-trips through one of these so
//! the session, the history log and the metrics stay consistent, and so the
//! proxy base URL never has to be guessed by the front end.

use std::sync::Arc;

use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};

use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::routes::{bridged_route, RouteHandler, user_id_from_request};
use shiny_plugin_sdk::services::PluginCtx;

use crate::proxy;

pub fn handle(ctx: &Arc<PluginCtx>, tag: &str) -> Option<RouteHandler> {
    let ctx = ctx.clone();
    match tag {
        "peakd_state" => Some(state(ctx)),
        "peakd_sessions" => Some(sessions(ctx)),
        "peakd_session_create" => Some(session_create(ctx)),
        "peakd_session_close" => Some(session_close(ctx)),
        "peakd_navigate" => Some(navigate(ctx)),
        "peakd_metrics" => Some(metrics(ctx)),
        "peakd_filter_toggle" => Some(filter_toggle(ctx)),
        _ => None,
    }
}

fn user_id(req: &axum::extract::Request) -> Result<String, AppError> {
    user_id_from_request(req).ok_or_else(|| AppError::Unauthorized("not authenticated".into()))
}

fn ok(data: Value) -> Response {
    axum::Json(json!({ "success": true, "data": data })).into_response()
}


/// Decode a JSON request body.
///
/// `Json` implements `FromRequest`, not `FromRequestParts`, so it has to
/// consume the whole request — which is fine here: every route that takes a
/// JSON body uses nothing else from it.
async fn take_json<T: DeserializeOwned>(
    req: axum::extract::Request,
) -> Result<T, AppError> {
    let (_parts, body) = req.into_parts();
    let bytes = axum::body::to_bytes(body, 1024 * 1024)
        .await
        .map_err(|e| AppError::BadRequest(format!("could not read body: {e}")))?;
    serde_json::from_slice(&bytes)
        .map_err(|e| AppError::BadRequest(format!("invalid body: {e}")))
}

/// Normalize whatever the address bar gave us into an absolute http(s) URL.
///
/// A bare word is a search, not a hostname: that is the behaviour every
/// address bar has, and guessing a TLD would be worse than searching.
pub fn normalize_input(raw: &str) -> Target {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Target::Search(String::new());
    }

    if let Ok(url) = url::Url::parse(trimmed) {
        if matches!(url.scheme(), "http" | "https") {
            return Target::Url(trimmed.to_string());
        }
    }

    // Note: `127.0.0.1:8080/x` parses "successfully" with scheme `127.0.0.1`
    // — `Url::parse` validates scheme *syntax*, not that the scheme is one a
    // browser can load. Nothing to do here: the http(s) case returned above,
    // so control falls through to the host heuristic below, which is exactly
    // what an IPv4 address with a port needs.

    // Looks like a host: single token, with a dot (a domain) or a port, or the
    // literal `localhost`. Everything else is a search phrase.
    let single_token = !trimmed.contains(char::is_whitespace);
    let looks_like_host = single_token
        && (trimmed.starts_with("localhost")
            || trimmed.contains('.')
            || trimmed.contains(':'));

    if looks_like_host {
        // `https://` for the open web. Defaulting to `http://` is not merely
        // outdated: most of the web now answers a plain-HTTP request with a
        // CDN error page (speedtest.net returns a Fastly "unknown domain"),
        // so the user would see an error for a site that works fine.
        //
        // Loopback is the exception — a local server has no certificate, so
        // `https://localhost:8080` would simply fail to connect.
        let host = trimmed.split(['/', ':']).next().unwrap_or(trimmed);
        let scheme = if is_loopback(host) { "http" } else { "https" };
        Target::Url(format!("{scheme}://{trimmed}"))
    } else {
        Target::Search(trimmed.to_string())
    }
}

/// True for addresses that are this machine.
fn is_loopback(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host == "127.0.0.1"
        || host == "[::1]"
        || host == "::1"
        || host.ends_with(".localhost")
}

#[derive(Debug, PartialEq, Eq)]
pub enum Target {
    Url(String),
    Search(String),
}

impl Target {
    pub fn into_url(self, searxng: Option<&str>) -> String {
        match self {
            Target::Url(url) => url,
            Target::Search(query) => {
                // A user-configured SearXNG instance wins (self-hosted, no
                // third party, and it is the engine this repo already talks to).
                //
                // The keyless fallback is **Brave Search**, chosen by
                // measurement rather than preference: DuckDuckGo refuses this
                // proxy with 403 (its bot detection dislikes the TLS stack),
                // and Mojeek/Ecosia likewise; Brave answers 200 with real
                // results. See `benchmarks/README.md`.
                let encoded = url::form_urlencoded::byte_serialize(query.as_bytes()).collect::<String>();
                match searxng {
                    Some(base) if !base.trim().is_empty() => {
                        let base = base.trim_end_matches('/');
                        format!("{base}/search?q={encoded}")
                    }
                    _ => format!("https://search.brave.com/search?q={encoded}"),
                }
            }
        }
    }
}

/// GET /api/peakd/state — what the window needs to render itself.
fn state(_ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| async move {
        user_id(&req)?;
        let base = proxy::base();
        let metrics = proxy::metrics();
        let sessions = proxy::list_sessions().await;
        Ok(ok(json!({
            "proxy_base": base,
            "proxy_addr": proxy::addr().map(|a| a.to_string()),
            "ready": base.is_some(),
            "rules": proxy::handle().map(|h| h.filter.rule_count()).unwrap_or(0),
            "filtering_paused": proxy::is_paused(),
            "metrics": metrics,
            "sessions": sessions,
            "home": "about:home",
        })))
    })
}

/// GET /api/peakd/sessions
fn sessions(_ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| async move {
        user_id(&req)?;
        Ok(ok(json!({ "sessions": proxy::list_sessions().await })))
    })
}

#[derive(Deserialize)]
struct NavigateBody {
    /// The session to move. Optional: a missing session means "the one the
    /// window is showing", which is how a first navigation arrives.
    session_id: Option<String>,
    /// Raw address-bar or AI input: a URL, or a search phrase.
    input: String,
    /// `page` (default) returns the URL to load; `text` returns the filtered
    /// document's text, which is what the AI tool wants.
    format: Option<String>,
}

/// POST /api/peakd/navigate — the single entry point for going somewhere.
fn navigate(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let body = take_json::<NavigateBody>(req).await?;

            let target = normalize_input(&body.input);
            let searxng = std::env::var("SEARXNG_URL").ok();
            let url = target.into_url(searxng.as_deref());

            let Some(view_url) = proxy::view_url(&url) else {
                return Err(AppError::Internal(
                    "the browser proxy is not running yet — try again in a moment".into(),
                ));
            };

            // Keep the session (or create one) pointing at the new URL. A
            // stale id from a reloaded page falls back to a fresh session
            // rather than failing the navigation.
            let session = match body.session_id.as_deref() {
                Some(id) => match proxy::update_session(id, Some(url.clone()), None).await {
                    Some(session) => session,
                    None => proxy::create_session(url.clone()).await,
                },
                None => proxy::create_session(url.clone()).await,
            };

            // Best-effort history: browsing must work even if the shared
            // database is unavailable (see the plugin DB caveat in §15).
            let _ = crate::history::record(&ctx, &uid, &url, body.format.as_deref()).await;

            if body.format.as_deref() == Some("text") {
                let text = crate::fetch::text(&view_url).await?;
                return Ok(ok(json!({
                    "session": session,
                    "url": url,
                    "view_url": view_url,
                    "text": text,
                })));
            }

            Ok(ok(json!({
                "session": session,
                "url": url,
                "view_url": view_url,
            })))
        }
    })
}

#[derive(Deserialize)]
struct SessionBody {
    id: String,
}

/// POST /api/peakd/session — open a new session.
fn session_create(_ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| async move {
        let _uid = user_id(&req)?;
        let session = proxy::create_session(String::new()).await;
        Ok(ok(json!({ "session": session })))
    })
}

/// POST /api/peakd/session/close
fn session_close(_ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| async move {
        let _uid = user_id(&req)?;
        let body = take_json::<SessionBody>(req).await?;
        let closed = proxy::close_session(&body.id).await;
        Ok(ok(json!({ "closed": closed })))
    })
}

/// GET /api/peakd/metrics — the counters the window's shield shows.
fn metrics(_ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| async move {
        user_id(&req)?;
        Ok(ok(json!({
            "metrics": proxy::metrics(),
            "rules": proxy::handle().map(|h| h.filter.rule_count()).unwrap_or(0),
            "filtering_paused": proxy::is_paused(),
        })))
    })
}

#[derive(Deserialize)]
struct FilterToggleBody {
    /// Omitted means "flip it", which is what a single toolbar button wants.
    paused: Option<bool>,
}

/// POST /api/peakd/filter/toggle — pause or resume ad blocking.
///
/// Some sites genuinely cannot work with their trackers removed. Without this
/// the only options would be "broken page" or "leave the browser", so the
/// window ships the same escape hatch every browser blocker has.
fn filter_toggle(_ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| async move {
        user_id(&req)?;
        let body = take_json::<FilterToggleBody>(req).await.unwrap_or(FilterToggleBody { paused: None });

        let Some(handle) = proxy::handle() else {
            return Err(AppError::Internal(
                "the browser proxy is not running yet — try again in a moment".into(),
            ));
        };

        let paused = body.paused.unwrap_or(!handle.is_paused());
        handle.set_paused(paused);
        tracing::info!("peakd: ad filtering {}", if paused { "paused" } else { "resumed" });

        Ok(ok(json!({
            "filtering_paused": paused,
            "metrics": proxy::metrics(),
            "rules": handle.filter.rule_count(),
        })))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_word_becomes_a_search() {
        assert_eq!(
            normalize_input("rust webview"),
            Target::Search("rust webview".into())
        );
    }

    #[test]
    fn hostname_defaults_to_https() {
        assert_eq!(
            normalize_input("example.com"),
            Target::Url("https://example.com".into())
        );
        assert_eq!(
            normalize_input("example.com:8080/x"),
            Target::Url("https://example.com:8080/x".into())
        );
    }

    #[test]
    fn localhost_defaults_to_http() {
        // A local dev server has no certificate; https would simply fail.
        assert_eq!(
            normalize_input("localhost:8080"),
            Target::Url("http://localhost:8080".into())
        );
        assert_eq!(
            normalize_input("127.0.0.1:8080/x"),
            Target::Url("http://127.0.0.1:8080/x".into())
        );
    }

    #[test]
    fn explicit_scheme_wins_over_the_default() {
        assert_eq!(
            normalize_input("http://example.com/a"),
            Target::Url("http://example.com/a".into())
        );
    }

    #[test]
    fn explicit_scheme_is_respected() {
        assert_eq!(
            normalize_input("https://example.com/a?b=1"),
            Target::Url("https://example.com/a?b=1".into())
        );
    }

    #[test]
    fn search_url_is_encoded() {
        let url = Target::Search("a b&c".into()).into_url(Some("http://searx.local/"));
        assert_eq!(url, "http://searx.local/search?q=a+b%26c");
    }

    #[test]
    fn search_falls_back_to_brave() {
        // Brave is the keyless default because DuckDuckGo/Mojeek/Ecosia all
        // refuse this proxy; see the doc comment on `Target::into_url`.
        let url = Target::Search("rust".into()).into_url(None);
        assert!(url.starts_with("https://search.brave.com/search?q=rust"));
    }
}
