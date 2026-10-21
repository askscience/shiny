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

fn post_json(path: &str, token: Option<&str>, body: Value) -> Request {
    let mut builder = Request::builder()
        .uri(path)
        .method("POST")
        .header("content-type", "application/json");
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    let mut request = builder.body(Body::from(body.to_string())).unwrap();
    request.extensions_mut().insert(ConnectInfo(
        "127.0.0.1:1234".parse::<std::net::SocketAddr>().unwrap(),
    ));
    request
}

/// The session contract behind "Remember my workspace" and the window's X.
///
/// Fresh mode (the default): activations are session-scoped, so the boot reset
/// drops them; deactivating a plugin deletes the activation instead of
/// remembering an "off". A remember-mode "off" is a user choice and survives.
/// Closing a window is a separate signal — it stops the app's process but never
/// touches activation. Both routes are authenticated.
#[tokio::test]
async fn session_reset_and_window_close_follow_the_session_contract() {
    let app = support::TestApp::new().await;
    user(&app, "one").await;
    let router = shiny::api::build_router(app.state.clone());

    // Both routes require a token.
    for path in ["/api/plugins/session/reset", "/api/plugins/window/closed"] {
        let body = if path.ends_with("closed") {
            json!({ "name": "calc" })
        } else {
            json!({})
        };
        assert_eq!(
            router
                .clone()
                .oneshot(post_json(path, None, body.clone()))
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED,
            "{path}"
        );
        let response = router
            .clone()
            .oneshot(post_json(path, Some("one"), body))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        assert_eq!(json_body(response).await["success"], true);
    }

    // Fresh mode: an activation lives in this session only, and the reset drops
    // it — a new sign-in starts with every plugin closed.
    app.state
        .plugins
        .set_enabled_for("one", "hello", true)
        .await
        .unwrap();
    let active = app
        .state
        .plugins
        .try_session_active_set("one")
        .await
        .unwrap();
    assert!(
        active.contains("hello"),
        "the session activation is live: {active:?}"
    );
    let response = router
        .clone()
        .oneshot(post_json(
            "/api/plugins/session/reset",
            Some("one"),
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        app.state
            .plugins
            .try_session_active_set("one")
            .await
            .unwrap()
            .is_empty(),
        "a fresh session starts with no plugin active"
    );

    // A remember-mode "off" is not a session activation: the reset leaves it
    // alone, so the user's explicit choice survives.
    app.state
        .plugins
        .set_enabled_for("one", "hello", false)
        .await
        .unwrap();
    let response = router
        .clone()
        .oneshot(post_json(
            "/api/plugins/session/reset",
            Some("one"),
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        !app.state.plugins.is_enabled_for("one", "hello").await,
        "an explicit off survives the reset"
    );

    // Deactivating in fresh mode deletes the activation instead of writing an
    // "off" — no default-off, and no leftover row for the next session.
    app.state
        .plugins
        .set_enabled_for("one", "hello", true)
        .await
        .unwrap();
    app.state
        .plugins
        .deactivate_for("one", "hello")
        .await
        .unwrap();
    assert!(
        app.state.plugins.is_enabled_for("one", "hello").await,
        "no row means the default (enabled) again, not a remembered off"
    );
    assert!(app
        .state
        .plugins
        .try_session_active_set("one")
        .await
        .unwrap()
        .is_empty());

    // With remember on, the same call writes the persistent "off" instead.
    sqlx::query(
        "INSERT INTO user_preferences (user_id, key, value) \
         VALUES ('one', 'session.remember', 'true')",
    )
    .execute(&app.state.pool)
    .await
    .unwrap();
    app.state
        .plugins
        .set_enabled_for("one", "hello", true)
        .await
        .unwrap();
    app.state
        .plugins
        .deactivate_for("one", "hello")
        .await
        .unwrap();
    assert!(
        !app.state.plugins.is_enabled_for("one", "hello").await,
        "remember mode persists the off"
    );
}
