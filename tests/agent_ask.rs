//! A question, a plan and the plugin builder's language choice, end to end:
//! a scripted model calls the action, the turn parks on the card, the answer
//! posted to `/api/agent/answer` releases it, and the decision is stored with
//! the conversation.
mod support;

use axum::{
    body::{to_bytes, Body},
    extract::{ConnectInfo, Request},
    http::StatusCode,
    routing::post,
    Json, Router,
};
use futures::StreamExt;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

/// A stand-in for Ollama: answers each chat call with the next scripted reply
/// and keeps what it was sent.
#[derive(Clone, Default)]
struct Model {
    replies: Arc<Mutex<VecDeque<String>>>,
    calls: Arc<Mutex<Vec<Value>>>,
}

impl Model {
    fn script(&self, replies: &[&str]) {
        let mut queue = self.replies.lock().unwrap();
        queue.clear();
        queue.extend(replies.iter().map(|reply| reply.to_string()));
        self.calls.lock().unwrap().clear();
    }

    /// Everything the model was sent on its `index`-th call of the script.
    fn prompt(&self, index: usize) -> String {
        let calls = self.calls.lock().unwrap();
        calls[index]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|message| message["content"].as_str().unwrap_or_default())
            .collect::<Vec<_>>()
            .join("\n")
    }

    async fn serve(&self) -> String {
        let model = self.clone();
        let app = Router::new().route(
            "/api/chat",
            post(move |Json(body): Json<Value>| {
                let model = model.clone();
                async move {
                    // The conversation's title is asked for in the background
                    // after a turn: it is not part of any script.
                    let titling = body["messages"][0]["content"]
                        .as_str()
                        .is_some_and(|system| system.starts_with("You title chat conversations"));
                    if titling {
                        return Json(
                            json!({ "message": { "role": "assistant", "content": "a test chat" } }),
                        );
                    }
                    model.calls.lock().unwrap().push(body);
                    let reply = model
                        .replies
                        .lock()
                        .unwrap()
                        .pop_front()
                        .unwrap_or_else(|| "Done.".into());
                    Json(json!({ "message": { "role": "assistant", "content": reply } }))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{address}")
    }
}

struct Fixture {
    app: support::TestApp,
    router: Router,
    model: Model,
}

async fn fixture() -> Fixture {
    let mut app = support::TestApp::new().await;
    let model = Model::default();
    let url = model.serve().await;
    app.state.ollama = shiny::services::ollama::OllamaClient::new(url, "test-model".into());
    for id in ["ada", "eve"] {
        sqlx::query("INSERT INTO travelers (id, name, email, password_hash, auth_token) VALUES (?, ?, ?, '!', ?)")
            .bind(id)
            .bind(id)
            .bind(format!("{id}@test"))
            .bind(id)
            .execute(&app.state.pool)
            .await
            .unwrap();
    }
    let router = shiny::api::build_router(app.state.clone());
    Fixture { app, router, model }
}

fn post_json(path: &str, token: &str, body: Value) -> Request {
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

async fn json_body(response: axum::response::Response) -> Value {
    serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap()).unwrap()
}

/// A streamed turn: its events, read one at a time.
struct Turn {
    body: axum::body::BodyDataStream,
    buffer: String,
}

impl Turn {
    async fn start(fixture: &Fixture, token: &str, body: Value) -> Self {
        let response = fixture
            .router
            .clone()
            .oneshot(post_json("/api/agent", token, body))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        Self {
            body: response.into_body().into_data_stream(),
            buffer: String::new(),
        }
    }

    /// The next event of the given type; events before it are skipped.
    async fn next(&mut self, wanted: &str) -> Value {
        loop {
            while let Some(end) = self.buffer.find("\n\n") {
                let chunk: String = self.buffer.drain(..end + 2).collect();
                let Some(data) = chunk.lines().find_map(|line| line.strip_prefix("data:")) else {
                    continue;
                };
                let event: Value = serde_json::from_str(data.trim()).unwrap();
                if event["type"] == wanted {
                    return event;
                }
                assert_ne!(event["type"], "error", "the turn failed: {event}");
                assert_ne!(
                    event["type"], "done",
                    "the turn ended before a '{wanted}' event: {event}"
                );
            }
            let bytes = tokio::time::timeout(std::time::Duration::from_secs(20), self.body.next())
                .await
                .unwrap_or_else(|_| panic!("no '{wanted}' event within 20 seconds"))
                .unwrap_or_else(|| panic!("the stream closed before a '{wanted}' event"))
                .unwrap();
            self.buffer.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
}

async fn answer(
    fixture: &Fixture,
    token: &str,
    turn_id: &str,
    request_id: &str,
    reply: Value,
) -> bool {
    let mut body = reply;
    body["turn_id"] = json!(turn_id);
    body["request_id"] = json!(request_id);
    let response = fixture
        .router
        .clone()
        .oneshot(post_json("/api/agent/answer", token, body))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    json_body(response).await["success"] == true
}

async fn stored(fixture: &Fixture, conversation: &str) -> (Option<String>, String, Option<String>) {
    sqlx::query_as(
        "SELECT dev_language, plan_state, pending FROM chat_agent_state WHERE conversation_id = ?",
    )
    .bind(conversation)
    .fetch_optional(&fixture.app.state.pool)
    .await
    .unwrap()
    .unwrap_or((None, "none".into(), None))
}

#[tokio::test]
async fn the_language_card_parks_the_turn_and_the_choice_stays_with_the_chat() {
    let fixture = fixture().await;
    fixture.model.script(&[
        r#"{"action":"plugin_dev_start","params":{}}"#,
        "Rust it is.",
    ]);
    let mut turn = Turn::start(
        &fixture,
        "ada",
        json!({ "message": "make me a notes plugin", "stream": true, "ask_cards": true, "turn_id": "t1" }),
    )
    .await;

    let ask = turn.next("ask").await;
    assert_eq!(ask["kind"], "choice");
    assert_eq!(ask["title"]["key"], "chat.ask.language.title");
    let ids: Vec<&str> = ask["options"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["python", "rust"]);
    let request_id = ask["request_id"].as_str().unwrap().to_string();

    // Only the account whose turn it is, on that turn, can answer.
    assert!(
        !answer(
            &fixture,
            "eve",
            "t1",
            &request_id,
            json!({ "option": "python" })
        )
        .await
    );
    assert!(
        !answer(
            &fixture,
            "ada",
            "another-turn",
            &request_id,
            json!({ "option": "python" })
        )
        .await
    );
    assert!(
        answer(
            &fixture,
            "ada",
            "t1",
            &request_id,
            json!({ "option": "rust" })
        )
        .await
    );
    assert!(
        !answer(
            &fixture,
            "ada",
            "t1",
            &request_id,
            json!({ "option": "python" })
        )
        .await,
        "answered once"
    );

    let done = turn.next("done").await;
    assert_eq!(done["data"]["reply"], "Rust it is.");
    let conversation = done["data"]["conversation_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        stored(&fixture, &conversation).await.0.as_deref(),
        Some("rust")
    );
    assert!(
        fixture
            .model
            .prompt(1)
            .contains("The user chose Rust for this chat"),
        "the model is told what the user chose: {}",
        fixture.model.prompt(1)
    );

    // The card is part of the conversation, with its answer and no tool card.
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT role, content FROM chat_messages WHERE conversation_id = ? ORDER BY rowid",
    )
    .bind(&conversation)
    .fetch_all(&fixture.app.state.pool)
    .await
    .unwrap();
    let asks: Vec<Value> = rows
        .iter()
        .filter(|(role, _)| role == "ask")
        .map(|(_, content)| serde_json::from_str(content).unwrap())
        .collect();
    assert_eq!(asks.len(), 1, "{rows:?}");
    assert_eq!(asks[0]["kind"], "choice");
    assert_eq!(asks[0]["answer"]["option"], "rust");
    assert!(rows.iter().all(|(role, _)| role != "tool"), "{rows:?}");

    // Later in the same chat: the prompt says so, and asking again shows no card.
    fixture.model.script(&[
        r#"{"action":"plugin_dev_start","params":{"language":"python"}}"#,
        "Still Rust.",
    ]);
    let mut again = Turn::start(
        &fixture,
        "ada",
        json!({
            "message": "let's go on", "stream": true, "ask_cards": true, "turn_id": "t2",
            "conversation_id": conversation,
        }),
    )
    .await;
    let done = again.next("done").await;
    assert_eq!(done["data"]["reply"], "Still Rust.");
    assert!(
        fixture
            .model
            .prompt(0)
            .contains("This chat builds plugins in Rust"),
        "{}",
        fixture.model.prompt(0)
    );
    assert!(
        fixture
            .model
            .prompt(1)
            .contains("already builds plugins in Rust"),
        "{}",
        fixture.model.prompt(1)
    );
    assert_eq!(
        stored(&fixture, &conversation).await.0.as_deref(),
        Some("rust")
    );

    // Another chat of the same account starts without a language.
    fixture.model.script(&["Hello."]);
    let mut other = Turn::start(
        &fixture,
        "ada",
        json!({ "message": "hi", "stream": true, "turn_id": "t3" }),
    )
    .await;
    other.next("done").await;
    assert!(
        !fixture.model.prompt(0).contains("This chat builds plugins"),
        "{}",
        fixture.model.prompt(0)
    );
}

#[tokio::test]
async fn a_plan_is_approved_on_its_card() {
    let fixture = fixture().await;
    fixture.model.script(&[
        r#"{"action":"plan_propose","params":{"title":"Notes plugin","steps":["Scaffold","Write the window"]}}"#,
        "On it.",
    ]);
    let mut turn = Turn::start(
        &fixture,
        "ada",
        json!({ "message": "plan a notes plugin", "stream": true, "ask_cards": true, "turn_id": "p1" }),
    )
    .await;
    let ask = turn.next("ask").await;
    assert_eq!(ask["kind"], "plan");
    assert_eq!(ask["title"]["text"], "Notes plugin");
    assert_eq!(ask["steps"], json!(["Scaffold", "Write the window"]));
    let request_id = ask["request_id"].as_str().unwrap().to_string();
    assert!(
        answer(
            &fixture,
            "ada",
            "p1",
            &request_id,
            json!({ "option": "approve" })
        )
        .await
    );

    let done = turn.next("done").await;
    let conversation = done["data"]["conversation_id"]
        .as_str()
        .unwrap()
        .to_string();
    let (_, plan_state, pending) = stored(&fixture, &conversation).await;
    assert_eq!(plan_state, "approved");
    assert_eq!(pending, None);
    assert!(
        fixture
            .model
            .prompt(1)
            .contains("The user approved the plan"),
        "{}",
        fixture.model.prompt(1)
    );
}

#[tokio::test]
async fn a_spoken_turn_asks_aloud_and_the_next_message_answers() {
    let fixture = fixture().await;
    fixture.model.script(&[
        r#"{"action":"plan_propose","params":{"title":"Notes plugin","steps":["Scaffold","Write the window"]}}"#,
        "I would scaffold it, then write the window. Shall I go ahead?",
    ]);
    let response = fixture
        .router
        .clone()
        .oneshot(post_json(
            "/api/agent",
            "ada",
            json!({ "message": "plan a notes plugin", "voice": true }),
        ))
        .await
        .unwrap();
    let body = json_body(response).await;
    assert_eq!(
        body["reply"],
        "I would scaffold it, then write the window. Shall I go ahead?"
    );
    let conversation = body["conversation_id"].as_str().unwrap().to_string();
    assert!(
        fixture
            .model
            .prompt(1)
            .contains("There is no card in a spoken turn"),
        "the model is told to ask aloud: {}",
        fixture.model.prompt(1)
    );
    let (_, plan_state, pending) = stored(&fixture, &conversation).await;
    assert_eq!(
        (plan_state.as_str(), pending.as_deref()),
        ("proposed", Some("plan"))
    );

    // A plain yes approves it, and that is decided before the model answers.
    fixture.model.script(&["Starting."]);
    let response = fixture
        .router
        .clone()
        .oneshot(post_json(
            "/api/agent",
            "ada",
            json!({ "message": "Sì, procedi", "voice": true, "conversation_id": conversation }),
        ))
        .await
        .unwrap();
    assert_eq!(json_body(response).await["reply"], "Starting.");
    let (_, plan_state, pending) = stored(&fixture, &conversation).await;
    assert_eq!(
        (plan_state.as_str(), pending.as_deref()),
        ("approved", None)
    );
    let prompt = fixture.model.prompt(0);
    assert!(
        prompt.contains("approved by the user") && prompt.contains("1. Scaffold"),
        "{prompt}"
    );
    assert!(
        prompt.contains("The user's message approves the plan"),
        "{prompt}"
    );
}

#[tokio::test]
async fn a_remote_client_cannot_open_the_plugin_builder() {
    let fixture = fixture().await;
    fixture.model.script(&[
        r#"{"action":"plugin_dev_start","params":{}}"#,
        "Not from here.",
    ]);
    let mut request = post_json(
        "/api/agent",
        "ada",
        json!({ "message": "make a plugin", "stream": true, "ask_cards": true, "turn_id": "r1" }),
    );
    request
        .headers_mut()
        .insert("x-forwarded-for", "203.0.113.7".parse().unwrap());
    let response = fixture.router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mut turn = Turn {
        body: response.into_body().into_data_stream(),
        buffer: String::new(),
    };
    // A card would park the turn, and waiting for `done` would time out.
    let done = turn.next("done").await;
    let conversation = done["data"]["conversation_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        fixture
            .model
            .prompt(1)
            .contains("only be built from the machine itself"),
        "{}",
        fixture.model.prompt(1)
    );
    assert_eq!(stored(&fixture, &conversation).await.0, None);
}

// ── The plugin builder, driven through the agent loop ───────────────────

/// One model step: a tool call as the model would write it.
fn call(action: &str, params: Value) -> String {
    json!({ "action": action, "params": params }).to_string()
}

/// Run one streamed turn of a builder chat. Cards are answered as given:
/// `language` for the choice, `plan` for the plan, `allow` for an install.
struct Answers {
    language: &'static str,
    plan: &'static str,
    allow: bool,
}

async fn builder_turn(
    fixture: &Fixture,
    turn_id: &str,
    conversation: Option<&str>,
    answers: &Answers,
) -> (String, Vec<Value>) {
    let mut body = json!({
        "message": "build it", "stream": true, "ask_cards": true, "approve_tools": true, "turn_id": turn_id,
    });
    if let Some(conversation) = conversation {
        body["conversation_id"] = json!(conversation);
    }
    let response = fixture
        .router
        .clone()
        .oneshot(post_json("/api/agent", "ada", body))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mut stream = response.into_body().into_data_stream();
    let mut buffer = String::new();
    let mut tools: Vec<Value> = Vec::new();
    loop {
        while let Some(end) = buffer.find("\n\n") {
            let chunk: String = buffer.drain(..end + 2).collect();
            let Some(data) = chunk.lines().find_map(|line| line.strip_prefix("data:")) else {
                continue;
            };
            let event: Value = serde_json::from_str(data.trim()).unwrap();
            match event["type"].as_str().unwrap_or_default() {
                "ask" => {
                    let option = if event["kind"] == "choice" {
                        answers.language
                    } else {
                        answers.plan
                    };
                    let request = event["request_id"].as_str().unwrap().to_string();
                    assert!(
                        answer(
                            fixture,
                            "ada",
                            turn_id,
                            &request,
                            json!({ "option": option })
                        )
                        .await
                    );
                }
                "approval" => {
                    tools.push(json!({ "action": "<consent asked>" }));
                    let body = json!({
                        "turn_id": turn_id,
                        "request_id": event["request_id"],
                        "decision": if answers.allow { "allow" } else { "deny" },
                    });
                    let response = fixture
                        .router
                        .clone()
                        .oneshot(post_json("/api/agent/approve", "ada", body))
                        .await
                        .unwrap();
                    assert_eq!(json_body(response).await["success"], true);
                }
                "tool" => tools.push(event),
                "done" => {
                    return (
                        event["data"]["conversation_id"]
                            .as_str()
                            .unwrap()
                            .to_string(),
                        tools,
                    );
                }
                "error" => panic!("the turn failed: {event}"),
                _ => {}
            }
        }
        let bytes = tokio::time::timeout(std::time::Duration::from_secs(1500), stream.next())
            .await
            .expect("the turn went quiet")
            .expect("the stream closed before the turn ended")
            .unwrap();
        buffer.push_str(&String::from_utf8_lossy(&bytes));
    }
}

/// The card record of the `index`-th call of `action` in a turn.
fn card<'a>(tools: &'a [Value], action: &str, index: usize) -> &'a Value {
    tools
        .iter()
        .filter(|tool| tool["action"] == action)
        .nth(index)
        .unwrap_or_else(|| panic!("no call #{index} of {action} in {tools:#?}"))
}

#[tokio::test]
async fn nothing_is_built_without_a_language_a_plan_and_the_right_module() {
    let fixture = fixture().await;
    let rust = Answers {
        language: "rust",
        plan: "changes",
        allow: true,
    };

    // Before a language is chosen the builder's commands do not exist for the
    // chat; once Rust is chosen, writing still waits for an approved plan.
    fixture.model.script(&[
        &call("plugin_dev_new", json!({ "name": "notes" })),
        &call("plugin_dev_start", json!({})),
        &call("plugin_dev_new", json!({ "name": "notes" })),
        &call("plan_propose", json!({ "steps": ["Create it"] })),
        &call("plugin_dev_new", json!({ "name": "notes" })),
        &call("plugin_dev_list", json!({})),
        "Waiting for your plan.",
    ]);
    let (conversation, tools) = builder_turn(&fixture, "g1", None, &rust).await;
    let reason = |index: usize| {
        card(&tools, "plugin_dev_new", index)["data"]["reason"]
            .as_str()
            .unwrap()
            .to_string()
    };
    // A refusal reaches the chat as a failed step that says why.
    assert_eq!(card(&tools, "plugin_dev_new", 0)["result"], "error");
    assert!(
        reason(0).contains("has not chosen a language"),
        "{}",
        reason(0)
    );
    assert!(
        reason(1).contains("has not approved a plan"),
        "{}",
        reason(1)
    );
    assert!(
        reason(2).contains("asked for changes to the plan"),
        "{}",
        reason(2)
    );
    // Reading is never held back by the plan.
    assert_eq!(card(&tools, "plugin_dev_list", 0)["result"], "ok");
    let workspaces = fixture.app.root.path().join("plugin-dev");
    assert!(!workspaces.join("notes").exists(), "nothing was created");

    // The builder's docs reached the model with the language, and only its own.
    let after_start = fixture.model.prompt(2);
    assert!(
        after_start.contains("Plugin builder commands for this chat"),
        "{after_start}"
    );
    assert!(after_start.contains("rust_build"), "{after_start}");

    // With the plan approved the plugin is created, read, changed and checked.
    let approve = Answers {
        plan: "approve",
        ..rust
    };
    fixture.model.script(&[
        &call("plan_propose", json!({ "steps": ["Create it"] })),
        &call("plugin_dev_new", json!({ "name": "notes", "title": "Notes", "description": "Quick notes" })),
        &call("plugin_dev_read", json!({ "path": "plugin.toml" })),
        &call("plugin_dev_patch", json!({ "path": "web/plugin.js", "find": "'plugin.notes.add'", "replace": "'plugin.notes.addNew'" })),
        &call("check_all", json!({})),
        &call("plugin_dev_patch", json!({ "path": "web/plugin.js", "find": "'plugin.notes.addNew'", "replace": "'plugin.notes.add'" })),
        &call("check_all", json!({})),
        &call("plugin_dev_write", json!({ "path": "../escape.rs", "content": "x" })),
        &call("plugin_dev_pack", json!({})),
        &call("plugin_dev_status", json!({})),
        "Checked.",
    ]);
    let (_, tools) = builder_turn(&fixture, "g2", Some(&conversation), &approve).await;
    assert_eq!(
        card(&tools, "plugin_dev_new", 0)["result"],
        "ok",
        "{:#?}",
        card(&tools, "plugin_dev_new", 0)
    );
    assert!(workspaces.join("notes/src/plugin.rs").is_file());
    assert!(card(&tools, "plugin_dev_read", 0)["data"]["content"]
        .as_str()
        .unwrap()
        .contains("name = \"notes\""));
    let failed = &card(&tools, "check_all", 0)["data"];
    assert_eq!(failed["ok"], false);
    assert_eq!(failed["errors"][0]["file"], "web/plugin.js");
    assert_eq!(
        card(&tools, "check_all", 1)["data"]["ok"],
        true,
        "{:#?}",
        card(&tools, "check_all", 1)
    );
    assert_eq!(
        card(&tools, "plugin_dev_write", 0)["result"],
        "error",
        "a path outside the plugin"
    );
    assert!(!fixture
        .app
        .root
        .path()
        .join("plugin-dev/escape.rs")
        .exists());
    let pack = card(&tools, "plugin_dev_pack", 0);
    assert_eq!(pack["result"], "error");
    assert!(
        pack["data"]["reason"]
            .as_str()
            .unwrap()
            .contains("not built"),
        "{pack}"
    );
    let status = &card(&tools, "plugin_dev_status", 0)["data"];
    assert_eq!(status["sources"]["checked"], true);
    assert_eq!(status["sources"]["built"], false);
    assert_eq!(status["installed"], Value::Null);

    // Another chat chose Python: the Rust commands are not its own, and the
    // Rust plugin belongs to the other chat's language.
    let python = Answers {
        language: "python",
        plan: "approve",
        allow: true,
    };
    fixture.model.script(&[
        &call("plugin_dev_start", json!({})),
        &call("rust_build", json!({})),
        &call("plugin_dev_tree", json!({ "name": "notes" })),
        "Not here.",
    ]);
    let (_, tools) = builder_turn(&fixture, "g3", None, &python).await;
    let build = card(&tools, "rust_build", 0);
    assert_eq!(build["result"], "error");
    assert!(
        build["data"]["reason"]
            .as_str()
            .unwrap()
            .contains("builds plugins in Python"),
        "{build}"
    );
    let tree = card(&tools, "plugin_dev_tree", 0);
    assert!(
        tree["data"]["reason"]
            .as_str()
            .unwrap()
            .contains("written in Rust"),
        "{tree}"
    );
    let docs = fixture.model.prompt(1);
    assert!(
        docs.contains("py_check") && !docs.contains("rust_build"),
        "only this chat's commands: {docs}"
    );
}

/// A Python plugin, the whole road: created, checked, started as a test,
/// packed, installed with the user's consent, then used through its tool and
/// its route. Nothing is compiled, so this runs with the other tests.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_python_plugin_is_checked_installed_and_used() {
    let fixture = fixture().await;
    let answers = Answers {
        language: "python",
        plan: "approve",
        allow: true,
    };
    fixture.model.script(&[
        &call("plugin_dev_start", json!({})),
        &call("plan_propose", json!({ "title": "Tasks", "steps": ["Create", "Check", "Install"] })),
        &call("plugin_dev_new", json!({ "name": "tasks", "title": "Tasks", "description": "A task list" })),
        // A mistake the check has to find, and an edit the kit refuses.
        &call("plugin_dev_patch", json!({ "path": "engine/plugin.py", "find": "import uuid", "replace": "import uuid\nimport requests" })),
        &call("plugin_dev_write", json!({ "path": "engine/shiny_plugin.py", "content": "print('mine')" })),
        &call("py_check", json!({})),
        &call("plugin_dev_patch", json!({ "path": "engine/plugin.py", "find": "import uuid\nimport requests", "replace": "import uuid" })),
        &call("check_all", json!({})),
        &call("plugin_dev_pack", json!({})),
        // Nothing is packed: this refuses without asking the user anything.
        &call("plugin_dev_install", json!({ "early": true })),
        &call("py_check", json!({})),
        &call("py_test", json!({})),
        &call("plugin_dev_pack", json!({})),
        &call("plugin_dev_install", json!({})),
        &call("plugin_dev_status", json!({})),
        &call("tasks_add", json!({ "text": "Water the plants" })),
        &call("tasks_list", json!({})),
        "Installed.",
    ]);
    let (conversation, tools) = builder_turn(&fixture, "p1", None, &answers).await;
    let consents = tools
        .iter()
        .filter(|tool| tool["action"] == "<consent asked>")
        .count();

    assert_eq!(
        card(&tools, "plugin_dev_write", 0)["result"],
        "error",
        "the kit is not edited"
    );
    let broken = &card(&tools, "py_check", 0)["data"];
    assert_eq!(broken["ok"], false, "{broken:#}");
    assert_eq!(broken["errors"][0]["file"], "engine/plugin.py");
    assert!(
        broken["errors"][0]["message"]
            .as_str()
            .unwrap()
            .contains("import of 'requests'"),
        "{broken:#}"
    );
    assert_eq!(
        card(&tools, "check_all", 0)["data"]["ok"],
        true,
        "{:#}",
        card(&tools, "check_all", 0)
    );
    // Its engine was not checked since the last change: no pack yet.
    assert_eq!(card(&tools, "plugin_dev_pack", 0)["result"], "error");
    assert_eq!(card(&tools, "py_check", 1)["data"]["ok"], true);
    let tested = &card(&tools, "py_test", 0)["data"];
    assert_eq!(tested["started"], true, "{tested:#}");
    assert_eq!(tested["ok"], true, "{tested:#}");
    assert_eq!(tested["tools"], json!(["tasks_add", "tasks_list"]));
    assert_eq!(
        tested["routes"],
        json!(["GET /api/tasks/items", "POST /api/tasks/items"])
    );

    assert_eq!(
        card(&tools, "plugin_dev_pack", 1)["data"]["archive"],
        "dist/tasks-0.1.0.tar.gz"
    );
    assert_eq!(
        card(&tools, "plugin_dev_install", 0)["result"],
        "error",
        "not packed yet"
    );
    assert_eq!(
        consents, 1,
        "only the install that could happen asked for consent"
    );
    let install = card(&tools, "plugin_dev_install", 1);
    assert_eq!(install["result"], "ok", "{install:#}");
    let status = &card(&tools, "plugin_dev_status", 0)["data"];
    assert_eq!(status["installed"]["running"], true, "{status:#}");
    assert_eq!(
        status["installed"]["tools"],
        json!(["tasks_add", "tasks_list"])
    );

    assert_eq!(
        card(&tools, "tasks_add", 0)["result"],
        "ok",
        "{:#}",
        card(&tools, "tasks_add", 0)
    );
    let listed = &card(&tools, "tasks_list", 0)["data"];
    assert_eq!(listed["count"], 1);
    assert_eq!(listed["items"][0]["text"], "Water the plants");

    let router = shiny::api::build_router(fixture.app.state.clone());
    let mut request = Request::builder()
        .uri("/api/tasks/items")
        .header("authorization", "Bearer ada")
        .body(Body::empty())
        .unwrap();
    request.extensions_mut().insert(ConnectInfo(
        "127.0.0.1:1234".parse::<std::net::SocketAddr>().unwrap(),
    ));
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        json_body(response).await["data"]["items"][0]["text"],
        "Water the plants"
    );

    // The installed copy carries its kit.
    let installed = fixture.app.root.path().join("plugins/tasks");
    assert!(installed.join("engine/shiny_plugin.py").is_file());
    assert!(installed.join("bin/tasks").is_file());

    // The window reports an error from the page; only an installed plugin's is kept.
    let router = shiny::api::build_router(fixture.app.state.clone());
    for (plugin, recorded) in [("tasks", true), ("ghost", false)] {
        let report = json!({ "plugin": plugin, "message": "items is not defined", "source": "plugin.js", "line": 41 });
        let response = router
            .clone()
            .oneshot(post_json("/api/plugins/dev/console", "ada", report))
            .await
            .unwrap();
        assert_eq!(
            json_body(response).await["data"]["recorded"],
            recorded,
            "{plugin}"
        );
    }

    // Later in the same chat: try it, read the window's errors, ship a new
    // version, go back to the previous one, and remove it.
    fixture.model.script(&[
        &call("plugin_dev_call", json!({ "tool": "tasks_list" })),
        &call("plugin_dev_call", json!({ "tool": "file_write" })),
        &call("plugin_dev_http", json!({ "path": "/api/tasks/items", "method": "POST", "body": { "text": "From the route" } })),
        &call("plugin_dev_http", json!({ "path": "/api/trips" })),
        &call("plugin_dev_console", json!({})),
        &call("plugin_dev_patch", json!({ "path": "plugin.toml", "find": "version = \"0.1.0\"", "replace": "version = \"0.1.1\"" })),
        &call("check_all", json!({})),
        &call("py_check", json!({})),
        &call("plugin_dev_pack", json!({})),
        &call("plugin_dev_install", json!({})),
        &call("plugin_dev_status", json!({})),
        &call("plugin_dev_rollback", json!({})),
        &call("plugin_dev_status", json!({ "refresh": true })),
        &call("tasks_list", json!({})),
        &call("plugin_dev_uninstall", json!({})),
        &call("plugin_dev_status", json!({ "after": "uninstall" })),
        &call("plugin_dev_open", json!({ "name": "photocraft" })),
        "Done.",
    ]);
    let (_, tools) = builder_turn(&fixture, "p2", Some(&conversation), &answers).await;
    let tried = &card(&tools, "plugin_dev_call", 0)["data"];
    assert!(
        tried["data"].as_str().unwrap().contains("Water the plants"),
        "{tried:#}"
    );
    assert_eq!(
        card(&tools, "plugin_dev_call", 1)["result"],
        "error",
        "only the plugin's own tools"
    );
    let posted = &card(&tools, "plugin_dev_http", 0)["data"];
    assert_eq!(posted["status"], 200, "{posted:#}");
    assert!(posted["body"].as_str().unwrap().contains("From the route"));
    assert_eq!(
        card(&tools, "plugin_dev_http", 1)["result"],
        "error",
        "only the plugin's own routes"
    );
    let console = &card(&tools, "plugin_dev_console", 0)["data"];
    assert_eq!(console["errors"][0]["message"], "items is not defined");
    assert_eq!(console["errors"][0]["line"], 41);

    assert_eq!(
        card(&tools, "plugin_dev_install", 0)["data"]["version"],
        "0.1.1"
    );
    assert_eq!(
        card(&tools, "plugin_dev_status", 0)["data"]["installed"]["version"],
        "0.1.1"
    );
    assert_eq!(
        card(&tools, "plugin_dev_rollback", 0)["result"],
        "ok",
        "{:#}",
        card(&tools, "plugin_dev_rollback", 0)
    );
    let back = &card(&tools, "plugin_dev_status", 1)["data"];
    assert_eq!(back["installed"]["version"], "0.1.0", "{back:#}");
    // The data outlives the versions.
    assert_eq!(card(&tools, "tasks_list", 0)["data"]["count"], 2);

    assert_eq!(card(&tools, "plugin_dev_uninstall", 0)["result"], "ok");
    assert_eq!(
        card(&tools, "plugin_dev_status", 2)["data"]["installed"],
        Value::Null
    );
    assert!(!installed.exists());
    assert!(
        fixture
            .app
            .root
            .path()
            .join("plugin-dev/tasks/engine/plugin.py")
            .is_file(),
        "the sources are kept"
    );
    let craft = card(&tools, "plugin_dev_open", 0);
    assert_eq!(craft["result"], "error");
    assert!(
        craft["data"]["reason"]
            .as_str()
            .unwrap()
            .contains("app from the store"),
        "{craft}"
    );
    fixture.app.state.plugins.stop_processes().await;
}

