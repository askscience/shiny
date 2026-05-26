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

    let token = match auth_header {
        Some(t) => t,
        None => return Err(StatusCode::UNAUTHORIZED),
    };

    let traveler = sqlx::query_as::<_, Traveler>(
        "SELECT * FROM travelers WHERE auth_token = ?1",
    )
    .bind(&token)
    .fetch_optional(&state.pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    .ok_or(StatusCode::UNAUTHORIZED)?;

    let mut req = req.map(Body::new);
    req.extensions_mut().insert(traveler);

    Ok(next.run(req).await)
}
