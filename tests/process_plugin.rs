//! A `runtime = "process"` plugin is an executable the server talks to over a
//! socket. Seen from the rest of the server it must behave like a library
//! plugin: same tools, same routes, same lifecycle. On top of that a crash
//! costs only the plugin, which comes back on the next request.
mod support;

use axum::{
    body::{to_bytes, Body},
    extract::{ConnectInfo, Request},
    http::StatusCode,
};
use serde_json::{json, Value};
use shiny::plugins::loader::PluginRuntime;
use shiny_plugin_sdk::context::AgentContext;
use std::time::Duration;
use tower::ServiceExt;

fn post(path: &str, token: &str, body: Value) -> Request {
    let mut request = Request::builder()
        .method("POST")
        .uri(path)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    request.extensions_mut().insert(ConnectInfo(
        "127.0.0.1:1234".parse::<std::net::SocketAddr>().unwrap(),
    ));
    request
}

async fn user(app: &support::TestApp, id: &str) {
    sqlx::query("INSERT INTO travelers (id, name, email, password_hash, auth_token) VALUES (?, ?, ?, '!', ?)")
        .bind(id).bind(id).bind(format!("{id}@test")).bind(id).execute(&app.state.pool).await.unwrap();
}

fn agent_context() -> AgentContext {
    serde_json::from_value(
        json!({ "lat": null, "lon": null, "heading": null, "lang": "en", "ollama_model": null }),
    )
    .unwrap()
}

fn alive(pid: u32) -> bool {
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

#[tokio::test]
async fn process_plugins_serve_tools_and_routes_and_survive_a_crash() {
    let app = support::TestApp::new().await;
    user(&app, "ada").await;
    app.stage_plugin("hello");
    app.stage_plugin("calculator");
    let mut loaded = app.load_plugins().await;
    loaded.sort();
    assert_eq!(loaded, vec!["calculator".to_string(), "hello".to_string()]);
    let plugins = &app.state.plugins;
    assert_eq!(
        plugins.loader().runtime_for("hello"),
        PluginRuntime::Process
    );
    for name in ["hello", "calculator"] {
        plugins.loader().call_on_load(name).await;
        plugins.set_enabled_for("ada", name, true).await.unwrap();
    }

    // A tool, with the plugin's own `humanize` for the result.
    let tools = plugins.tools();
    let outcome = tools
        .invoke(
            "hello",
            &app.state.plugin_ctx(),
            "ada",
            "ada",
            &json!({ "name": "Ada" }),
            &agent_context(),
            None,
        )
        .await
        .unwrap();
    assert_eq!(outcome.data["reply"], "Hello, Ada!");
    assert_eq!(
        tools.humanize("hello", &outcome.result, &outcome.data),
        "Said hello to Ada"
    );
    assert_eq!(tools.step_label("hello"), "Saying hello…");

    // A route, forwarded with the caller's identity; the plugin writes the
    // shared database.
    let router = shiny::api::build_router(app.state.clone());
    let response = router
        .clone()
        .oneshot(post(
            "/api/calculator/eval",
            "ada",
            json!({ "expression": "6*7" }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1 << 20).await.unwrap()).unwrap();
    assert_eq!(body["data"]["result_text"], "42");
    let rows: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM calculator_history WHERE user_id = 'ada'")
            .fetch_one(&app.state.pool)
            .await
            .unwrap();
    assert_eq!(rows.0, 1);

    // An error inside the plugin comes back as the same HTTP error.
    let response = router
        .clone()
        .oneshot(post(
            "/api/calculator/eval",
            "ada",
            json!({ "expression": "" }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // Kill the process: the server stays up and the next request restarts it.
    let supervisor = plugins.loader().supervisor_for("calculator").unwrap();
    let first = supervisor.pid().unwrap();
    unsafe { libc::kill(first as libc::pid_t, libc::SIGKILL) };
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while alive(first) && std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let response = router
        .clone()
        .oneshot(post(
            "/api/calculator/eval",
            "ada",
            json!({ "expression": "1+1" }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let second = supervisor.pid().unwrap();
    assert_ne!(first, second);

    // Uninstall stops the process.
    let hello_pid = plugins
        .loader()
        .supervisor_for("hello")
        .unwrap()
        .pid()
        .unwrap();
    assert!(plugins.uninstall("hello").await);
    assert!(
        !alive(hello_pid) || {
            tokio::time::sleep(Duration::from_millis(500)).await;
            !alive(hello_pid)
        }
    );
}
