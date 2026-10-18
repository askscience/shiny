//! Focus mode has to let the focused plugin's own tools through — for EVERY
//! plugin, not just the one that happened to be tested by hand. This loads the
//! explicitly built cdylibs in a temporary install tree and checks the same registry the agent
//! dispatches against:
//!
//!   1. every registered tool names the plugin that owns it (a tool with no
//!      owner is read as a core action, and focus mode would then refuse it for
//!      every plugin — exactly how `word` broke);
//!   2. with plugin P focused, every tool owned by P passes the focus gate, and
//!      a tool owned by another plugin does not;
//!   3. through the real dispatcher: `doc_create` reaches the word plugin while
//!      word is focused, and `calc_list` is refused while a different plugin is.
//!
//! Run with:
//!     cargo test --test focus_mode_scope -- --nocapture

mod support;
use serde_json::json;
use shiny::models::Traveler;
use shiny::services::agent_tools::{execute_action, focus_blocks_plugin};
use shiny_plugin_sdk::context::AgentContext;

fn ctx(focused: &[&str]) -> AgentContext {
    AgentContext {
        lat: None,
        lon: None,
        heading: None,
        lang: "en".into(),
        ollama_model: None,
        workspaces_enabled: true,
        focused: focused.iter().map(|s| s.to_string()).collect(),
    }
}

fn traveler() -> Traveler {
    Traveler {
        id: "focus-scope-test".into(),
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

/// A focus-mode refusal, as opposed to any other failure.
fn refused_by_focus_mode(out: &shiny::services::agent_tools::ActionOutcome) -> bool {
    out.result == "unavailable"
        && out.data.get("focus_mode").and_then(|v| v.as_bool()) == Some(true)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_plugins_own_tools_survive_focus_mode() {
    let app = support::TestApp::new().await;
    app.stage_all_plugins();
    let state = &app.state;
    let installed = app.load_plugins().await;

    let tools = state.plugins.tools();
    let keys = tools.list();
    assert!(
        !keys.is_empty(),
        "no plugin tools registered — did nothing load?"
    );
    println!("{} plugins, {} tools", installed.len(), keys.len());

    // 1. Every registered tool must name its owner. These are all plugin tools
    //    (core actions live in the dispatcher's own match), so an empty owner
    //    means focus mode would treat it as a core action and refuse it for
    //    every plugin — the bug that made Word unusable while focused.
    let unowned: Vec<&String> = keys
        .iter()
        .filter(|k| tools.owner_of(k).is_empty())
        .collect();
    assert!(
        unowned.is_empty(),
        "plugin tools with no owner: {unowned:?}"
    );

    // 2. For each plugin: focused ⇒ its own tools pass, another plugin's don't.
    let mut checked = 0;
    for name in &installed {
        let own: Vec<&String> = keys.iter().filter(|k| tools.owner_of(k) == *name).collect();
        if own.is_empty() {
            continue;
        }
        let on = ctx(&[name.as_str()]);
        for key in &own {
            assert!(
                !focus_blocks_plugin(&on, name),
                "{name}: own tool {key} was blocked while {name} is focused"
            );
            checked += 1;
        }
        // Any tool belonging to someone else must be refused.
        if let Some(other) = keys.iter().find(|k| tools.owner_of(k) != *name) {
            assert!(
                focus_blocks_plugin(&on, &tools.owner_of(other)),
                "{name}: another plugin's tool {other} was allowed"
            );
        }
        // With nothing focused, everything is available again.
        assert!(
            !focus_blocks_plugin(&ctx(&[]), name),
            "{name}: blocked with focus off"
        );
    }
    println!("{checked} tools checked across {} plugins", installed.len());
    assert!(
        checked > 20,
        "expected the plugin sets to be substantial, checked {checked}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_focused_plugins_tool_reaches_the_plugin_and_another_does_not() {
    let app = support::TestApp::new().await;
    app.stage_plugin("word");
    app.stage_plugin("calc");
    let state = &app.state;
    let installed = app.load_plugins().await;
    assert!(
        installed.iter().any(|n| n == "word"),
        "word plugin not loaded: {installed:?}"
    );
    assert!(
        installed.iter().any(|n| n == "calc"),
        "calc plugin not loaded: {installed:?}"
    );

    let tools = state.plugins.tools();
    assert!(
        tools.list().iter().any(|t| t == "doc_create"),
        "word's real tool is doc_create"
    );
    assert!(
        tools.list().iter().any(|t| t == "calc_list"),
        "calc's real tool is calc_list"
    );
    assert_eq!(tools.owner_of("doc_create"), "word");
    assert_eq!(tools.owner_of("calc_list"), "calc");

    let traveler = traveler();
    // The state tables reference the account, so it has to exist first.
    sqlx::query("INSERT INTO travelers (id, name, email, password_hash) VALUES (?1, ?2, ?3, ?4)")
        .bind(&traveler.id)
        .bind(&traveler.name)
        .bind(&traveler.email)
        .bind(&traveler.password_hash)
        .execute(&state.pool)
        .await
        .expect("insert traveller");
    // A fresh install is an opt-in session: only plugins the user enabled are
    // active, and an inactive plugin's tools are refused before focus mode even
    // gets a say (that gate is older and stays).
    for name in ["word", "calc"] {
        state
            .plugins
            .set_enabled_for(&traveler.id, name, true)
            .await
            .expect("enable for the test user");
    }

    // Focused on word: word's own tool must reach the plugin.
    let word_ctx = ctx(&["word"]);
    let out = execute_action(
        state,
        &traveler,
        &word_ctx,
        "doc_create",
        &json!({ "title": "Focus probe" }),
    )
    .await
    .expect("doc_create dispatches");
    assert!(
        !refused_by_focus_mode(&out),
        "word's own tool was refused while word is focused: {:?}",
        out.data
    );

    // ... while another plugin's tool is refused, before it runs.
    let blocked = execute_action(state, &traveler, &word_ctx, "calc_list", &json!({}))
        .await
        .expect("calc_list dispatches");
    assert!(
        refused_by_focus_mode(&blocked),
        "another plugin's tool must be refused while word is focused: {:?}",
        blocked.data
    );

    // A name that does not exist is NOT focus mode's fault — it must report the
    // unknown action, so the model fixes the name instead of blaming the focus.
    let unknown = execute_action(
        state,
        &traveler,
        &word_ctx,
        "word_create_document",
        &json!({}),
    )
    .await;
    match unknown {
        Ok(out) => assert!(
            !refused_by_focus_mode(&out),
            "an unknown tool must not be reported as a focus refusal: {:?}",
            out.data
        ),
        Err(e) => assert!(e.to_string().contains("Unknown action"), "{e}"),
    }

    // With nothing focused, the other plugin works again.
    let out = execute_action(state, &traveler, &ctx(&[]), "calc_list", &json!({}))
        .await
        .expect("calc_list dispatches");
    assert!(
        !refused_by_focus_mode(&out),
        "focus off must not block anything"
    );
}
