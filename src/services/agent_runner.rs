//! Shared agent execution loop with step callbacks.

use serde_json::{Value, json};
use shiny_plugin_sdk::tools::{normalize_action_name, UiText};

use crate::api::AppState;
use crate::errors::AppError;
use crate::models::Traveler;
use crate::services::ai::AiClient;
use crate::services::approval::{ApprovalGate, Authorization};
use crate::services::agent_steps::{
    build_continuation_system, build_planning_messages, describe_tool_step,
    done_label_for_action, failed_label_for_action, messages_char_count, step_label_for_action,
    thinking_label,
};
use crate::services::agent_tools::{
    execute_action, parse_actions, strip_action_blocks, AgentContext,
};
use crate::services::artifacts::Artifact;
use crate::services::navigation::NavigationSession;

const MAX_TOOL_STEPS: usize = 40;

/// Has the user stopped this turn? (No handle = nothing to cancel.)
fn turn_stopped(cancel: &Option<crate::services::agent_cancel::CancelHandle>) -> bool {
    cancel.as_ref().is_some_and(|c| c.is_cancelled())
}

/// Keep the speech bubble to one short line: collapse whitespace, cap length.
/// The model gets the full note — the UI never should.
fn shorten_for_ui(msg: &str) -> String {
    let one_line = msg.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= 72 {
        return one_line;
    }
    let cut: String = one_line.chars().take(71).collect();
    format!("{}…", cut.trim_end())
}

/// Human, UI-facing status line for a tool about to run: the plugin's own keyed
/// `Tool::step_ui` when it has one, else its `step_label`, else core's map. The
/// web client translates the key and falls back to the English text.
fn ui_step(state: &AppState, action: &str) -> UiText {
    if state.plugins.tools().has(action) {
        if let Some(mut ui) = state.plugins.tools().step_ui(action) {
            ui.text = shorten_for_ui(&ui.text);
            return ui;
        }
        let label = state.plugins.tools().step_label(action);
        return UiText::new(shorten_for_ui(&label));
    }
    let step = step_label_for_action(action);
    UiText::keyed(step.key, step.text)
}

/// Human, UI-facing "done" line for a finished tool. Plugin tools answer with
/// their own keyed `Tool::done_ui`, then their `humanize` (short by convention);
/// core actions fall back to the built-in phrases. A JSON-looking result is
/// rejected rather than shown.
fn ui_done(state: &AppState, action: &str, result: &str, data: &Value) -> UiText {
    if result != "ok" {
        let step = failed_label_for_action(action);
        return UiText::keyed(step.key, step.text);
    }
    if state.plugins.tools().has(action) {
        if let Some(mut ui) = state.plugins.tools().done_ui(action, result, data) {
            let trimmed = ui.text.trim();
            // A keyed line may carry `{placeholders}`; an unkeyed one must not
            // look like JSON.
            if ui.key.is_some() || (!trimmed.is_empty() && !trimmed.contains('{')) {
                ui.text = shorten_for_ui(trimmed);
                return ui;
            }
        }
        let human = state.plugins.tools().humanize(action, result, data);
        let human = human.trim();
        if !human.is_empty() && !human.contains('{') {
            return UiText::new(shorten_for_ui(human));
        }
    }
    let step = done_label_for_action(action);
    UiText::keyed(step.key, step.text)
}

