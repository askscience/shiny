//! REST routes for the browser window.
//!
//! The window is a client of the plugin, not a peer of it: every action the
//! user takes (go, back, reload, search) round-trips through one of these so
//! the session and the history log stay consistent. The page itself is rendered
//! by a native child webview (`crates/peakd`'s `browse` module), so no proxied
//! view URL has to be handed back.

use std::sync::Arc;

use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};

use shiny_plugin_sdk::errors::AppError;
use shiny_plugin_sdk::routes::{bridged_route, RouteHandler, user_id_from_request};
use shiny_plugin_sdk::services::PluginCtx;

use crate::sessions;

pub fn handle(ctx: &Arc<PluginCtx>, tag: &str) -> Option<RouteHandler> {
    let ctx = ctx.clone();
    match tag {
        "browser_state" => Some(state(ctx)),
        "browser_sessions" => Some(sessions_route(ctx)),
        "browser_session_create" => Some(session_create(ctx)),
        "browser_session_close" => Some(session_close(ctx)),
        "browser_navigate" => Some(navigate(ctx)),
        "browser_history" => Some(history_route(ctx.clone())),
        "browser_news" => Some(news_route(ctx.clone())),
        "browser_news_click" => Some(news_click(ctx)),
        "browser_preview" => Some(preview_route(ctx)),
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
                // measurement rather than preference: through the old proxied
                // client DuckDuckGo answered 403 (its bot detection disliked
                // the TLS stack) and Mojeek/Ecosia likewise; Brave answered 200
                // with real results.
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

/// GET /api/browser/state — what the window needs to render itself.
fn state(_ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| async move {
        user_id(&req)?;
        let sessions = sessions::list_sessions().await;
        Ok(ok(json!({
            "ready": true,
            "sessions": sessions,
            "home": "about:home",
        })))
    })
}

/// GET /api/browser/sessions
fn sessions_route(_ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| async move {
        user_id(&req)?;
        Ok(ok(json!({ "sessions": sessions::list_sessions().await })))
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

/// POST /api/browser/navigate — the single entry point for going somewhere.
fn navigate(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let body = take_json::<NavigateBody>(req).await?;

            let raw = body.input.trim().to_string();
            let target = normalize_input(&raw);
            // A typed phrase is a search, and a search is the strongest
            // interest signal the home surface's recommender gets — keep the
            // words the user actually typed, not the engine URL they became.
            let query: Option<String> = match &target {
                Target::Search(q) if !q.trim().is_empty() => Some(q.trim().to_string()),
                _ => None,
            };
            let searxng = std::env::var("SEARXNG_URL").ok();
            let url = target.into_url(searxng.as_deref());

            // Keep the session (or create one) pointing at the new URL. A
            // stale id from a reloaded page falls back to a fresh session
            // rather than failing the navigation.
            let session = match body.session_id.as_deref() {
                Some(id) => match sessions::update_session(id, Some(url.clone()), None).await {
                    Some(session) => session,
                    None => sessions::create_session(url.clone()).await,
                },
                None => sessions::create_session(url.clone()).await,
            };

            // Best-effort history: browsing must work even if the shared
            // database is unavailable (see the plugin DB caveat in §15).
            let _ = crate::history::record(
                &ctx,
                &uid,
                &url,
                body.format.as_deref(),
                query.as_deref(),
            )
            .await;

            if body.format.as_deref() == Some("text") {
                let text = crate::fetch::text(&url).await?;
                return Ok(ok(json!({
                    "session": session,
                    "url": url,
                    "text": text,
                })));
            }

            Ok(ok(json!({
                "session": session,
                "url": url,
            })))
        }
    })
}

#[derive(Deserialize)]
struct SessionBody {
    id: String,
}

/// POST /api/browser/session — open a new session.
fn session_create(_ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| async move {
        let _uid = user_id(&req)?;
        let session = sessions::create_session(String::new()).await;
        Ok(ok(json!({ "session": session })))
    })
}

