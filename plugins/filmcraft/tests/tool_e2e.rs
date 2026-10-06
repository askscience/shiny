//! End-to-end test of the plugin's *tools*, without the LLM.
//!
//! The agent path is `ToolRegistry::invoke` → the plugin's `Tool::invoke`, and
//! Ollama is not always running, so these tests drive the tool half directly: the
//! same `ToolRequest` the core would build, on the same single-threaded plugin
//! runtime the core hands work to. That covers the part this plugin is
//! responsible for — argument parsing, the wait budget, the error the model
//! reads, and the outcome shape.
//!
//! What it deliberately does not cover is core's own routing: that the tools are
//! registered, advertised and activated. `docs/README.md` covers how to check
//! that part against a running server.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use shiny_plugin_sdk::{
    context::AgentContext,
    services::PluginCtx,
    tools::{ParamHelpers, Tool, ToolRequest},
};

use shiny_filmcraft_plugin::tools::{batch, browser};

/// A minimal `AgentContext`; nothing in these tools reads it.
fn agent_ctx() -> AgentContext {
    AgentContext {
        lat: None,
        lon: None,
        heading: None,
        lang: "en".into(),
        ollama_model: None,
        workspaces_enabled: true,
    }
}

/// Run a tool on a dedicated single-threaded runtime, like `rt::bridge` does.
fn run_tool(tool: &dyn Tool, user: &str, params: Value) -> Result<Value, String> {
    let ctx = PluginCtx::with_manifest(Arc::new(shiny_plugin_sdk::manifest::Manifest {
        name: "filmcraft".into(),
        version: semver::Version::new(0, 1, 0),
        api_level: 1,
        entry_symbol: "shiny_plugin_entry".into(),
        target_triple: None,
        description: None,
        author: None,
        summary: None,
        migrations_dir: "migrations".into(),
        skills_dir: "skills".into(),
        web_dir: "web".into(),
        signature: None,
    }));

    let req = ToolRequest {
        user_id: user,
        traveler_id: user,
        params: &params,
        ctx: &agent_ctx(),
        os_home: None,
    };

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    rt.block_on(tool.invoke(&ctx, req))
        .map(|outcome| {
            json!({ "result": outcome.result, "data": outcome.data })
        })
        .map_err(|e| e.to_string())
}

/* ── The window tools ─────────────────────────────────────────────────── */

#[test]
fn film_command_needs_a_command_id() {
    let tool = browser::FilmCommand;
    let err = run_tool(&tool, "tool-user", json!({})).unwrap_err();
    assert!(err.contains("command"), "{err}");
}

#[test]
fn film_command_rejects_non_object_params() {
    let tool = browser::FilmCommand;
    let err = run_tool(&tool, "tool-user", json!({ "command": "project.inspect", "params": 7 })).unwrap_err();
    assert!(err.contains("must be an object"), "{err}");
}

#[test]
fn film_command_without_a_window_tells_the_model_what_to_do() {
    // No window is polling for this user, so the call must fail — and the
    // message has to be actionable, because the model reads it.
    let tool = browser::FilmCommand;
    let err = run_tool(&tool, "no-window-user", json!({ "command": "project.inspect" })).unwrap_err();
    assert!(err.contains("show_plugin"), "the error should name the action: {err}");
    assert!(err.contains("filmcraft"), "the error should name the plugin: {err}");
}

