//! The Craft MCP host end to end: a server follows the app's **window**, and
//! focus mode narrows the surface to the focused apps.
//!
//! The CLI is the app's real contract (`<app>-cli mcp`, JSON-RPC over stdio), so
//! the fixture speaks the same protocol on the same wire while the host, the
//! tool registry and the dispatcher under test are the real ones. What this
//! proves, in order: an open app is hosted and its tools reach the model, a real
//! `execute_action` call lands in the app's process with the arguments, focus
//! mode keeps only the focused apps, closing a window stops the server, and
//! turning AI control off stops it too.
//!
//! Run with:
//!     cargo test --test craft_mcp_host -- --nocapture

mod support;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::json;
use shiny::models::Traveler;
use shiny::services::agent_tools::{execute_action, AgentContext};

/// The app CLI as the host starts it (`mcp --automation-read-root …`), speaking
/// MCP over stdio. Every request is appended to `<cli>.requests` so the test can
/// check what actually reached the app's process.
const FAKE_CLI: &str = r#"#!/bin/sh
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$0.requests"
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"initialize"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":"2025-06-18","capabilities":{},"serverInfo":{"name":"fake-app","version":"1"}}}\n' "$id"
      ;;
    *'"method":"tools/list"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[{"name":"command_list","description":"List the app commands.","inputSchema":{"type":"object","properties":{}}},{"name":"command_run","description":"Run one command by id.","inputSchema":{"type":"object","properties":{"id":{"type":"string"}},"required":["id"]}}]}}\n' "$id"
      ;;
    *'"method":"tools/call"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"content":[{"type":"text","text":"command_list ok"}]}}\n' "$id"
      ;;
  esac
done
"#;

fn ctx() -> AgentContext {
    AgentContext {
        lat: None,
        lon: None,
        heading: None,
        lang: "en".into(),
        ollama_model: None,
        workspaces_enabled: true,
        focused: Vec::new(),
    }
}

fn traveler() -> Traveler {
    Traveler {
        id: "craft-mcp-test".into(),
        name: "Test".into(),
        email: "test@example.invalid".into(),
        password_hash: String::new(),
        auth_token: None,
        created_at: None,
        updated_at: None,
        username: None,
        avatar: None,
        is_admin: None,
        unix_user: None,
        unix_uid: None,
        unix_home: None,
        auth_source: None,
    }
}

/// A store-installed app: a `kind = "web"` plugin with no cdylib and no tools of
/// its own — the MCP host is what contributes them.
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
             web_dir = \"web\"\n\
             category = \"Craft\"\n"
        ),
    )
    .unwrap();
    std::fs::write(dir.join("web/index.html"), "<!doctype html>").unwrap();
}

/// Install the fake CLI where "AI control" puts the real one.
fn write_cli(app: &support::TestApp, id: &str) -> PathBuf {
    let path = shiny::plugins::craft_mcp::cli_path(Path::new(&app.state.config.plugins_dir), id);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, FAKE_CLI).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).unwrap();
    }
    path
}

/// One agent turn's worth of the host's view: these windows are open, these are
/// focused.
async fn sync(app: &support::TestApp, open: &[&str], focused: &[&str]) {
    let set = |names: &[&str]| names.iter().map(|name| name.to_string()).collect::<BTreeSet<_>>();
    let home = app.root.path().to_string_lossy().into_owned();
    app.state
        .craft_mcp
        .sync(
            &app.state.plugins,
            &app.state.craft_control,
            Path::new(&app.state.config.plugins_dir),
            &home,
            &set(open),
            &set(focused),
        )
        .await;
}

