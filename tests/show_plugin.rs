//! "Show me Word" opens Word: `show_plugin` activates an installed plugin
//! that is not active this session instead of refusing (the model then
//! claimed "the window is open" over the error).
mod support;
use serde_json::json;
use shiny::models::Traveler;
use shiny::services::agent_tools::execute_action;
use shiny_plugin_sdk::context::AgentContext;

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
        id: "show-plugin-test".into(),
        name: "Test".into(),
        email: "show@example.invalid".into(),
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn showing_an_inactive_plugin_activates_it() {
    let app = support::TestApp::new().await;
    app.stage_plugin("word");
    app.load_plugins().await;
    let state = &app.state;
    let traveler = traveler();
    sqlx::query("INSERT INTO travelers (id, name, email, password_hash) VALUES (?1, ?2, ?3, ?4)")
        .bind(&traveler.id)
        .bind(&traveler.name)
        .bind(&traveler.email)
        .bind(&traveler.password_hash)
        .execute(&state.pool)
        .await
        .unwrap();
    assert!(
        !state
            .plugins
            .session_active_plugin_enabled(&traveler.id, "word")
            .await
    );

    let out = execute_action(
        state,
        &traveler,
        &ctx(),
        "show_plugin",
        &json!({ "name": "word" }),
    )
    .await
    .expect("show_plugin runs");
    assert_eq!(out.result, "ok", "{:?}", out.data);
    assert_eq!(out.data["plugin"], "word");
    assert_eq!(out.data["activated"], true);
    assert!(
        state
            .plugins
            .session_active_plugin_enabled(&traveler.id, "word")
            .await
    );

    // Already active: shown, nothing to activate.
    let again = execute_action(
        state,
        &traveler,
        &ctx(),
        "show_plugin",
        &json!({ "name": "word" }),
    )
    .await
    .unwrap();
    assert_eq!(again.result, "ok");
    assert!(again.data.get("activated").is_none());

    // Not installed: still an error.
    let missing = execute_action(
        state,
        &traveler,
        &ctx(),
        "show_plugin",
        &json!({ "name": "nope" }),
    )
    .await
    .unwrap();
    assert_eq!(missing.result, "error");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_bare_sheet_and_its_filled_twin_are_one_spreadsheet() {
    let app = support::TestApp::new().await;
    app.stage_plugin("calc");
    app.load_plugins().await;
    let state = &app.state;
    let traveler = traveler();
    sqlx::query("INSERT INTO travelers (id, name, email, password_hash) VALUES (?1, ?2, ?3, ?4)")
        .bind(&traveler.id)
        .bind(&traveler.name)
        .bind(&traveler.email)
        .bind(&traveler.password_hash)
        .execute(&state.pool)
        .await
        .unwrap();
    state
        .plugins
        .set_enabled_for(&traveler.id, "calc", true)
        .await
        .unwrap();

    // What models do: the sheet first, then again with its cells.
    let bare = execute_action(
        state,
        &traveler,
        &ctx(),
        "calc_create",
        &json!({ "title": "Trip budget" }),
    )
    .await
    .unwrap();
    let filled = execute_action(
        state,
        &traveler,
        &ctx(),
        "calc_create",
        &json!({ "title": "Trip budget", "cells": { "A1": "Flights", "B1": "184" } }),
    )
    .await
    .unwrap();
    assert_eq!(bare.result, "ok");
    assert_eq!(filled.result, "ok");
    assert_eq!(
        filled.data["sheet_id"], bare.data["sheet_id"],
        "the empty sheet was filled, not duplicated"
    );

    let list = execute_action(state, &traveler, &ctx(), "calc_list", &json!({}))
        .await
        .unwrap();
    let text = list.data.to_string();
    assert_eq!(text.matches("Trip budget").count(), 1, "{text}");

    // A sheet that has cells is never overwritten by a new one of the same name.
    let another = execute_action(
        state,
        &traveler,
        &ctx(),
        "calc_create",
        &json!({ "title": "Trip budget", "cells": { "A1": "Hotel" } }),
    )
    .await
    .unwrap();
    assert_ne!(another.data["sheet_id"], bare.data["sheet_id"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn calendar_events_accept_the_shapes_models_send() {
    let app = support::TestApp::new().await;
    app.stage_plugin("calendar");
    app.load_plugins().await;
    let state = &app.state;
    let traveler = traveler();
    sqlx::query("INSERT INTO travelers (id, name, email, password_hash) VALUES (?1, ?2, ?3, ?4)")
        .bind(&traveler.id)
        .bind(&traveler.name)
        .bind(&traveler.email)
        .bind(&traveler.password_hash)
        .execute(&state.pool)
        .await
        .unwrap();
    state
        .plugins
        .set_enabled_for(&traveler.id, "calendar", true)
        .await
        .unwrap();
    for params in [
        json!({ "title": "Train", "start_time": "2026-11-20T08:15:00" }),
        json!({ "summary": "Train", "start": "2026-11-20T08:15:00Z", "end": "2026-11-20T10:15:00Z" }),
        json!({ "event": "Train", "date": "2026-11-20", "time": "8:15 am" }),
    ] {
        let out = execute_action(state, &traveler, &ctx(), "add_event", &params).await;
        let out = out.unwrap_or_else(|e| panic!("{params}: {e}"));
        assert_eq!(out.result, "ok", "{params}: {:?}", out.data);
        assert_eq!(out.data["date"], "2026-11-20", "{params}");
        assert_eq!(out.data["start_time"], "08:15", "{params}");
        assert_eq!(out.data["title"], "Train", "{params}");
    }
}