/// The whole road with the real compiler: create, check, build, pack, install
/// with the user's consent, then use the plugin's tool and its route.
///
/// Ignored by default: the first build compiles the SDK and its dependencies
/// (minutes). Run it with
/// `cargo test -p shiny --test agent_ask -- --ignored a_rust_plugin`.
/// `SHINY_TEST_PLUGIN_TARGET` may name a warm target directory to reuse.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "runs a real cargo build"]
async fn a_rust_plugin_is_built_installed_and_used() {
    let fixture = fixture().await;
    let workspaces = fixture.app.root.path().join("plugin-dev");
    if let Ok(warm) = std::env::var("SHINY_TEST_PLUGIN_TARGET") {
        std::fs::create_dir_all(&workspaces).unwrap();
        std::fs::create_dir_all(&warm).unwrap();
        std::os::unix::fs::symlink(&warm, workspaces.join(".target")).unwrap();
    }
    let answers = Answers {
        language: "rust",
        plan: "approve",
        allow: true,
    };
    fixture.model.script(&[
        &call("plugin_dev_start", json!({})),
        &call("plan_propose", json!({ "title": "Notes", "steps": ["Create", "Build", "Install"] })),
        &call("plugin_dev_new", json!({ "name": "notes", "title": "Notes", "description": "Quick notes" })),
        // A first build straight from the template: cargo rewrites its lock
        // file while building, which must not make the build look stale.
        &call("check_all", json!({ "first": true })),
        &call("rust_build", json!({ "first": true })),
        &call("rust_test", json!({ "first": true })),
        // A mistake the compiler has to find.
        &call("plugin_dev_patch", json!({ "path": "src/store.rs", "find": "let id = uuid::Uuid::new_v4().to_string();", "replace": "let id: i64 = uuid::Uuid::new_v4().to_string();" })),
        &call("check_all", json!({})),
        &call("rust_build", json!({})),
        &call("plugin_dev_patch", json!({ "path": "src/store.rs", "find": "let id: i64 = ", "replace": "let id = " })),
        &call("check_all", json!({})),
        &call("rust_build", json!({})),
        &call("rust_test", json!({})),
        &call("plugin_dev_pack", json!({})),
        &call("plugin_dev_install", json!({})),
        &call("plugin_dev_status", json!({})),
        &call("notes_add", json!({ "text": "Buy milk" })),
        &call("notes_list", json!({})),
        "Installed.",
    ]);
    let (_, tools) = builder_turn(&fixture, "b1", None, &answers).await;

    assert_eq!(card(&tools, "rust_build", 0)["data"]["built"], true);
    let first = &card(&tools, "rust_test", 0)["data"];
    assert_eq!(
        first["started"], true,
        "a build is not made stale by its own lock file: {first:#}"
    );
    let broken = &card(&tools, "rust_build", 1)["data"];
    assert_eq!(broken["built"], false, "{broken:#}");
    assert_eq!(broken["errors"][0]["file"], "src/store.rs", "{broken:#}");
    assert!(broken["errors"][0]["line"].as_u64().unwrap() > 0);
    assert_eq!(broken["errors"][0]["code"], "E0308");
    let built = &card(&tools, "rust_build", 2)["data"];
    assert_eq!(built["built"], true, "{built:#}");

    assert_eq!(
        card(&tools, "plugin_dev_pack", 0)["data"]["archive"],
        "dist/notes-0.1.0.tar.gz"
    );
    let tested = &card(&tools, "rust_test", 1)["data"];
    assert_eq!(tested["started"], true, "{tested:#}");
    assert_eq!(tested["tools"], json!(["notes_add", "notes_list"]));
    let install = card(&tools, "plugin_dev_install", 0);
    assert_eq!(install["result"], "ok", "{install:#}");
    assert_eq!(install["data"]["tools"], json!(["notes_add", "notes_list"]));
    let status = &card(&tools, "plugin_dev_status", 0)["data"];
    assert_eq!(status["installed"]["version"], "0.1.0");
    assert_eq!(status["installed"]["running"], true);
    assert_eq!(
        status["installed"]["routes"],
        json!(["GET /api/notes/items", "POST /api/notes/items"])
    );

    // The installed plugin's own tools answer in the same turn.
    assert_eq!(
        card(&tools, "notes_add", 0)["result"],
        "ok",
        "{:#}",
        card(&tools, "notes_add", 0)
    );
    let listed = &card(&tools, "notes_list", 0)["data"];
    assert_eq!(listed["count"], 1);
    assert_eq!(listed["items"][0]["text"], "Buy milk");

    // And its route serves the window. The router the test holds was built
    // before the install; a rebuilt one has the plugin's routes.
    let router = shiny::api::build_router(fixture.app.state.clone());
    let mut request = Request::builder()
        .uri("/api/notes/items")
        .header("authorization", "Bearer ada")
        .body(Body::empty())
        .unwrap();
    request.extensions_mut().insert(ConnectInfo(
        "127.0.0.1:1234".parse::<std::net::SocketAddr>().unwrap(),
    ));
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        json_body(response).await["data"]["items"][0]["text"],
        "Buy milk"
    );

    // The plugin's process is a child of this test: stop it before leaving.
    fixture.app.state.plugins.stop_processes().await;
}