async fn running(app: &support::TestApp) -> Vec<String> {
    app.state.craft_mcp.running_apps().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_mcp_server_follows_the_open_window_and_focus_mode_narrows_it() {
    let app = support::TestApp::new().await;
    for name in ["photocraft", "wordcraft"] {
        stage_web_app(&app, name);
    }
    let installed = app.load_plugins().await;
    assert!(
        installed.iter().any(|name| name == "photocraft"),
        "the web apps did not load: {installed:?}"
    );

    let traveler = traveler();
    sqlx::query("INSERT INTO travelers (id, name, email, password_hash) VALUES (?1, ?2, ?3, ?4)")
        .bind(&traveler.id)
        .bind(&traveler.name)
        .bind(&traveler.email)
        .bind(&traveler.password_hash)
        .execute(&app.state.pool)
        .await
        .expect("insert traveller");
    for name in ["photocraft", "wordcraft"] {
        app.state
            .plugins
            .set_enabled_for(&traveler.id, name, true)
            .await
            .expect("enable the app for the test user");
    }

    let cli = write_cli(&app, "photocraft");
    write_cli(&app, "wordcraft");

    // No window open: no server, no tool, nothing for the model to read.
    sync(&app, &[], &[]).await;
    assert!(running(&app).await.is_empty(), "no window, no server");

    // PhotoCraft's window is open: its CLI is hosted, its tools are registered
    // under its own plugin id, and the model is shown them.
    sync(&app, &["photocraft"], &[]).await;
    assert_eq!(running(&app).await, vec!["photocraft".to_string()]);
    let tools = app.state.plugins.tools();
    assert_eq!(tools.owner_of("photocraft_command_list"), "photocraft");
    assert!(tools.has("photocraft_command_run"));
    assert!(
        !tools.has("wordcraft_command_list"),
        "an app with no window has no tools"
    );
    let active = app.state.plugins.session_active_set(&traveler.id).await;
    let docs = app.state.plugins.skills_markdown_for(&active);
    assert!(
        docs.contains("photocraft_command_list"),
        "the model must be shown the open app's tools"
    );

    // The tool works through the real dispatcher, and the call lands in the
    // app's own process with the arguments the model passed.
    let out = execute_action(
        &app.state,
        &traveler,
        &ctx(),
        "photocraft_command_run",
        &json!({ "id": "doc.open" }),
    )
    .await
    .expect("the MCP tool dispatches");
    assert_eq!(out.result, "ok", "{:?}", out.data);
    assert_eq!(out.owner.as_deref(), Some("photocraft"));
    let requests = std::fs::read_to_string(format!("{}.requests", cli.display())).unwrap();
    assert!(requests.contains("\"method\":\"tools/call\""), "{requests}");
    assert!(requests.contains("\"name\":\"command_run\""), "{requests}");
    assert!(requests.contains("\"id\":\"doc.open\""), "{requests}");

    // Focus mode on WordCraft, with both windows open: only the focused app is
    // served — the other app's server is stopped and its tools disappear.
    sync(&app, &["photocraft", "wordcraft"], &["wordcraft"]).await;
    assert_eq!(running(&app).await, vec!["wordcraft".to_string()]);
    assert!(!tools.has("photocraft_command_list"), "unfocused app's tools are gone");
    assert!(tools.has("wordcraft_command_list"));

    // Focus off: every open app serves again.
    sync(&app, &["photocraft", "wordcraft"], &[]).await;
    assert_eq!(
        running(&app).await,
        vec!["photocraft".to_string(), "wordcraft".to_string()]
    );

    // A window that is not a Craft app is ignored.
    sync(&app, &["calc", "photocraft"], &[]).await;
    assert_eq!(running(&app).await, vec!["photocraft".to_string()]);

    // Windows closed: the servers stop and the tools leave the registry.
    sync(&app, &[], &[]).await;
    assert!(running(&app).await.is_empty(), "closing the window stops the server");
    assert!(!tools.has("photocraft_command_list") && !tools.has("wordcraft_command_list"));
    assert!(
        !app.state
            .plugins
            .skills_markdown_for(&active)
            .contains("photocraft_command_list"),
        "a closed app must not be advertised to the model"
    );

    // AI control off (the CLI is removed) stops a running server even while the
    // window stays open.
    sync(&app, &["photocraft"], &[]).await;
    assert_eq!(running(&app).await, vec!["photocraft".to_string()]);
    std::fs::remove_file(&cli).expect("remove the fake CLI");
    sync(&app, &["photocraft"], &[]).await;
    assert!(
        running(&app).await.is_empty(),
        "removing the CLI must stop the app's server"
    );
}

/// Turning AI control off is a decision the app's own updates have to respect:
/// the CLI is removed and the opt-out recorded in its place, so a later install
/// or update does not silently pull the 60–124 MB bundle back.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn turning_ai_control_off_is_remembered() {
    let app = support::TestApp::new().await;
    let plugins_dir = PathBuf::from(&app.state.config.plugins_dir);
    write_cli(&app, "vectorcraft");
    assert!(
        shiny::plugins::craft_mcp::ai_control_allowed(&plugins_dir, "vectorcraft"),
        "AI control starts out on: installing the app brings its CLI"
    );

    // The recorded version is what tells an app update whether the CLI already
    // on disk is the one this release ships — no 60–124 MB for nothing, and a
    // newer release does re-install.
    std::fs::write(
        shiny::plugins::craft_mcp::version_path(&plugins_dir, "vectorcraft"),
        "0.13.1",
    )
    .expect("write the CLI version");
    assert!(shiny::plugins::craft_mcp::ai_control_current(
        &plugins_dir,
        "vectorcraft",
        "0.13.1"
    ));
    assert!(
        !shiny::plugins::craft_mcp::ai_control_current(&plugins_dir, "vectorcraft", "0.14.0"),
        "a newer release reinstalls the CLI"
    );

    app.state
        .store
        .remove_mcp(&app.state, "vectorcraft")
        .expect("turn AI control off");

    assert!(
        !shiny::plugins::craft_mcp::cli_path(&plugins_dir, "vectorcraft").is_file(),
        "the CLI is gone"
    );
    assert!(
        !shiny::plugins::craft_mcp::ai_control_allowed(&plugins_dir, "vectorcraft"),
        "and the choice is recorded for the next install/update"
    );
}
