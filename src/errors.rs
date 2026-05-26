use std::fmt;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

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
                (StatusCode::BAD_GATEWAY, "External service error".into())
            }
        };

        (
            status,
            Json(ErrorResponse {
                success: false,
                data: None,
                error: message,
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
