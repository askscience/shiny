//! Route contribution spec. Plugins build `Vec<RouteSpec>` at registration
//! time; the core installer mounts them onto the live router.

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
    pub auth: String,
    /// Stable tag the plugin's `route_handler()` resolves to a real handler.
    pub handler_tag: String,
}