/// The shell command an action is about to run, when it is the Terminal's
/// `terminal_exec`. Resolves aliases ("terminal", "shell", "run_command") and a
/// plugin-namespaced name ("terminal.terminal_exec") through the registry, so a
/// gate cannot be sidestepped by calling the tool by another of its names.
fn terminal_command(state: &AppState, action: &str, params: &Value) -> Option<String> {
    let normalized = normalize_action_name(action);
    let canonical = state
        .plugins
        .tools()
        .canonical_name(&normalized)
        .or_else(|| {
            let (_, rest) = normalized.split_once('.')?;
            state.plugins.tools().canonical_name(rest)
        })?;
    if canonical != "terminal_exec" {
        return None;
    }
    params
        .get("command")
        .or_else(|| params.get("cmd"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|command| !command.is_empty())
        .map(str::to_string)
}

pub struct AgentRunInput {
    pub message: String,
    pub mode: String,
    pub lang: String,
    pub ai_name: String,
    pub system: String,
    /// Compact plugin catalog line for continuation prompts (e.g.
    /// "traveler: Trip tracking…; hello: Demo…"). Empty when no plugins.
    pub plugins_hint: String,
    /// Earlier turns of this conversation (`(role, content)` pairs), so the
    /// model keeps the thread's context even mid-tool-loop.
    pub history: Vec<(String, String)>,
    /// True when the request came from speech (STT) and the reply is spoken
    /// back (TTS) — the reply must then be conversational prose, not markdown.
    pub voice: bool,
    /// True when the user stopped the previous reply to correct it: the message
    /// is read against the request it corrects, not as a new question.
    pub correction: bool,
    /// Ollama's `think` switch for this turn: `Some(true)` asks a thinking
    /// model for its reasoning trace, `Some(false)` turns it off, `None`
    /// leaves the model's own default in place.
    pub think: Option<bool>,
    pub ctx: AgentContext,
    /// Craft apps whose windows the client reported open for this turn, and the
    /// home their CLIs are scoped to. The loop itself opens an app when the
    /// model calls `show_plugin`, and it has to expose that app's MCP the same
    /// way the turn start did (see `plugins::craft_mcp`).
    pub craft_open: std::collections::BTreeSet<String>,
    pub craft_home: String,
    /// Terminal-command consent for this turn. `None` only in tests; a real
    /// turn always carries one, so a command cannot run without passing the
    /// hard-coded guard and the chat's allow/deny answer.
    pub approval: Option<ApprovalGate>,
}

pub struct ActionTaken {
    pub action: String,
    pub result: String,
    /// Small outcome payload (e.g. `{video_id}` for youtube_play) — included
    /// only when compact, so the frontend can act on it.
    pub data: Option<Value>,
}

pub struct AgentRunResult {
    pub success: bool,
    pub reply: String,
    pub mode: String,
    /// Response artifacts as tagged JSON values (include the `plugin` key so
    /// the UI can group surfaces by plugin without a refetch).
    pub artifacts: Vec<Value>,
    pub actions_taken: Vec<ActionTaken>,
    pub navigation: Option<NavigationSession>,
    /// Plugin the AI chose to surface (via the show_plugin tool).
    pub focus_plugin: Option<String>,
    /// File the AI asked the frontend to open (generate_image `open`).
    pub open_file: Option<Value>,
    pub steps: Vec<String>,
    /// The model's reasoning trace for this turn (every thinking block of the
    /// tool loop, joined). Empty unless thinking was on and the model has it.
    pub thinking: String,
    /// Mid-turn rows persisted with the conversation, in visual order:
    /// `("approval", {command, decision})` and `("media", {kind, url, …})`.
    pub events: Vec<(String, String)>,
    /// True when the user stopped this turn before it finished. The reply (if
    /// any) is a partial answer, and the caller records an invisible note so
    /// the model understands why the turn looks truncated next time.
    pub interrupted: bool,
}

#[allow(clippy::too_many_arguments)]
pub async fn run_agent<F, G, H>(
    state: &AppState,
    traveler: &Traveler,
    trip_id: Option<&str>,
    ai: &AiClient,
    model: Option<&str>,
    input: AgentRunInput,
    cancel: Option<crate::services::agent_cancel::CancelHandle>,
    mut on_step: F,
    mut on_thinking: G,
    mut on_artifact: H,
) -> Result<AgentRunResult, AppError>
where
    // The step line, plus the tool it belongs to when it is a tool's
    // (`None` for the thinking opener and the refusals with no action).
    F: FnMut(&UiText, Option<&str>),
    G: FnMut(&str),
    // A just-saved artifact, streamed to the UI before the turn ends. A window
    // the tool opens (the Browser) must exist before the next tool call, not
    // only when the final response renders the cards.
    H: FnMut(Value),
{
    let mut artifacts: Vec<Value> = Vec::new();
    let mut actions_taken: Vec<ActionTaken> = Vec::new();
    let mut navigation: Option<NavigationSession> = None;
    let mut focus_plugin: Option<String> = None;
    let mut open_file: Option<Value> = None;
    let mut injected_skills: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut completed_steps: Vec<String> = Vec::new();
    let mut thinking_log = String::new();
    let mut final_reply = String::new();
    let mut turn_events: Vec<(String, String)> = Vec::new();
    let mut interrupted = false;

    // The continuation prompt is append-only: its system half and the
    // conversation history are rendered once, then each finished tool appends
    // one `[Done]` line. Re-rendering every earlier step from scratch on each
    // iteration made the prompt copy cost grow quadratically.
    let planning_messages = build_planning_messages(&input.system, &input.message);
    let continuation_system = build_continuation_system(
        &input.ai_name,
        &input.lang,
        &input.mode,
        &input.plugins_hint,
        &input.history,
        input.voice,
        input.correction,
    );
    let mut continuation_messages = vec![
        ("system".to_string(), continuation_system),
        ("user".to_string(), input.message.clone()),
    ];

    let think = thinking_label();
    on_step(&UiText::keyed(think.key, think.text), None);

    // Craft-app AI control is a property of the turn start, but the loop itself
    // opens windows (`show_plugin`): this is the set the MCP host is synced with
    // as apps are opened mid-turn.
    let focused_craft: std::collections::BTreeSet<String> = input.ctx.focused.iter().cloned().collect();
    let mut craft_open = input.craft_open.clone();

    for iteration in 0..MAX_TOOL_STEPS {
        if turn_stopped(&cancel) {
            interrupted = true;
            break;
        }

        let messages = if completed_steps.is_empty() && iteration == 0 {
            planning_messages.clone()
        } else {
            continuation_messages.clone()
        };

        let size = messages_char_count(&messages);
        tracing::debug!("Agent AI call #{iteration}: ~{size} chars");
        if size > 200_000 {
            tracing::warn!("Agent prompt very large ({size} chars), continuing with slim context");
        }

        // Dropping the in-flight request is the point of the whole feature: a
        // local model that keeps generating blocks the *next* question, so a
        // stopped turn must abandon the call, not just ignore its answer.
        let outcome = match &cancel {
            Some(handle) => tokio::select! {
                biased;
                _ = handle.cancelled() => {
                    interrupted = true;
                    break;
                }
                result = ai.chat_ex(messages, model, input.think) => result?,
            },
            None => ai.chat_ex(messages, model, input.think).await?,
        };
        // A thinking model narrates before it answers: keep every block of the
        // tool loop, hand each one to the UI as it arrives.
        if let Some(trace) = outcome.thinking.as_deref() {
            if !thinking_log.is_empty() {
                thinking_log.push_str("\n\n");
            }
            thinking_log.push_str(trace);
            on_thinking(trace);
        }
        let response = outcome.content;
        // Full transparency: log what the model actually emitted so failures
        // (e.g. "the AI isn't writing to calc") are visible in data/shiny.log.
        tracing::info!(
            "Agent response #{iteration}: {}",
            response.chars().take(1200).collect::<String>()
        );
        let actions = parse_actions(&response);

        if actions.is_empty() {
            // The model replied in plain language — that IS the final reply
            // (tool results already reached it via the [Done] step notes).
            final_reply = strip_action_blocks(&response);
            if final_reply.is_empty() {
                let trimmed = response.trim();
                // If the whole response was (malformed) JSON, don't leak the
                // raw tool call into the chat — fall back to a neutral ack.
                final_reply = if trimmed.starts_with('{') || trimmed.starts_with('[') {
                    "Done.".into()
                } else {
                    trimmed.to_string()
                };
            }
            break;
        }

        // Execute EVERY tool call the model emitted this turn, in order, so
        // multi-step intents ("close all plugins", "create then fill a doc")
        // run sequentially instead of silently dropping everything after the
        // first action.
        for (action, params) in actions {
            // A tool can be slow (a download, a model call of its own). Check
            // between actions so a stop does not have to wait for the batch.
            if turn_stopped(&cancel) {
                interrupted = true;
                break;
            }
            tracing::info!(
                "Agent tool call: {action} {}",
                serde_json::to_string(&params).unwrap_or_default()
            );

            // Workspaces do not exist on a vertical, phone-like screen: the
            // tool is refused before it runs, with a step that says so rather
            // than claiming a workspace was created.
            if action.starts_with("workspace_") && !input.ctx.workspaces_enabled {
                tracing::info!("Refusing {action}: workspaces are off on this screen");
                let step = step_label_for_action("workspaces_unavailable");
                on_step(&UiText::keyed(step.key, step.text), None);
                continue;
            }

            // The assistant's shell is gated per command, in code: the
            // hard-coded guard refuses anything that could destroy the system,
            // then the user allows or denies what is left (unless this chat
            // already allows every command). A refused command never runs.
            if let Some(command) = terminal_command(state, &action, &params) {
                if let Some(reason) = crate::services::terminal_guard::block_reason(&command) {
                    tracing::warn!("Blocked dangerous terminal command: {command}");
                    turn_events.push((
                        "approval".into(),
                        json!({ "command": &command, "decision": "blocked" }).to_string(),
                    ));
                    let step = UiText::keyed("step.commandBlocked", "Blocked a dangerous command");
                    on_step(&step, Some(action.as_str()));
                    actions_taken.push(ActionTaken {
                        action: "terminal_exec".into(),
                        result: "blocked".into(),
                        data: Some(json!({
                            "command": command,
                            "reason": reason,
                            "blocked": true,
                        })),
                    });
                    let note = format!(
                        "terminal_exec blocked by the built-in safety guard ({reason}): nothing \
                         was run. Do not retry it. Tell the user briefly that the command was \
                         blocked and, if a safer alternative exists, propose that instead."
                    );
                    continuation_messages.push(("user".to_string(), format!("[Done] {note}")));
                    completed_steps.push(note);
                    continue;
                }

                let authorization = match &input.approval {
                    Some(gate) => {
                        if gate.will_ask() {
                            let step = UiText::keyed(
                                "step.awaitingApproval",
                                "Waiting for your approval…",
                            );
                            on_step(&step, Some(action.as_str()));
                        }
                        gate.authorize(&action, &command, cancel.as_ref()).await
                    }
                    None => Authorization::Unavailable(
                        "no approval channel is configured for this turn. Nothing was run.",
                    ),
                };

                let decision = match &authorization {
                    Authorization::Allow => "allow",
                    Authorization::Denied(_) => "deny",
                    Authorization::Unavailable(_) => "unavailable",
                };
                turn_events.push((
                    "approval".into(),
                    json!({ "command": &command, "decision": decision }).to_string(),
                ));

                let denial = match authorization {
                    Authorization::Allow => None,
                    Authorization::Denied(reason) => {
                        Some(("step.commandDenied", "Command denied", reason))
                    }
                    Authorization::Unavailable(reason) => Some((
                        "step.commandNeedsApproval",
                        "Command needs approval in the chat",
                        reason,
                    )),
                };
                if let Some((key, text, reason)) = denial {
                    on_step(&UiText::keyed(key, text), Some(action.as_str()));
                    actions_taken.push(ActionTaken {
                        action: "terminal_exec".into(),
                        result: "denied".into(),
                        data: Some(json!({
                            "command": command,
                            "reason": reason,
                            "denied": true,
                        })),
                    });
                    let note = format!("terminal_exec was not run: {reason}");
                    continuation_messages.push(("user".to_string(), format!("[Done] {note}")));
                    completed_steps.push(note);
                    continue;
                }
            }

            on_step(&ui_step(state, &action), Some(action.as_str()));

            match execute_action(state, traveler, &input.ctx, &action, &params).await {
            Ok(outcome) => {
                // Always carry the full payload so the frontend can act on it
                // (open a specific email/document, focus the touched item).
                actions_taken.push(ActionTaken {
                    action: outcome.action.clone(),
                    result: outcome.result.clone(),
                    data: Some(outcome.data.clone()),
                });

                if outcome.action == "navigate_to" && outcome.result == "ok" {
                    if let Ok(nav) = serde_json::from_value::<NavigationSession>(
                        outcome.data.get("navigator").cloned().unwrap_or(json!({})),
                    ) {
                        navigation = Some(nav);
                    }
                }

                if outcome.action == "show_plugin" && outcome.result == "ok" {
                    if let Some(p) = outcome.data.get("plugin").and_then(|v| v.as_str()) {
                        focus_plugin = Some(p.to_string());
                    }
                }

                if outcome.action == "generate_image" && outcome.result == "ok" {
                    open_file = outcome.data.get("open_file").cloned();
                }

                // Generated media is its own chat element, persisted as a
                // `media` row and re-rendered when the conversation is opened
                // again. The reply's markdown copy is stripped below, so it is
                // never shown twice.
                if outcome.result == "ok" {
                    let media = match outcome.action.as_str() {
                        "generate_image" => outcome
                            .data
                            .get("url")
                            .and_then(Value::as_str)
                            .map(|url| {
                                json!({
                                    "kind": "image",
                                    "url": url,
                                    "file": outcome.data.get("file").cloned().unwrap_or(Value::Null),
                                    "path": outcome.data.get("path").cloned().unwrap_or(Value::Null),
                                })
                            }),
                        "generate_song" => outcome
                            .data
                            .get("url")
                            .and_then(Value::as_str)
                            .map(|url| {
                                json!({
                                    "kind": "song",
                                    "url": url,
                                    "file": outcome.data.get("file").cloned().unwrap_or(Value::Null),
                                    "path": outcome.data.get("path").cloned().unwrap_or(Value::Null),
                                    "title": outcome.data.get("title").cloned().unwrap_or(Value::Null),
                                })
                            }),
                        _ => None,
                    };
                    if let Some(media) = media {
                        turn_events.push(("media".into(), media.to_string()));
                    }
                }

                let mut produced: Vec<Artifact> = Vec::new();
                if let Some(art) = outcome.artifact {
                    produced.push(art);
                }
                produced.extend(outcome.extra_artifacts);
                for art in produced {
                    if let Err(e) = crate::services::artifacts::save_artifact(
                        &state.pool,
                        &traveler.id,
                        trip_id,
                        &art,
                        outcome.owner.as_deref(),
                    )
                    .await
                    {
                        tracing::warn!("Failed to autosave artifact: {}", e);
                    }
                    // Tag the response payload so the UI can group by plugin
                    // without refetching the summary list.
                    let mut v = serde_json::to_value(&art).unwrap_or_else(|_| json!({}));
                    if let (Some(owner), Value::Object(map)) = (&outcome.owner, &mut v) {
                        map.insert("plugin".into(), json!(owner));
                    }
                    // Stream it now: the window a card belongs to may be what
                    // the next tool in this same turn has to drive.
                    on_artifact(v.clone());
                    artifacts.push(v);
                }

                let mut note = describe_tool_step(&outcome.action, &outcome.result, &outcome.data);

                // A Craft app the AI just opened exposes its MCP from now on:
                // start its server and hand the model its tools, so it can drive
                // the app in this same conversation instead of reaching for the
                // terminal. Focus mode wins: the host keeps only the focused
                // apps, so opening another one changes nothing there.
                if outcome.action == "show_plugin" && outcome.result == "ok" {
                    if let Some(name) = outcome.data.get("plugin").and_then(|v| v.as_str()) {
                        if crate::plugins::store::CraftApp::by_id(name).is_some() {
                            craft_open.insert(name.to_string());
                            state
                                .craft_mcp
                                .sync(
                                    &state.plugins,
                                    &state.craft_control,
                                    std::path::Path::new(&state.config.plugins_dir),
                                    &input.craft_home,
                                    &craft_open,
                                    &focused_craft,
                                )
                                .await;
                            let docs = state
                                .plugins
                                .tools()
                                .skills_markdown_for(&std::collections::BTreeSet::from([name
                                    .to_string()]));
                            if !docs.trim().is_empty() {
                                note.push_str(&format!(
                                    "\n\nApp '{name}' tools (available from now on):\n{docs}"
                                ));
                            }
                        }
                    }
                }

                // Keep a plugin's tool docs in the conversation after its tools
                // are used: the continuation prompt is slim, so without this the
                // model "forgets" calc_write after calc_create and replies
                // without filling the sheet. Injected once per plugin per run.
                if let Some(owner) = &outcome.owner {
                    if owner != "core" && injected_skills.insert(owner.clone()) {
                        // A plugin carries its own skills docs; a store-installed
                        // Craft app has none, so its MCP tools' docs stand in.
                        let skills = state.plugins.skills_for(owner);
                        let skills = if skills.trim().is_empty() {
                            state
                                .plugins
                                .tools()
                                .skills_markdown_for(&std::collections::BTreeSet::from([owner
                                    .clone()]))
                        } else {
                            skills
                        };
                        if !skills.trim().is_empty() {
                            // A Craft app's tools act on its engine, not on the
                            // window the user sees: the model has to save.
                            let reminder = if crate::plugins::store::CraftApp::by_id(owner).is_some() {
                                "\nThese tools edit the app's engine, not the window the user sees: save/export the document before answering and give the path."
                            } else {
                                ""
                            };
                            note.push_str(&format!("\n\nPlugin '{owner}' tools:{reminder}\n{skills}"));
                        }
                    }
                }

                // A freshly activated plugin is usable right away — hand the
                // model its skills docs so it can call the plugin's tools in
                // this same conversation.
                if outcome.action == "plugin_activate" && outcome.result == "ok" {
                    let already = outcome.data.get("already").and_then(|v| v.as_bool()).unwrap_or(false);
                    if !already {
                        if let Some(name) = outcome.data.get("plugin").and_then(|v| v.as_str()) {
                            let skills = state.plugins.skills_for(name);
                            if !skills.trim().is_empty() {
                                note.push_str(&format!(
                                    "\n\nPlugin '{name}' tools (available from now on):\n{skills}"
                                ));
                            }
                        }
                    }
                }

                continuation_messages
                    .push(("user".to_string(), format!("[Done] {note}")));
                completed_steps.push(note);
                // The model gets the full note above; the bubble gets a short
                // human line (never the raw payload).
                on_step(
                    &ui_done(state, &outcome.action, &outcome.result, &outcome.data),
                    Some(outcome.action.as_str()),
                );
            }
            Err(e) => {
                actions_taken.push(ActionTaken {
                    action: action.clone(),
                    result: "error".into(),
                    data: None,
                });
                let note = describe_tool_step(&action, "error", &json!({ "error": e.to_string() }));
                continuation_messages
                    .push(("user".to_string(), format!("[Done] {note}")));
                completed_steps.push(note);
                let step = failed_label_for_action(&action);
                on_step(&UiText::keyed(step.key, step.text), Some(action.as_str()));
            }
            }
        }

        if interrupted {
            break;
        }
    }

    // Generated media is a conversation row of its own: drop its markdown from
    // the answer so the live view and every reopen show it exactly once.
    final_reply = crate::services::media::strip_generated_media(&final_reply);

    // A stopped turn has no answer yet: leave it empty so the caller stores an
    // honest "no reply was produced" rather than a fake "Done.".
    if final_reply.is_empty() && !interrupted {
        final_reply = "Done.".into();
    }

    Ok(AgentRunResult {
        success: true,
        reply: final_reply,
        mode: input.mode,
        artifacts,
        actions_taken,
        navigation,
        focus_plugin,
        open_file,
        steps: completed_steps,
        thinking: thinking_log,
        events: turn_events,
        interrupted,
    })
}
