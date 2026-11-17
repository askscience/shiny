//! The wire contract between the Shiny host and a plugin that runs as its own
//! process (`runtime = "process"` in `plugin.toml`).
//!
//! A process plugin is an executable. The host starts it with the path of a
//! Unix socket in [`SOCKET_ENV`]; the plugin listens there and speaks
//! HTTP/1.1. Two kinds of request reach it:
//!
//! - **control calls**: `POST` with a JSON body to one of the [`paths`] below
//!   (handshake, lifecycle hooks, agent tools);
//! - **route requests**: the user's HTTP request to one of the plugin's routes,
//!   forwarded unchanged (method, URI, headers, body) with the route's
//!   `handler_tag` added in [`ROUTE_TAG_HEADER`]. Identity and path parameters
//!   travel in the same `x-shiny-*` headers a library plugin reads.
//!
//! # Compatibility rules
//!
//! This crate *is* the compatibility promise, so it changes by rules:
//!
//! - The host and the plugin each support a range of protocol versions and the
//!   handshake picks the highest one both know ([`negotiate`]).
//! - **Additive changes keep the version**: a new optional field (always with
//!   `#[serde(default)]`), a new control path a peer may not implement (a 404
//!   means "not supported"). Unknown fields are ignored on both sides: no type
//!   here uses `deny_unknown_fields`, and none may.
//! - **A breaking change bumps [`PROTOCOL_MAX`]** and the host keeps speaking
//!   the previous versions for several releases; [`PROTOCOL_MIN`] only rises
//!   when that window closes.
//! - Payloads shaped by the SDK (agent context, artifacts, navigation, the
//!   config snapshot) travel as JSON values and follow the same additive rule.
//!
//! `tests/v1_messages.rs` keeps every message a version-1 peer sends, byte for
//! byte: they must keep decoding as long as version 1 is supported.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Oldest protocol version this build speaks.
pub const PROTOCOL_MIN: u32 = 1;
/// Newest protocol version this build speaks.
pub const PROTOCOL_MAX: u32 = 1;

/// Environment variable carrying the socket path the plugin must listen on.
pub const SOCKET_ENV: &str = "SHINY_PLUGIN_SOCKET";
/// Environment variable carrying the plugin's name (for its logs).
pub const NAME_ENV: &str = "SHINY_PLUGIN_NAME";
/// Environment variable carrying the plugin's install directory. The process
/// runs in the server's working directory (relative paths a library plugin
/// used keep meaning the same thing), so this is how it finds its own files.
pub const DIR_ENV: &str = "SHINY_PLUGIN_DIR";
/// Environment variable carrying the server executable's path, for plugins
/// that look for helper programs installed next to it.
pub const HOST_EXE_ENV: &str = "SHINY_HOST_EXE";
/// Header naming the route a forwarded request is for.
pub const ROUTE_TAG_HEADER: &str = "x-shiny-route-tag";

/// Control endpoints. Everything under `/_shiny/` is reserved for them; any
/// other path is a forwarded route request.
pub mod paths {
    /// [`super::Hello`] → [`super::Welcome`]. Always the first call.
    pub const HELLO: &str = "/_shiny/hello";
    /// `on_load`: start background work. Empty body, [`super::Done`] back.
    pub const LOAD: &str = "/_shiny/load";
    /// `on_unload`: flush state before the host stops the process.
    pub const UNLOAD: &str = "/_shiny/unload";
    /// [`super::UserRegistered`] → [`super::Done`].
    pub const USER_REGISTERED: &str = "/_shiny/user-registered";
    /// [`super::ToolCall`] → [`super::ToolReply`].
    pub const TOOL: &str = "/_shiny/tool";
    /// Prefix shared by every control endpoint.
    pub const PREFIX: &str = "/_shiny/";
}

/// An inclusive range of protocol versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionRange {
    pub min: u32,
    pub max: u32,
}

impl VersionRange {
    /// The range this build of the crate speaks.
    pub const fn current() -> Self {
        Self {
            min: PROTOCOL_MIN,
            max: PROTOCOL_MAX,
        }
    }
}

/// The highest version both ranges contain, if any.
pub fn negotiate(a: VersionRange, b: VersionRange) -> Option<u32> {
    let low = a.min.max(b.min);
    let high = a.max.min(b.max);
    (low <= high).then_some(high)
}

/// Host → plugin, first call: who the host is and what it speaks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hello {
    pub protocol: VersionRange,
    /// The host's version (`0.2.8`), for the plugin's logs and diagnostics.
    #[serde(default)]
    pub host_version: String,
    /// The SDK's `ConfigSnapshot` as JSON.
    pub config: Value,
}

