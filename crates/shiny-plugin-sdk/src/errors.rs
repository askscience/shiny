//! Plugin-visible error type. Same wire shape as the core AppError.

use std::fmt;
use std::sync::OnceLock;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

/// Host-installed message translator. The `shiny` core registers one that maps
/// an English user-facing message to the caller's interface language; without a
/// host (a plugin exercised standalone, tests) messages pass through unchanged.
static TRANSLATOR: OnceLock<fn(&str) -> String> = OnceLock::new();

/// Install the process-wide translator. First call wins.
pub fn set_message_translator(translator: fn(&str) -> String) {
    let _ = TRANSLATOR.set(translator);
}

/// Localize a user-facing message (identity when no translator is installed).
pub fn translate(message: impl Into<String>) -> String {
    let message = message.into();
    match TRANSLATOR.get() {
        Some(translate) => translate(&message),
        None => message,
    }
}

#[derive(Debug)]
pub enum AppError {
    NotFound(String),
    Unauthorized(String),
    BadRequest(String),
    Internal(String),
    Database(sqlx::Error),
    Http(reqwest::Error),
}

#[derive(Serialize)]
struct ErrorResponse {
    success: bool,
    data: Option<()>,
    error: String,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
            AppError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, msg),
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
            AppError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg),
            AppError::Database(e) => {
                tracing::error!("Database error: {}", e);
                (StatusCode::INTERNAL_SERVER_ERROR, "Database error".into())
            }
            AppError::Http(e) => {
                tracing::error!("HTTP error: {}", e);
                let msg = if e.is_connect() {
                    "Could not reach an external service. Check your network connection.".into()
                } else if e.is_timeout() {
                    "Request timed out. Try again.".into()
                } else {
                    format!("External service error: {}", e)
                };
                (StatusCode::BAD_GATEWAY, msg)
            }
        };

        (
            status,
            Json(ErrorResponse {
                success: false,
                data: None,
                error: translate(message),
            }),
        )
            .into_response()
    }
}

impl From<sqlx::Error> for AppError {
    fn from(e: sqlx::Error) -> Self {
        AppError::Database(e)
    }
}

impl From<reqwest::Error> for AppError {
    fn from(e: reqwest::Error) -> Self {
        AppError::Http(e)
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError::Internal(format!("IO error: {}", e))
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        AppError::Internal(format!("JSON error: {}", e))
    }
}

impl std::error::Error for AppError {}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppError::NotFound(msg) => write!(f, "Not found: {}", msg),
            AppError::Unauthorized(msg) => write!(f, "Unauthorized: {}", msg),
            AppError::BadRequest(msg) => write!(f, "Bad request: {}", msg),
            AppError::Internal(msg) => write!(f, "Internal error: {}", msg),
            AppError::Database(e) => write!(f, "Database error: {}", e),
            AppError::Http(e) => write!(f, "HTTP error: {}", e),
        }
    }
}