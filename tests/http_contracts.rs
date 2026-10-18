mod support;
use axum::{
    body::{to_bytes, Body},
    extract::{ConnectInfo, Request},
    http::StatusCode,
};
use serde_json::{json, Value};
use tower::ServiceExt;

fn request(path: &str, token: Option<&str>) -> Request {
    let mut builder = Request::builder().uri(path);
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    let mut request = builder.body(Body::empty()).unwrap();
    request.extensions_mut().insert(ConnectInfo(
        "127.0.0.1:1234".parse::<std::net::SocketAddr>().unwrap(),
    ));
    request
}
async fn user(app: &support::TestApp, id: &str) {
    sqlx::query("INSERT INTO travelers (id, name, email, password_hash, auth_token) VALUES (?, ?, ?, '!', ?)")
        .bind(id).bind(id).bind(format!("{id}@test")).bind(id).execute(&app.state.pool).await.unwrap();
}
async fn json_body(response: axum::response::Response) -> Value {
    serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap()).unwrap()
}

#[tokio::test]
async fn health_is_local_only_and_optional_providers_do_not_block_readiness() {
    let app = support::TestApp::new().await;
    let router = shiny::api::build_router(app.state.clone());
    for path in ["/api/health/live", "/api/health/ready"] {
        let response = router.clone().oneshot(request(path, None)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().contains_key("x-request-id"));
        assert_eq!(json_body(response).await["success"], true);
        for header in [
            "x-shiny-remote",
            "x-forwarded-for",
            "x-forwarded-proto",
            "x-forwarded-host",
        ] {
            let mut req = request(path, None);
            req.headers_mut().insert(header, "remote".parse().unwrap());
            assert_eq!(
                router.clone().oneshot(req).await.unwrap().status(),
                StatusCode::FORBIDDEN
            );
        }
        let mut req = request(path, None);
        req.extensions_mut().insert(ConnectInfo(
            "192.0.2.10:1234".parse::<std::net::SocketAddr>().unwrap(),
        ));
        assert_eq!(
            router.clone().oneshot(req).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
    }
    app.state.pool.close().await;
    assert_eq!(
        router
            .clone()
            .oneshot(request("/api/health/ready", None))
            .await
            .unwrap()
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        router
            .oneshot(request("/api/health/live", None))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn protected_routes_preserve_auth_and_response_contracts() {
    let app = support::TestApp::new().await;
    user(&app, "one").await;
    let router = shiny::api::build_router(app.state.clone());
    for path in [
        "/api/travelers/me",
        "/api/preferences",
        "/api/plugins/active",
        "/api/chat/conversations",
    ] {
        assert_eq!(
            router
                .clone()
                .oneshot(request(path, None))
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        let response = router
            .clone()
            .oneshot(request(path, Some("one")))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        assert_eq!(json_body(response).await["success"], true);
    }
    let mut req = request("/api/preferences", Some("stale"));
    req.headers_mut()
        .insert("cookie", "shiny_token=one".parse().unwrap());
    assert_eq!(router.oneshot(req).await.unwrap().status(), StatusCode::OK);
}

#[tokio::test]
async fn preference_formats_and_account_isolation_are_preserved() {
    let app = support::TestApp::new().await;
    for id in ["one", "two"] {
        user(&app, id).await;
    }
    let router = shiny::api::build_router(app.state.clone());
    let preferences = json!({"session.remember":"true", "desktop.layout":"{\"mode\":\"windows\"}"});
    let mut req = Request::builder()
        .uri("/api/preferences")
        .method("PUT")
        .header("authorization", "Bearer one")
        .header("content-type", "application/json")
        .body(Body::from(preferences.to_string()))
        .unwrap();
    req.extensions_mut().insert(ConnectInfo(
        "127.0.0.1:1234".parse::<std::net::SocketAddr>().unwrap(),
    ));
    assert_eq!(
        router.clone().oneshot(req).await.unwrap().status(),
        StatusCode::OK
    );
    assert_eq!(
        json_body(
            router
                .clone()
                .oneshot(request("/api/preferences", Some("one")))
                .await
                .unwrap()
        )
        .await["data"],
        preferences
    );
    assert_eq!(
        json_body(
            router
                .oneshot(request("/api/preferences", Some("two")))
                .await
                .unwrap()
        )
        .await["data"],
        json!({})
    );
}

#[tokio::test]
async fn remote_host_mutations_are_refused_before_touching_the_host() {
    let app = support::TestApp::new().await;
    user(&app, "one").await;
    let router = shiny::api::build_router(app.state.clone());
    for path in [
        "/api/power/off",
        "/api/network/disconnect",
        "/api/audio/mute",
        "/api/bluetooth/scan",
    ] {
        let mut req = Request::builder()
            .uri(path)
            .method("POST")
            .header("authorization", "Bearer one")
            .header("x-shiny-remote", "true")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut().insert(ConnectInfo(
            "127.0.0.1:1234".parse::<std::net::SocketAddr>().unwrap(),
        ));
        assert_eq!(
            router.clone().oneshot(req).await.unwrap().status(),
            StatusCode::FORBIDDEN,
            "{path}"
        );
    }
}

#[tokio::test]
async fn storage_failure_is_not_reported_as_an_empty_plugin_session() {
    let app = support::TestApp::new().await;
    app.state.pool.close().await;
    assert!(app
        .state
        .plugins
        .try_session_active_set("one")
        .await
        .is_err());
    assert!(!app.state.plugins.is_enabled_for("one", "terminal").await);
}
