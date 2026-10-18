//! Native plugin surface captured from the pre-refactor libraries. Normal
//! tests compare it; the explicit ignored capture test is a maintenance tool.
mod support;
use serde_json::{json, Value};

async fn snapshot() -> Value {
    let app = support::TestApp::new().await;
    app.stage_all_plugins();
    app.load_plugins().await;
    let tools = app.state.plugins.tools();
    let keys = tools.list();
    let contributions = app.state.plugins.inner.contribs.read();
    let mut plugins = serde_json::Map::new();
    for contribution in contributions.iter() {
        let name = &contribution.manifest.name;
        let mut names: Vec<_> = keys
            .iter()
            .filter(|key| tools.owner_of(key) == *name)
            .cloned()
            .collect();
        names.sort();
        let mut routes: Vec<_> = contribution
            .routes
            .iter()
            .map(|(route, _)| {
                json!({
                    "path": route.path, "method": format!("{:?}", route.method), "auth": route.auth,
                })
            })
            .collect();
        routes.sort_by_key(|route| route.to_string());
        plugins.insert(
            name.clone(),
            json!({ "api_level": contribution.manifest.api_level,
            "tools": names, "routes": routes }),
        );
    }
    Value::Object(plugins)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_plugin_tools_and_routes_preserve_their_contract() {
    let expected: Value =
        serde_json::from_str(include_str!("fixtures/plugin-contracts.json")).unwrap();
    assert_eq!(snapshot().await, expected);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "Explicit baseline capture only; review intentional public contract changes"]
async fn capture_plugin_contracts() {
    let value = snapshot().await;
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/plugin-contracts.json");
    std::fs::write(path, serde_json::to_string_pretty(&value).unwrap() + "\n").unwrap();
}
