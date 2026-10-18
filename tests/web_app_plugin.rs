//! A `kind = "web"` plugin is a static site the shell frames in a window, so
//! its route has to carry different headers from everything else the core
//! serves — and nothing else may change: the plugin needs no cdylib, registers
//! no tools, and still appears in `/api/plugins` as a web-app surface.
mod support;

use axum::{
    body::{to_bytes, Body},
    extract::{ConnectInfo, Request},
    http::StatusCode,
};
use serde_json::Value;
use std::path::Path;
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

/// Write a web-app plugin fixture: manifest plus the app's static site.
fn stage_web_app(app: &support::TestApp, name: &str) {
    let dir = Path::new(&app.state.config.plugins_dir).join(name);
    std::fs::create_dir_all(dir.join("web")).unwrap();
    std::fs::write(
        dir.join("plugin.toml"),
        format!(
            "name = \"{name}\"\n\
             version = \"0.10.0\"\n\
             api_level = 1\n\
             kind = \"web\"\n\
             description = \"Image editing\"\n\
             summary = \"PhotoCraft — the app's own web build\"\n\
             web_dir = \"web\"\n\
             category = \"Craft\"\n"
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join("web/index.html"),
        "<!doctype html><html><head><title>PhotoCraft</title></head><body>app</body></html>",
    )
    .unwrap();
    std::fs::write(dir.join("web/app_web_bg.wasm"), b"\0asm\x01\0\0\0").unwrap();
}

#[tokio::test]
async fn web_apps_are_served_framable_while_the_shell_stays_locked() {
    let app = support::TestApp::new().await;
    user(&app, "one").await;
    stage_web_app(&app, "photocraft");
    assert_eq!(app.load_plugins().await, vec!["photocraft".to_string()]);

    let router = shiny::api::build_router(app.state.clone());

    // The framed document: SAMEORIGIN (not DENY), a policy that admits the
    // app's own inline boot script and its wasm, and a revalidated entry page.
    let response = router
        .clone()
        .oneshot(request("/plugins/photocraft/", None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-frame-options"], "SAMEORIGIN");
    assert_eq!(response.headers()["cache-control"], "no-cache");
    let csp = response.headers()["content-security-policy"]
        .to_str()
        .unwrap()
        .to_string();
    assert!(csp.contains("frame-ancestors 'self'"), "{csp}");
    assert!(csp.contains("'wasm-unsafe-eval'"), "{csp}");
    assert!(csp.contains("'unsafe-inline'"), "{csp}");
    let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
    assert!(String::from_utf8_lossy(&body).contains("PhotoCraft"));

    // Content-hashed assets are immutable: a 30 MB wasm must not come back on
    // every shell reload.
    let response = router
        .clone()
        .oneshot(request("/plugins/photocraft/app_web_bg.wasm", None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()["cache-control"],
        "public, max-age=31536000, immutable"
    );
    assert_eq!(
        response.headers()["content-type"],
        "application/wasm",
        "browsers refuse to stream-compile a wasm served as anything else"
    );

    // The shell keeps its own protection — the frames it may create are its
    // business, being framed by anyone else is not.
    let response = router
        .clone()
        .oneshot(request("/api/health/live", None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-frame-options"], "DENY");
    assert!(response.headers()["content-security-policy"]
        .to_str()
        .unwrap()
        .contains("frame-ancestors 'none'"));

    // The API tells the shell which surface to mount for this plugin.
    let body = json_body(
        router
            .oneshot(request("/api/plugins", Some("one")))
            .await
            .unwrap(),
    )
    .await;
    let entry = body["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["name"] == "photocraft")
        .expect("the web app is listed")
        .clone();
    assert_eq!(entry["web_app"], true);
    assert_eq!(entry["surface"], false, "a web app ships no plugin.js");
    assert_eq!(entry["version"], "0.10.0");
    assert_eq!(entry["category"], "Craft");
}

/// The store is an authenticated surface like the rest of the plugin API.
#[tokio::test]
async fn the_craft_store_requires_authentication() {
    let app = support::TestApp::new().await;
    user(&app, "one").await;
    let router = shiny::api::build_router(app.state.clone());
    for (method, path) in [
        ("GET", "/api/plugins/store"),
        ("POST", "/api/plugins/store/refresh"),
        ("POST", "/api/plugins/store/install"),
    ] {
        let mut req = request(path, None);
        *req.method_mut() = method.parse().unwrap();
        assert_eq!(
            router.clone().oneshot(req).await.unwrap().status(),
            StatusCode::UNAUTHORIZED,
            "{method} {path}"
        );
    }
}

/// A web-app plugin without an entry document must fail the install instead of
/// opening an empty window.
#[tokio::test]
async fn a_web_app_needs_an_entry_document() {
    let app = support::TestApp::new().await;
    let dir = Path::new(&app.state.config.plugins_dir).join("brokenapp");
    std::fs::create_dir_all(dir.join("web")).unwrap();
    std::fs::write(
        dir.join("plugin.toml"),
        "name = \"brokenapp\"\nversion = \"1.0.0\"\napi_level = 1\nkind = \"web\"\n",
    )
    .unwrap();
    let error = app
        .state
        .plugins
        .register_dir_static(&dir, app.state.plugin_ctx())
        .await
        .expect_err("a web app with no index.html cannot load");
    assert!(
        error.to_string().contains("index.html"),
        "the error names what is missing: {error}"
    );
}
