use axum::body::{Body, HttpBody};
use axum::extract::{State, Request};
use axum::middleware::Next;
use axum::response::Response;
use axum::http::StatusCode;
use bytes::Bytes;
use http::header::AUTHORIZATION;

use crate::api::AppState;
use crate::models::Traveler;

pub async fn auth_middleware<B>(
    State(state): State<AppState>,
    req: Request<B>,
    next: Next,
) -> Result<Response, StatusCode>
where
    B: HttpBody<Data = Bytes> + Send + 'static,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    let auth_header = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|v| v.to_string());

    let cookie_token = req
        .headers()
        .get(axum::http::header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(extract_session_cookie);

    // Try the Bearer token first, then the `shiny_token` session cookie. A
    // valid cookie rescues a request whose localStorage token has gone stale,
    // and keeps a user signed in across page reloads.
    let mut traveler = None;
    for candidate in [auth_header, cookie_token].into_iter().flatten() {
        match sqlx::query_as::<_, Traveler>("SELECT * FROM travelers WHERE auth_token = ?1")
            .bind(&candidate)
            .fetch_optional(&state.pool)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        {
            Some(t) => {
                traveler = Some(t);
                break;
            }
            None => continue,
        }
    }
    let traveler = traveler.ok_or(StatusCode::UNAUTHORIZED)?;

    let user_id = traveler.id.clone();

    let mut req = req.map(Body::new);
    {
        let extensions = req.extensions_mut();
        // Portable identity for plugin route handlers (they can't reference
        // the core `Traveler` type).
        extensions.insert(shiny_plugin_sdk::routes::UserId(user_id.clone()));
        extensions.insert(shiny_plugin_sdk::routes::TravelerId(user_id));
        extensions.insert(traveler);
    }

    Ok(next.run(req).await)
}

/// Extract the `shiny_token` value from a raw `Cookie` header, if present.
fn extract_session_cookie(cookie_header: &str) -> Option<String> {
    cookie_header.split(';').find_map(|part| {
        let (name, value) = part.trim().split_once('=')?;
        (name == "shiny_token" && !value.is_empty()).then(|| value.to_string())
    })
}
