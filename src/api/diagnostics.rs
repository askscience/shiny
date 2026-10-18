//! Request tracing deliberately excludes URI query strings, headers and bodies.
//! Bootstrap URLs may contain credentials; agent/tool payloads are private.
use axum::{extract::Request, middleware::Next, response::Response};
use tracing::Instrument;

pub async fn trace(request: Request, next: Next) -> Response {
    let id = uuid::Uuid::new_v4().to_string();
    let span = tracing::info_span!("request", request_id = %id,
        method = %request.method(), path = %request.uri().path());
    async move {
        let started = std::time::Instant::now();
        let mut response = next.run(request).await;
        tracing::debug!(
            status = response.status().as_u16(),
            elapsed_ms = started.elapsed().as_millis(),
            "Response headers ready"
        );
        response
            .headers_mut()
            .insert("x-request-id", id.parse().expect("UUID is a valid header"));
        response
    }
    .instrument(span)
    .await
}