/// Plugin → host, reply to [`Hello`]: the chosen version and everything the
/// plugin contributes.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Welcome {
    /// The version picked with [`negotiate`]; `0` when none fits, and then
    /// `error` says why.
    pub protocol: u32,
    /// The SDK's `Manifest` as JSON.
    #[serde(default)]
    pub manifest: Value,
    #[serde(default)]
    pub tools: Vec<ToolSpec>,
    #[serde(default)]
    pub routes: Vec<RouteDecl>,
    #[serde(default)]
    pub skills_md: String,
    #[serde(default)]
    pub persona: String,
    #[serde(default)]
    pub context_lines: Vec<String>,
    /// Stop the process after this many idle seconds; the next request starts
    /// it again. `None`: keep running (background work started in `on_load`).
    #[serde(default)]
    pub idle_stop_secs: Option<u64>,
    /// The SDK revision the plugin was built with, for diagnostics only.
    #[serde(default)]
    pub sdk: String,
    #[serde(default)]
    pub error: Option<String>,
}

/// One agent tool, described once at handshake.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub step_label: String,
    #[serde(default)]
    pub doc_fragment: Option<String>,
    #[serde(default)]
    pub step_ui: Option<UiText>,
}

/// One HTTP route the plugin serves.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteDecl {
    /// `GET`, `POST`, `PUT`, `DELETE` or `PATCH`.
    pub method: String,
    pub path: String,
    /// `auth`, `public` or `admin`.
    #[serde(default = "default_auth")]
    pub auth: String,
    pub handler_tag: String,
}

fn default_auth() -> String {
    "auth".into()
}

/// Translatable text (mirrors the SDK's `UiText`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UiText {
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub params: serde_json::Map<String, Value>,
    #[serde(default)]
    pub text: String,
}

/// Host → plugin: run one agent tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub tool: String,
    pub user_id: String,
    pub traveler_id: String,
    #[serde(default)]
    pub params: Value,
    /// The SDK's `AgentContext` as JSON.
    #[serde(default)]
    pub context: Value,
    #[serde(default)]
    pub os_home: Option<String>,
}

/// Plugin → host: what one tool call produced.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolReply {
    /// Present when the tool returned an outcome.
    #[serde(default)]
    pub outcome: Option<Outcome>,
    /// Present when the tool failed.
    #[serde(default)]
    pub error: Option<WireError>,
    /// The tool's `humanize(result, data)` for this outcome.
    #[serde(default)]
    pub humanized: Option<String>,
    /// The tool's `done_ui(result, data)` for this outcome.
    #[serde(default)]
    pub done_ui: Option<UiText>,
}

/// The SDK's `ActionOutcome`, field by field.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Outcome {
    pub action: String,
    pub result: String,
    #[serde(default)]
    pub data: Value,
    #[serde(default)]
    pub artifact: Option<Value>,
    #[serde(default)]
    pub extra_artifacts: Vec<Value>,
    #[serde(default)]
    pub navigation: Option<Value>,
}

/// An error crossing the socket. `kind` maps back onto the SDK's `AppError`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WireError {
    /// `not_found`, `unauthorized`, `bad_request` or `internal`.
    pub kind: String,
    pub message: String,
}

/// Host → plugin: a new user registered.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserRegistered {
    pub user_id: String,
}

/// Generic acknowledgement for calls that return nothing.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Done {
    #[serde(default)]
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negotiation_picks_the_highest_shared_version() {
        let host = VersionRange { min: 1, max: 3 };
        assert_eq!(negotiate(host, VersionRange { min: 1, max: 1 }), Some(1));
        assert_eq!(negotiate(host, VersionRange { min: 2, max: 5 }), Some(3));
        assert_eq!(negotiate(host, VersionRange { min: 4, max: 5 }), None);
    }

    #[test]
    fn unknown_fields_are_ignored_and_missing_ones_default() {
        // A newer peer may send fields this build does not know; an older one
        // may leave out fields added since. Both must decode.
        let welcome: Welcome = serde_json::from_str(
            r#"{"protocol":1,"tools":[{"name":"x","future":true}],"later":1}"#,
        )
        .unwrap();
        assert_eq!(welcome.tools[0].name, "x");
        assert!(welcome.routes.is_empty());
        let reply: ToolReply = serde_json::from_str("{}").unwrap();
        assert!(reply.outcome.is_none() && reply.error.is_none());
    }
}