/// A turn that opens the builder itself gets the builder's longer leash from
/// that moment: building a plugin takes more steps than an ordinary turn may.
#[tokio::test]
async fn a_turn_that_opens_the_builder_may_run_long() {
    let fixture = fixture().await;
    let answers = Answers {
        language: "python",
        plan: "approve",
        allow: true,
    };
    let mut script: Vec<String> = vec![
        call("plugin_dev_start", json!({})),
        call("plan_propose", json!({ "steps": ["Create it"] })),
        call("plugin_dev_new", json!({ "name": "longjob" })),
    ];
    // Well past the forty steps of an ordinary turn.
    script.extend((0..60).map(|step| call("plugin_dev_tree", json!({ "step": step }))));
    script.push("All sixty looked at.".into());
    let script: Vec<&str> = script.iter().map(String::as_str).collect();
    fixture.model.script(&script);

    let response = fixture
        .router
        .clone()
        .oneshot(post_json(
            "/api/agent",
            "ada",
            json!({ "message": "build it", "stream": true, "ask_cards": true, "approve_tools": true, "turn_id": "long" }),
        ))
        .await
        .unwrap();
    let mut turn = Turn {
        body: response.into_body().into_data_stream(),
        buffer: String::new(),
    };
    for _ in 0..2 {
        let ask = turn.next("ask").await;
        let option = if ask["kind"] == "choice" {
            answers.language
        } else {
            answers.plan
        };
        let request = ask["request_id"].as_str().unwrap().to_string();
        assert!(
            answer(
                &fixture,
                "ada",
                "long",
                &request,
                json!({ "option": option })
            )
            .await
        );
    }
    let done = turn.next("done").await;
    assert_eq!(
        done["data"]["reply"], "All sixty looked at.",
        "the turn was not cut at forty steps"
    );
}
