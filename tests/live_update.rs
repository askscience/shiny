//! Updates that take effect while the server runs: the version endpoint the
//! page compares, the rescan that loads changed plugins again, and the swap
//! request that hands the port to a new binary.
mod support;

use axum::{
    body::{to_bytes, Body},
    extract::{ConnectInfo, Request},
    http::{Method, StatusCode},
};
use serde_json::Value;
use std::path::Path;
use tower::ServiceExt;

fn request(method: Method, path: &str, token: Option<&str>, remote: bool) -> Request {
    let mut builder = Request::builder().method(method).uri(path);
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    if remote {
        builder = builder.header("x-shiny-remote", "1");
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

/// A `kind = "web"` plugin: no library to build, so the fixture is a few files.
fn stage_web_app(dir: &Path, name: &str, version: &str) {
    std::fs::create_dir_all(dir.join("web")).unwrap();
    std::fs::write(
        dir.join("plugin.toml"),
        format!(
            "name = \"{name}\"\nversion = \"{version}\"\napi_level = 1\nkind = \"web\"\n\
             description = \"Fixture\"\nweb_dir = \"web\"\n"
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join("web/index.html"),
        "<!doctype html><title>fixture</title>",
    )
    .unwrap();
}

/// Make a later write land on a different mtime even on coarse filesystems.
fn bump(path: &Path, bytes: &[u8]) {
    std::fs::write(path, bytes).unwrap();
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(2);
    let _ = std::fs::File::options()
        .write(true)
        .open(path)
        .and_then(|f| f.set_modified(later));
}

#[tokio::test]
async fn the_version_endpoint_names_the_build_and_every_plugin() {
    let app = support::TestApp::new().await;
    user(&app, "u1").await;
    let plugins = Path::new(&app.state.config.plugins_dir);
    stage_web_app(&plugins.join("fixtureapp"), "fixtureapp", "1.0.0");
    app.load_plugins().await;
    let router = shiny::api::build_router(app.state.clone());

    let response = router
        .clone()
        .oneshot(request(
            Method::GET,
            "/api/system/version",
            Some("u1"),
            false,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["data"]["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(body["data"]["build"].as_str().unwrap().len(), 16);
    assert_eq!(body["data"]["web"].as_str().unwrap().len(), 16);
    let digest = body["data"]["plugins"]["fixtureapp"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(digest.len(), 16);

    // The plugin list carries the same fingerprint for the window loader.
    let response = router
        .clone()
        .oneshot(request(Method::GET, "/api/plugins", Some("u1"), false))
        .await
        .unwrap();
    let list = json_body(response).await;
    let entry = list["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "fixtureapp")
        .expect("fixture listed");
    assert_eq!(entry["digest"], digest);

    // Without a session the endpoint is closed like the rest of the API.
    let response = router
        .oneshot(request(Method::GET, "/api/system/version", None, false))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_rescan_loads_what_changed_on_disk_and_only_that() {
    let app = support::TestApp::new().await;
    user(&app, "u1").await;
    let plugins = Path::new(&app.state.config.plugins_dir);
    let dir = plugins.join("fixtureapp");
    stage_web_app(&dir, "fixtureapp", "1.0.0");
    let router = shiny::api::build_router(app.state.clone());

    // Nothing loaded yet: the rescan finds the plugin.
    let response = router
        .clone()
        .oneshot(request(
            Method::POST,
            "/api/plugins/rescan",
            Some("u1"),
            false,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let report = json_body(response).await;
    assert_eq!(report["data"]["added"], serde_json::json!(["fixtureapp"]));
    assert_eq!(report["data"]["updated"], serde_json::json!([]));
    let first = app.state.plugins.digest_for("fixtureapp").unwrap();

    // Unchanged files: nothing happens.
    let report = json_body(
        router
            .clone()
            .oneshot(request(
                Method::POST,
                "/api/plugins/rescan",
                Some("u1"),
                false,
            ))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(report["data"]["added"], serde_json::json!([]));
    assert_eq!(report["data"]["updated"], serde_json::json!([]));

    // A web-only change is the page's business: the plugin is not reloaded,
    // and the fingerprint the page compares differs from the loaded one.
    bump(
        &dir.join("web/index.html"),
        b"<!doctype html><title>fixture v2</title>",
    );
    let report = json_body(
        router
            .clone()
            .oneshot(request(
                Method::POST,
                "/api/plugins/rescan",
                Some("u1"),
                false,
            ))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(report["data"]["updated"], serde_json::json!([]));
    let on_disk = app.state.plugins.disk_plugins();
    assert_eq!(on_disk["fixtureapp"].digest.code, first.code);
    assert_ne!(on_disk["fixtureapp"].digest.all, first.all);
    // …and the fingerprint the page sees follows the files on disk at once,
    // with no reload on the server.
    let version = json_body(
        router
            .clone()
            .oneshot(request(
                Method::GET,
                "/api/system/version",
                Some("u1"),
                false,
            ))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(
        version["data"]["plugins"]["fixtureapp"],
        on_disk["fixtureapp"].digest.all
    );

    // A code change (the manifest) loads the plugin again.
    bump(
        &dir.join("plugin.toml"),
        b"name = \"fixtureapp\"\nversion = \"1.1.0\"\napi_level = 1\nkind = \"web\"\nweb_dir = \"web\"\n",
    );
    let report = json_body(
        router
            .clone()
            .oneshot(request(
                Method::POST,
                "/api/plugins/rescan",
                Some("u1"),
                false,
            ))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(report["data"]["updated"], serde_json::json!(["fixtureapp"]));
    let reloaded = app
        .state
        .plugins
        .list()
        .into_iter()
        .find(|m| m.name == "fixtureapp")
        .unwrap();
    assert_eq!(reloaded.version.to_string(), "1.1.0");
    assert_ne!(
        app.state.plugins.digest_for("fixtureapp").unwrap().code,
        first.code
    );

    // A broken manifest fails that plugin alone and keeps the previous load.
    bump(
        &dir.join("plugin.toml"),
        b"name = \"fixtureapp\"\nversion = \"not a version\"\n",
    );
    let report = json_body(
        router
            .clone()
            .oneshot(request(
                Method::POST,
                "/api/plugins/rescan",
                Some("u1"),
                false,
            ))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(report["data"]["updated"], serde_json::json!([]));
    assert_eq!(report["data"]["failed"][0][0], "fixtureapp");
    assert!(app.state.plugins.is_installed("fixtureapp"));
}

#[tokio::test]
async fn a_user_copy_shadows_the_system_plugin_and_is_reported() {
    let app = support::TestApp::new().await;
    let system = app.root.path().join("system-plugins");
    let user_dir = app.root.path().join("user-plugins");
    stage_web_app(&system.join("shared"), "shared", "1.0.0");
    stage_web_app(&system.join("onlysystem"), "onlysystem", "1.0.0");
    stage_web_app(&user_dir.join("shared"), "shared", "2.0.0");
    let manager = shiny::plugins::manager::PluginManager::with_system_dir(
        user_dir.clone(),
        Some(system),
        app.state.pool.clone(),
    );
    let disk = manager.disk_plugins();
    assert!(disk["shared"].overrides_system);
    assert!(disk["shared"].dir.starts_with(&user_dir));
    assert!(!disk["onlysystem"].overrides_system);

    let report = manager.rescan(app.state.plugin_ctx()).await;
    assert_eq!(report.overridden, vec!["shared".to_string()]);
    let mut added = report.added.clone();
    added.sort();
    assert_eq!(added, vec!["onlysystem".to_string(), "shared".to_string()]);
    let shared = manager
        .list()
        .into_iter()
        .find(|m| m.name == "shared")
        .unwrap();
    assert_eq!(shared.version.to_string(), "2.0.0", "the user copy wins");
}

#[tokio::test]
async fn the_swap_is_asked_once_and_only_from_this_machine() {
    let app = support::TestApp::new().await;
    user(&app, "u1").await;
    let router = shiny::api::build_router(app.state.clone());

    let response = router
        .clone()
        .oneshot(request(
            Method::POST,
            "/api/system/reexec",
            Some("u1"),
            true,
        ))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "a remote client cannot swap the server"
    );
    assert!(!app.state.reexec.requested());

    let response = router
        .clone()
        .oneshot(request(
            Method::POST,
            "/api/system/reexec",
            Some("u1"),
            false,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(json_body(response).await["data"]["status"], "scheduled");
    assert!(app.state.reexec.requested());

    let response = router
        .oneshot(request(
            Method::POST,
            "/api/system/reexec",
            Some("u1"),
            false,
        ))
        .await
        .unwrap();
    assert_eq!(json_body(response).await["data"]["status"], "pending");
}
