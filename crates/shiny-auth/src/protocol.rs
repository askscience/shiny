//! The wire protocol between the Shiny server and the `shiny-auth` helper.
//!
//! One request per connection: a single JSON line in, a single JSON line out.
//! The socket is a local Unix socket, so this is not a network protocol.

use serde::{Deserialize, Serialize};

/// Request from the server.
#[derive(Debug, Deserialize)]
pub struct Request {
    /// `"verify"` (default) or `"ping"`.
    #[serde(default)]
    pub op: Option<String>,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
}

/// Response to the server. Never carries the password back.
#[derive(Debug, Serialize)]
pub struct Response {
    pub ok: bool,
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl Response {
    pub fn ok() -> Self {
        Self {
            ok: true,
            code: "success".into(),
            message: None,
        }
    }

    pub fn denied(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            code: "denied".into(),
            message: Some(message.into()),
        }
    }

    pub fn unavailable(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            code: "unavailable".into(),
            message: Some(message.into()),
        }
    }

    pub fn error(code: &str, message: impl Into<String>) -> Self {
        Self {
            ok: false,
            code: code.into(),
            message: Some(message.into()),
        }
    }

    pub fn to_line(&self) -> String {
        let mut line = serde_json::to_string(self).unwrap_or_else(|_| {
            r#"{"ok":false,"code":"internal","message":"encode failed"}"#.to_string()
        });
        line.push('\n');
        line
    }
}
