//! Route contribution spec. Plugins build `Vec<RouteSpec>` at registration
//! time; the core installer mounts them onto the live router.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Delete,
    Patch,
}

impl HttpMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            HttpMethod::Get => "GET",
            HttpMethod::Post => "POST",
            HttpMethod::Put => "PUT",
            HttpMethod::Delete => "DELETE",
            HttpMethod::Patch => "PATCH",
        }
    }
}

/// Static route spec. `handler_tag` is a stable identifier this plugin
/// resolves back to a function via its own dispatch table when the core
/// installer calls `Plugin::route_handler(tag)`.
///
/// (The SDK stays axum-shape-agnostic. Each plugin is responsible for
/// turning a tag into an `axum::Handler` inside its `Plugin::build_routes`
/// implementation; the loadable interface keeps the trait surface tiny.)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteSpec {
    pub method: HttpMethod,
    pub path: String,
    /// Auth requirement: "auth" (must present a valid bearer token),
    /// "public" (no auth), "admin" (must be a user with `is_admin=1`).
    /// Note: the current core has no admin role — "admin" is treated as "auth".
    pub auth: String,
    /// Stable tag the plugin's `route_handler()` resolves to a real handler.
    pub handler_tag: String,
}

// ─────────────────────────────────────────────────────────────
// Route handlers & identity
// ─────────────────────────────────────────────────────────────

/// Authenticated identity the core auth middleware injects into plugin route
/// requests before dispatching. Plugins read these from request extensions —
/// core's `Traveler` type is not visible to plugins, so these SDK types are
/// the portable identity surface.
#[derive(Debug, Clone)]
pub struct UserId(pub String);

#[derive(Debug, Clone)]
pub struct TravelerId(pub String);

/// Header names the core auth middleware uses to hand the authenticated
/// identity to plugin route handlers.
///
/// These cannot be request *extensions*: each plugin cdylib statically links
/// its own copy of this SDK, so a `UserId` type inside the plugin has a
/// different `TypeId` than the one the core inserted, and `http::Extensions`
/// keys by `TypeId` — the lookup always misses. Header names are matched by
/// plain string comparison, so they cross the dlopen boundary safely.
pub const USER_ID_HEADER: &str = "x-shiny-user-id";
pub const TRAVELER_ID_HEADER: &str = "x-shiny-traveler-id";

/// Read the authenticated user id the core auth middleware injected as a
/// request header.
pub fn user_id_from_request(req: &axum::extract::Request) -> Option<String> {
    req.headers()
        .get(USER_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}

/// Read the authenticated traveler id the core auth middleware injected as a
/// request header.
pub fn traveler_id_from_request(req: &axum::extract::Request) -> Option<String> {
    req.headers()
        .get(TRAVELER_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}

/// Header the core router uses to hand captured path parameters (e.g. the
/// `:id` in `/api/documents/:id`) to plugin route handlers.
///
/// Path params have the same dlopen-boundary problem as identity: they are
/// stored by axum in request extensions as its private `UrlParams` type, and a
/// plugin's copy of axum has a different `TypeId` for that type, so
/// `axum::extract::Path` inside a plugin can never see them. The core therefore
/// re-serializes the matched params into this header before dispatching.
pub const PATH_PARAMS_HEADER: &str = "x-shiny-path-params";

/// Read the path parameters the core injected as a JSON array header. Returns
/// them as `(name, value)` pairs, in route declaration order and with percent
/// decoding already applied.
pub fn path_params_from_request(req: &axum::extract::Request) -> Option<Vec<(String, String)>> {
    let value = req.headers().get(PATH_PARAMS_HEADER)?.to_str().ok()?;
    serde_json::from_str(value).ok()
}

/// A resolved plugin route handler. Takes the incoming request (with the
/// authenticated identity in its extensions) and returns a response.
///
/// **Always wrap the body with [`bridged_route`]** so any sqlx/reqwest/tokio
/// work runs on the plugin-owned runtime (§15) — the route equivalent of
/// `bridged(...)` for tools.
pub type RouteHandler = Arc<
    dyn Fn(axum::extract::Request) -> Pin<Box<dyn Future<Output = Response> + Send>>
        + Send
        + Sync,
>;

/// Wrap an async `(Request) -> Result<Response, AppError>` handler so it runs
/// on the plugin-owned runtime. Use exactly like `bridged(...)` for tools.
/// `AppError` is converted to its HTTP response inside the bridge.
pub fn bridged_route<F, Fut>(f: F) -> RouteHandler
where
    F: Fn(axum::extract::Request) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<Response, crate::errors::AppError>> + Send + 'static,
{
    Arc::new(move |req| {
        let fut = f(req);
        Box::pin(crate::rt::bridge(async move {
            match fut.await {
                Ok(resp) => resp,
                Err(e) => e.into_response(),
            }
        }))
    })
}