#[test]
fn film_command_returns_the_window_answer() {
    let user = "tool-user-with-window";
    shiny_filmcraft_plugin::relay::heartbeat(user);
    let tool = browser::FilmCommand;

    // Stand in for the window: drain the queue and answer.
    let window = std::thread::spawn(move || {
        for _ in 0..300 {
            if let Some(req) = shiny_filmcraft_plugin::relay::take(user) {
                let id = req["id"].as_u64().expect("an id");
                let command = req["params"]["command"].as_str().unwrap_or_default().to_string();
                assert_eq!(req["method"], "engine.execute", "engine commands go through one method");
                shiny_filmcraft_plugin::relay::complete(
                    user,
                    id,
                    Ok(json!({ "name": command, "sequences": 2 })),
                );
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("the window never received the command");
    });

    let out = run_tool(&tool, user, json!({ "command": "project.inspect", "wait_ms": 5000 }))
        .expect("the tool succeeded");
    window.join().expect("the window thread finished");

    assert_eq!(out["result"], "ok");
    assert_eq!(out["data"]["command"], "project.inspect");
    assert_eq!(out["data"]["result"]["sequences"], 2, "the window's answer is passed through");
}

#[test]
fn film_command_surfaces_the_window_error() {
    let user = "tool-user-window-error";
    shiny_filmcraft_plugin::relay::heartbeat(user);
    let tool = browser::FilmCommand;

    let window = std::thread::spawn(move || {
        for _ in 0..300 {
            if let Some(req) = shiny_filmcraft_plugin::relay::take(user) {
                let id = req["id"].as_u64().expect("an id");
                shiny_filmcraft_plugin::relay::complete(user, id, Err("no sequence is open".into()));
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("no command arrived");
    });

    let err = run_tool(&tool, user, json!({ "command": "project.inspect", "wait_ms": 5000 }))
        .unwrap_err();
    window.join().expect("the window thread finished");
    assert!(err.contains("no sequence is open"), "{err}");
}

#[test]
fn film_commands_reports_a_closed_window_as_data_not_an_error() {
    // The model should be told to open the window, not handed a failure.
    let tool = browser::FilmCommands;
    let out = run_tool(&tool, "closed-window-user", json!({})).expect("the tool succeeded");
    assert_eq!(out["result"], "ok");
    assert_eq!(out["data"]["window"], false);
    assert!(
        out["data"]["hint"].as_str().unwrap_or_default().contains("show_plugin"),
        "the hint should name the action, got {}",
        out["data"]["hint"]
    );
}

#[test]
fn film_commands_lists_the_registry_from_a_live_window() {
    let user = "tool-user-commands";
    shiny_filmcraft_plugin::relay::heartbeat(user);
    let tool = browser::FilmCommands;

    let window = std::thread::spawn(move || {
        for _ in 0..300 {
            if let Some(req) = shiny_filmcraft_plugin::relay::take(user) {
                assert_eq!(req["method"], "engine.commands", "the list is a control method");
                let id = req["id"].as_u64().expect("an id");
                shiny_filmcraft_plugin::relay::complete(
                    user,
                    id,
                    Ok(json!([{ "id": "project.inspect" }, { "id": "timeline.move" }])),
                );
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("no request arrived");
    });

    let out = run_tool(&tool, user, json!({})).expect("the tool succeeded");
    window.join().expect("the window thread finished");
    assert_eq!(out["data"]["window"], true);
    assert_eq!(out["data"]["commands"][0]["id"], "project.inspect");
}

/* ── The headless tools ───────────────────────────────────────────────── */

#[test]
fn film_headless_needs_a_command_id() {
    let tool = batch::FilmHeadless;
    let err = run_tool(&tool, "hl-user", json!({})).unwrap_err();
    assert!(err.contains("command"), "{err}");
}

#[test]
fn film_headless_rejects_non_object_params() {
    let tool = batch::FilmHeadless;
    let err = run_tool(&tool, "hl-user", json!({ "command": "project.inspect", "params": "x" })).unwrap_err();
    assert!(err.contains("must be an object"), "{err}");
}

#[test]
fn film_headless_reports_an_engine_error_as_a_tool_error() {
    let tool = batch::FilmHeadless;
    let err = run_tool(
        &tool,
        "hl-user",
        json!({ "command": "definitely.not.a.command", "timeout_s": 15 }),
    )
    .unwrap_err();
    assert!(err.contains("definitely.not.a.command"), "{err}");
}

#[test]
fn film_export_requires_a_project_and_an_output() {
    let tool = batch::FilmExport;
    let err = run_tool(&tool, "exp-user", json!({})).unwrap_err();
    assert!(err.contains("project"), "{err}");

    let err = run_tool(&tool, "exp-user", json!({ "project": "/tmp/x.fcproj" })).unwrap_err();
    assert!(err.contains("output"), "{err}");
}

#[test]
fn film_export_rejects_non_object_settings() {
    let tool = batch::FilmExport;
    let err = run_tool(
        &tool,
        "exp-user",
        json!({ "project": "/tmp/x.fcproj", "output": "/tmp/out.mp4", "settings": 3 }),
    )
    .unwrap_err();
    assert!(err.contains("must be an object"), "{err}");
}

#[test]
fn film_export_cancel_needs_a_job_id() {
    let tool = batch::FilmExportCancel;
    let err = run_tool(&tool, "exp-user", json!({})).unwrap_err();
    assert!(err.contains("job"), "{err}");
}

#[test]
fn film_export_status_on_an_empty_session_is_an_empty_list() {
    let tool = batch::FilmExportStatus;
    let out = run_tool(&tool, "status-user", json!({})).expect("the tool succeeded");
    assert_eq!(out["result"], "ok");
    assert!(
        out["data"]["jobs"].is_array(),
        "without a job id the tool reports every job, got {}",
        out["data"]
    );
}

#[test]
fn film_export_status_for_an_unknown_job_is_a_hint_not_an_error() {
    let tool = batch::FilmExportStatus;
    let out = run_tool(&tool, "status-user", json!({ "job": 999 })).expect("the tool succeeded");
    assert_eq!(out["result"], "ok");
    assert_eq!(out["data"]["job"], Value::Null);
    assert!(
        out["data"]["hint"].as_str().unwrap_or_default().contains("999"),
        "the hint should name the job, got {}",
        out["data"]["hint"]
    );
}