/// POST /api/browser/session/close
fn session_close(_ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| async move {
        let _uid = user_id(&req)?;
        let body = take_json::<SessionBody>(req).await?;
        let closed = sessions::close_session(&body.id).await;
        Ok(ok(json!({ "closed": closed })))
    })
}

/// GET /api/browser/history — recent navigations, newest first.
///
/// Exposed for two reasons: the home surface's "because you searched for …"
/// chips are built from the same rows the recommender ranks, and a user can
/// see what the algorithm has learned about them. A recommender the user
/// cannot inspect is indistinguishable from one that is broken.
fn history_route(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let limit = query_limit(&req).unwrap_or(20);
            let rows = crate::history::recent_rows(&ctx, &uid, limit).unwrap_or_default();
            Ok(ok(json!({ "history": rows })))
        }
    })
}

/// GET /api/browser/news — the related-news cards for the home surface.
///
/// `?refresh=1` bypasses the per-topic cache (the home surface's refresh
/// button), `?limit=` caps the shelf.
fn news_route(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let refresh = query_flag(&req, "refresh");
            let limit = query_limit(&req).unwrap_or(12).clamp(1, 30);
            let news = crate::news::home_news(&ctx, &uid, limit, refresh).await?;
            Ok(ok(serde_json::to_value(news).unwrap_or(Value::Null)))
        }
    })
}

#[derive(Deserialize)]
struct NewsClickBody {
    /// The card's URL.
    url: String,
    /// The interest the card was recommended for; re-recording it is what
    /// makes a click a *stronger* signal than a passive visit.
    #[serde(default)]
    topic: Option<String>,
}

/// POST /api/browser/news/click — record a click on a recommended card.
///
/// The home surface sends this before navigating. It is the recommender's only
/// explicit feedback channel: a card the user opened is worth more than one
/// they scrolled past, and it costs one row in the same history table.
fn news_click(ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| {
        let ctx = ctx.clone();
        async move {
            let uid = user_id(&req)?;
            let body = take_json::<NewsClickBody>(req).await?;
            let _ = crate::history::record(
                &ctx,
                &uid,
                &body.url,
                Some("news_click"),
                body.topic.as_deref(),
            )
            .await;
            Ok(ok(json!({ "recorded": true })))
        }
    })
}

#[derive(Deserialize)]
struct PreviewBody {
    /// The link the pointer is resting on.
    url: String,
}

/// POST /api/browser/preview — title/description/image for a hovered link.
///
/// The window fetches this on hover (debounced) to show a preview card. It is
/// SSRF-guarded inside [`crate::preview::fetch`], which also caches the result
/// so a pointer sweeping a page never turns into a request storm.
fn preview_route(_ctx: Arc<PluginCtx>) -> RouteHandler {
    bridged_route(move |req: axum::extract::Request| async move {
        user_id(&req)?;
        let body = take_json::<PreviewBody>(req).await?;
        let preview = crate::preview::fetch(&body.url).await?;
        Ok(ok(serde_json::to_value(preview).unwrap_or(Value::Null)))
    })
}

/// Read an integer query parameter without a `Query<T>` extractor.
///
/// The bridged routes hand the handler a whole `Request`, and pulling one
/// value out of the query string is cheaper than reconstructing the parts for
/// an extractor that would then consume the body these routes do not use.
fn query_param(req: &axum::extract::Request, key: &str) -> Option<String> {
    let query = req.uri().query()?;
    url::form_urlencoded::parse(query.as_bytes())
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.into_owned())
}

fn query_limit(req: &axum::extract::Request) -> Option<usize> {
    query_param(req, "limit")
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|n| *n > 0)
}

fn query_flag(req: &axum::extract::Request, key: &str) -> bool {
    matches!(
        query_param(req, key).as_deref(),
        Some("1") | Some("true") | Some("yes")
    )
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
        // answered 403 to the old proxied client.
        let url = Target::Search("rust".into()).into_url(None);
        assert!(url.starts_with("https://search.brave.com/search?q=rust"));
    }
}
