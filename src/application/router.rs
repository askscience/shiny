use arc_swap::ArcSwap;
use std::sync::Arc;
use std::task::{Context, Poll};
use tower::{Service, ServiceExt};

/// Swappable router handle: implements `tower::Service<IncomingStream>` by
/// delegating to the currently-loaded `Router`, so plugin installs/uninstalls
/// can hot-swap the live router without a server restart.
#[derive(Clone)]
pub struct RouterHandle {
    inner: Arc<ArcSwap<axum::Router>>,
}

impl RouterHandle {
    pub fn new(router: axum::Router) -> Self {
        Self {
            inner: Arc::new(ArcSwap::from_pointee(router)),
        }
    }

    pub fn swap(&self, router: axum::Router) {
        self.inner.store(Arc::new(router));
    }
}

impl RouterHandle {
    fn connection_router(&self, remote: std::net::SocketAddr) -> axum::Router {
        let inner = self.inner.clone();
        // Keep-alive connections must select the current router for *each*
        // request, while an in-flight request retains its own Arc snapshot.
        axum::Router::new().fallback_service(tower::service_fn(
            move |mut request: axum::extract::Request| {
                request
                    .extensions_mut()
                    .insert(axum::extract::ConnectInfo(remote));
                let router = inner.load_full().as_ref().clone();
                router.oneshot(request)
            },
        ))
    }
}

impl<'a> Service<axum::serve::IncomingStream<'a>> for RouterHandle {
    type Response = axum::Router;
    type Error = std::convert::Infallible;
    type Future = std::future::Ready<Result<axum::Router, std::convert::Infallible>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, stream: axum::serve::IncomingStream<'a>) -> Self::Future {
        // The router rotates through this handle, so the peer address cannot
        // be injected by a layer built at router-construction time. Add it
        // here, per connection, as `ConnectInfo`: handlers use it to tell a
        // local request from a remote one (network changes are loopback-only).
        let router = self.connection_router(stream.remote_addr());
        std::future::ready(Ok(router))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{to_bytes, Body},
        extract::Request,
        routing::get,
    };

    #[tokio::test]
    async fn an_existing_connection_sees_a_rebuilt_router() {
        let handle =
            RouterHandle::new(axum::Router::new().route("/version", get(|| async { "old" })));
        let connection = handle.connection_router("127.0.0.1:1234".parse().unwrap());
        let request = || {
            Request::builder()
                .uri("/version")
                .body(Body::empty())
                .unwrap()
        };
        let response = connection.clone().oneshot(request()).await.unwrap();
        assert_eq!(to_bytes(response.into_body(), 100).await.unwrap(), "old");
        handle.swap(axum::Router::new().route("/version", get(|| async { "new" })));
        let response = connection.oneshot(request()).await.unwrap();
        assert_eq!(to_bytes(response.into_body(), 100).await.unwrap(), "new");
    }
    #[tokio::test]
    async fn a_router_swap_preserves_in_flight_requests() {
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let started = Arc::new(std::sync::Mutex::new(Some(started_tx)));
        let release = Arc::new(tokio::sync::Notify::new());
        let old = axum::Router::new().route(
            "/version",
            get({
                let release = release.clone();
                move || {
                    let started = started.clone();
                    let release = release.clone();
                    async move {
                        let _ = started.lock().unwrap().take().unwrap().send(());
                        release.notified().await;
                        "old"
                    }
                }
            }),
        );
        let handle = RouterHandle::new(old);
        let connection = handle.connection_router("127.0.0.1:1234".parse().unwrap());
        let request = || {
            Request::builder()
                .uri("/version")
                .body(Body::empty())
                .unwrap()
        };
        let running = tokio::spawn(connection.clone().oneshot(request()));
        started_rx.await.unwrap();
        handle.swap(axum::Router::new().route("/version", get(|| async { "new" })));
        let response = connection.oneshot(request()).await.unwrap();
        assert_eq!(to_bytes(response.into_body(), 100).await.unwrap(), "new");
        release.notify_one();
        let response = running.await.unwrap().unwrap();
        assert_eq!(to_bytes(response.into_body(), 100).await.unwrap(), "old");
    }
}
