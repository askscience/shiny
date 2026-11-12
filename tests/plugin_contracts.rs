//! Native plugin surface captured from the pre-refactor libraries. The normal
//! test fails when a tool or route disappears or changes and records new ones
//! by itself (outside CI); the explicit ignored capture test rewrites the
//! baseline after an intentional removal.
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

fn fixture_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/plugin-contracts.json")
}

fn write_fixture(value: &Value) {
    std::fs::write(
        fixture_path(),
        serde_json::to_string_pretty(value).unwrap() + "\n",
    )
    .unwrap();
}

/// What the baseline promises and the current plugins no longer provide: a
/// plugin, tool or route that disappeared, was renamed, or changed method,
/// auth or API level. Additions are not listed: they break nobody.
fn broken_promises(expected: &Value, actual: &Value) -> Vec<String> {
    let mut broken = Vec::new();
    let Some(expected) = expected.as_object() else {
        return broken;
    };
    for (plugin, promised) in expected {
        let Some(current) = actual.get(plugin) else {
            broken.push(format!("plugin `{plugin}` is gone"));
            continue;
        };
        if promised["api_level"] != current["api_level"] {
            broken.push(format!(
                "plugin `{plugin}`: api_level {} became {}",
                promised["api_level"], current["api_level"]
            ));
        }
        for list in ["tools", "routes"] {
            let now = current[list].as_array().cloned().unwrap_or_default();
            for item in promised[list].as_array().into_iter().flatten() {
                if !now.contains(item) {
                    broken.push(format!("plugin `{plugin}`: {list} entry {item} is gone"));
                }
            }
        }
    }
    broken
}

/// Removing or changing a tool or route fails; adding one passes. Outside CI
/// the baseline then records the additions by itself, so new tools and routes
/// never need a manual recapture. An intentional removal still does (see
/// `capture_plugin_contracts`).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_plugin_tools_and_routes_preserve_their_contract() {
    let expected: Value =
        serde_json::from_str(include_str!("fixtures/plugin-contracts.json")).unwrap();
    let actual = snapshot().await;
    let broken = broken_promises(&expected, &actual);
    assert!(
        broken.is_empty(),
        "native plugin contract broken:\n  {}\nIf this is intentional, recapture the baseline:\n  \
         cargo test -p shiny --test plugin_contracts capture_plugin_contracts -- --ignored",
        broken.join("\n  ")
    );
    if actual != expected && std::env::var_os("CI").is_none() {
        write_fixture(&actual);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "Explicit baseline capture only; review intentional public contract changes"]
async fn capture_plugin_contracts() {
    write_fixture(&snapshot().await);
}

#[test]
fn additions_pass_and_removals_fail() {
    let base = json!({ "a": { "api_level": 1, "tools": ["x"], "routes": [{ "path": "/p" }] } });
    let added = json!({
        "a": { "api_level": 1, "tools": ["x", "y"], "routes": [{ "path": "/p" }, { "path": "/q" }] },
        "b": { "api_level": 1, "tools": [], "routes": [] },
    });
    assert!(broken_promises(&base, &added).is_empty());
    let removed = json!({ "a": { "api_level": 1, "tools": [], "routes": [{ "path": "/p2" }] } });
    assert_eq!(broken_promises(&base, &removed).len(), 2);
    assert_eq!(broken_promises(&base, &json!({})).len(), 1);
}
