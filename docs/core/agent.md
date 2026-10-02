# The agent loop and tool dispatch

Shiny's "AI assistant" is a single agent loop: the user speaks or types, the model
is asked to either call one tool or answer in plain language, the core executes
the tool, feeds the result back, and repeats until the model stops calling
tools or the turn is stopped. Everything domain-specific (trips, mail, word
processor, radio, …) is a **plugin tool**; the core owns only the loop, the
dispatch machinery, and a handful of built-in control actions.

This document follows one turn from HTTP request to saved reply. It covers the
system-prompt assembly, action parsing, the registry dispatch path, the
continuation loop, stop/cancel, step labels and humanization, artifact
autosave, notifications, and provider resolution.

Related reading: [Chat & memory](chat.md),
[Voice](voice.md) (spoken-reply rules),
[Plugins](../plugins/README.md),
[Desktop](desktop.md) (workspaces/window control),
[Web UI](web-ui.md) (the frontend turn state machine),
and the REST reference in [Chat & agent API](../api/chat-agent.md).

---

## 1. At a glance

| Piece | File | Role |
|---|---|---|
| HTTP entry points | `src/api/agent.rs` | `AgentRequest` parsing, system-prompt assembly, JSON + SSE handlers, stop handler, turn persistence. |
| Run loop | `src/services/agent_runner.rs` | `run_agent`: iteration, cancellation, tool execution, artifacts, result assembly. |
| Prompt shapes & labels | `src/services/agent_steps.rs` | Planning/continuation prompts, `describe_tool_step`, step/done/failed labels, `thinking_label`. |
| Action parsing & dispatch | `src/services/agent_tools.rs` | `parse_actions`, `strip_action_blocks`, `execute_action`, core built-ins. |
| Cancellation | `src/services/agent_cancel.rs` | `TurnRegistry`, `CancelHandle`, `StopOutcome`, late-stop annotation. |
| Provider resolution | `src/services/ai.rs` | `AiClient`, `ResolvedAi`, `resolve_for_user`. |
| Providers | `src/services/openai.rs`, `crates/shiny-plugin-sdk/src/services.rs` | OpenAI-compatible and Ollama clients. |
| Memory | `src/services/chat_memory.rs` | Conversations, history, turn saving, interrupted notes. |
| Artifacts | `src/services/artifacts.rs` | Autosave, summaries, merge updates. |
| Tool trait / registry | `crates/shiny-plugin-sdk/src/tools.rs`, `src/plugins/registry.rs` | `Tool`, `ToolRequest`, `parse_actions`, aliases, `bridged()`. |
| Traveler plugin fallback | `src/services/agent_tools.rs` | Refusal of traveler verbs when the plugin is unloaded/deactivated. |

The loop is deliberately small. Its constants:

```rust
// src/services/agent_runner.rs
const MAX_TOOL_STEPS: usize = 40;
```

---

## 2. Request entry points

The live route table is in `src/api/mod.rs`:

```rust
.route("/api/agent", post(agent::handle_agent_dispatch))
.route("/api/agent/stop", post(agent::handle_agent_stop))
```

All routes below `protected_routes` in `src/api/mod.rs` are wrapped by
`auth_middleware` (`src/auth/mod.rs`), which resolves a `Traveler` from either
the `Authorization: Bearer <token>` header or the `shiny_token` cookie (plus a
loopback-only session token used by the local kiosk). The resolved `Traveler`
arrives in the handler as an axum `Extension`.

`handle_agent_dispatch` chooses the transport by the request's `stream` field:

```rust
pub async fn handle_agent_dispatch(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
    Json(body): Json<AgentRequest>,
) -> Result<axum::response::Response, AppError> {
    if body.stream.unwrap_or(false) {
        let sse = handle_agent_stream(State(state), Extension(traveler), Json(body)).await?;
        Ok(sse.into_response())
    } else {
        let json = handle_agent(State(state), Extension(traveler), Json(body)).await?;
        Ok(json.into_response())
    }
}
```

| Mode | Handler | Transport |
|---|---|---|
| `stream: false` / omitted | `handle_agent` | One `application/json` response. |
| `stream: true` | `handle_agent_stream` | `text/event-stream` SSE: `step` events then `done`/`error`. |

The frontend always sets `stream: true` (`web/js/agent.js`) and parses the SSE
stream itself; see [§9](#9-sse-vs-json-modes).

### Request shape

`AgentRequest` (`src/api/agent.rs`):

| Field | Type | Meaning |
|---|---|---|
| `message` | string (required) | The user's turn. |
| `mode` | string? | Free-form mode label; defaults to `"single"`. Echoed back in the response and injected into prompts. |
| `lang` | string? | ISO language code; defaults to `"en"`. Converted to a language name for the prompt. |
| `ai_name` | string? | Assistant persona name; defaults to `"Shiny"`. |
| `ollama_model` | string? | Per-request model override (Ollama only). |
| `stream` | bool? | Selects SSE vs JSON. |
| `context` | object? | `{lat, lon, heading}` — live location. |
| `desktop` | object? | `{active, workspaces:[{index, windows:[]}], workspaces_enabled}`. |
| `conversation_id` | string? | Continue this thread; omit to start a new one. |
| `voice` | bool (default `false`) | Spoken reply constraints. |
| `turn_id` | string? | Client-generated id for stop/cancel. |

### Stop request

`POST /api/agent/stop` takes `{turn_id, conversation_id?}` and is handled by
`handle_agent_stop` (see [§8](#8-stop--cancel)).

---

## 3. Preparing a turn (`prepare_agent`)

`prepare_agent` (`src/api/agent.rs`) does everything that does not depend on the
model: defaults, context, provider-independent model selection, the active
plugin set, the skill/persona/context blocks, the plugin catalog, the desktop
snapshot, and conversation memory. It returns a `PreparedAgent`:

```rust
struct PreparedAgent {
    input: AgentRunInput,
    trip_id: Option<String>,
    conversation_id: String,
    requested_model: Option<String>,
    turn_id: Option<String>,
}
```

### 3.1 Defaults and context

```rust
let mode = body.mode.unwrap_or_else(|| "single".into());
let lang = body.lang.unwrap_or_else(|| "en".into());
let voice = body.voice;
let requested_model = /* trimmed body.ollama_model, or None */;
let ctx = AgentContext {
    lat, lon, heading,
    lang: lang.clone(),
    ollama_model: requested_model.clone(),
    workspaces_enabled: body.desktop...workspaces_enabled.unwrap_or(true),
};
```

`AgentContext` is defined in the SDK (`crates/shiny-plugin-sdk/src/context.rs`)
and is handed to every tool invocation. `workspaces_enabled` defaults to `true`
so older clients keep the wide-screen behaviour; a vertical, phone-like screen
sends `false`.

### 3.2 Active plugin set

```rust
let installed: BTreeSet<String> = state.plugins.session_active_set(&traveler.id).await;
```

`session_active_set` (`src/plugins/manager.rs`) is governed by the
`session.remember` preference:

- **remember on** — installed plugins minus those explicitly disabled.
- **remember off** — only plugins with an explicit `enabled = 1` row (opt-in,
  starts empty and grows as the user or the AI activates plugins).

The active set decides which plugins' skills, persona fragments and context
lines enter the system prompt, and which plugin tools are callable.

### 3.3 Traveler context

When `traveler` is active, the core adds the active trip and the last three
diary entries:

```rust
let active_trip = fetch_active_trip(&state.pool, &traveler.id).await?;
let recent_diary = /* date, summary FROM diary_entries ORDER BY date DESC LIMIT 3 */;
```

These become the `trip_line` and `diary_line` context lines. When `traveler` is
not active they collapse to `No active trip` / `No recent diary entries`.

### 3.4 Skills, persona, context lines

```rust
let core_skill = load_core_skill();                       // web/skills/core-assistant.md
let plugin_skill = state.plugins.skills_markdown_for(&installed);
let skill = if plugin_skill.trim().is_empty() { core_skill }
            else { format!("{core_skill}\n\n---\n\n{plugin_skill}") };

let plugin_persona = state.plugins.persona_concat_for(&installed);
let persona = if plugin_persona.trim().is_empty() { "a helpful AI assistant".to_string() }
              else { plugin_persona };

let context_lines = state.plugins.context_lines_for(&installed);
```

- `load_core_skill` reads `web/skills/core-assistant.md` from disk at request
  time (so edits are picked up without a restart), falling back to
  `"Use JSON action blocks."`.
- `skills_markdown_for` concatenates each active plugin's `skills_md` fragment
  plus every active tool's `doc_fragment()`.
- `persona_concat_for` joins the active plugins' one-line persona fragments.
- `context_lines_for` collects active plugins' arbitrary `context` lines
  (e.g. an "Active trip" line) and is rendered as a `## Context` addition.

### 3.5 The full plugin catalog

`catalog` lists **every installed plugin** (active and inactive) with its
description/summary and active flag. It drives three blocks:

- `active_lines` / `inactive_lines` — for the `## Plugins` catalog block.
- `plugin_windows_block` — a `## Plugin windows` block that tells the model how
  to surface an active plugin's window (`show_plugin`), and either documents
  the workspace tools or forbids them on a vertical screen.
- `plugins_hint` — a compact one-line catalog ("`traveler: Trip tracking;
  hello: Demo`", inactive plugins suffixed `(inactive)`) passed to the
  continuation prompt so the model can still activate/deactivate mid-loop.

### 3.6 Desktop block

`desktop_state_block` (`src/api/agent.rs`) turns the client's `desktop` snapshot
into a `## Desktop (current layout)` section. On a vertical screen
(`workspaces_enabled == false`) it says the desktop is a single column and the
workspace tools do nothing. On a wide screen it lists `workspace N: windows…`
and the active workspace, and advises moving windows into existing workspaces
rather than creating empty ones.

---

## 4. The run loop (`run_agent`)

`run_agent` (`src/services/agent_runner.rs`) is called identically by the JSON
and SSE handlers. Its inputs:

```rust
pub struct AgentRunInput {
    pub message: String,
    pub mode: String,
    pub lang: String,
    pub ai_name: String,
    pub system: String,          // the assembled planning system prompt
    pub plugins_hint: String,    // compact catalog for continuations
    pub history: Vec<(String, String)>,
    pub voice: bool,
    pub ctx: AgentContext,
}
```

Return value:

```rust
pub struct AgentRunResult {
    pub success: bool,           // always true on Ok
    pub reply: String,
    pub mode: String,
    pub artifacts: Vec<Value>,   // tagged with "plugin"
    pub actions_taken: Vec<ActionTaken>,
    pub navigation: Option<NavigationSession>,
    pub focus_plugin: Option<String>,
    pub steps: Vec<String>,      // full LLM-facing notes
    pub interrupted: bool,
}
```

### 4.1 Iteration skeleton

```rust
on_step(thinking_label());

for iteration in 0..MAX_TOOL_STEPS {
    if turn_stopped(&cancel) { interrupted = true; break; }

    let messages = if completed_steps.is_empty() && iteration == 0 {
        build_planning_messages(&input.system, &input.message)
    } else {
        build_continuation_messages(
            &input.ai_name, &input.lang, &input.mode, &input.message,
            &completed_steps, &input.plugins_hint, &input.history, input.voice,
        )
    };

    let size = messages_char_count(&messages);
    if size > 200_000 { tracing::warn!("Agent prompt very large …"); }

    let response = /* ai.chat(messages, model), cancel-aware */;

    let actions = parse_actions(&response);
    if actions.is_empty() {
        final_reply = strip_action_blocks(&response);
        // … fallbacks …
        break;
    }

    for (action, params) in actions { /* execute */ }

    if interrupted { break; }
}
```

Key behaviours:

- The **first** model call uses the full planning prompt (system + user). Every
  later call uses the slim continuation prompt.
- `MAX_TOOL_STEPS = 40` caps how many model round-trips one turn may take.
- A prompt over 200 000 characters logs a warning but still proceeds ("slim
  context" is a debugging aid, not a hard cap).
- `on_step` is the callback that powers SSE `step` events and the dock status
  line.

### 4.2 The cancellation-aware model call

```rust
let response = match &cancel {
    Some(handle) => tokio::select! {
        biased;
        _ = handle.cancelled() => { interrupted = true; break; }
        result = ai.chat(messages, model) => result?,
    },
    None => ai.chat(messages, model).await?,
};
```

A local model that keeps generating blocks the next question, so a stopped turn
must **abandon** the in-flight HTTP call, not merely ignore its answer.
`tokio::select!` with `biased` makes cancellation win immediately. The full
model output is logged (first 1200 chars) at `info` level so tool-call failures
are visible in `data/shiny.log`.

### 4.3 No actions = final reply

```rust
if actions.is_empty() {
    final_reply = strip_action_blocks(&response);
    if final_reply.is_empty() {
        let trimmed = response.trim();
        final_reply = if trimmed.starts_with('{') || trimmed.starts_with('[') {
            "Done.".into()
        } else {
            trimmed.to_string()
        };
    }
    break;
}
```

A plain-language answer ends the turn. If the model emitted only malformed JSON,
the raw braces are stripped and a neutral `Done.` is used rather than leaking a
tool call into the chat. If the model returned an empty reply, the final fallback
(after the loop) is also `Done.` — unless the turn was interrupted.

### 4.4 Executing actions

Every tool call the model emitted is executed **in order** — multi-step intents
("close all plugins", "create then fill a document") run sequentially rather
than silently dropping everything after the first action.

```rust
for (action, params) in actions {
    if turn_stopped(&cancel) { interrupted = true; break; }

    // Workspaces do not exist on a vertical screen.
    if action.starts_with("workspace_") && !input.ctx.workspaces_enabled {
        on_step("Workspaces are unavailable on this screen");
        continue;
    }

    on_step(&ui_step_label(state, &action));       // e.g. "Searching the web…"

    match execute_action(state, traveler, &input.ctx, &action, &params).await {
        Ok(outcome) => { /* see 4.5 */ }
        Err(e) => {
            actions_taken.push(ActionTaken { action: action.clone(), result: "error".into(), data: None });
            completed_steps.push(describe_tool_step(&action, "error", &json!({ "error": e.to_string() })));
            on_step(failed_label_for_action(&action));
        }
    }
}

if interrupted { break; }
```

The stop is re-checked **between actions**, so a slow tool (a download, a model
call of its own) does not force the stop to wait for the whole batch.

The workspace guard fires before dispatch and emits an explicit step. On a
phone-like screen the core refuses `workspace_*` rather than claiming a
workspace was created. (The tool implementations also guard, returning
`result: "unavailable"`; the pre-check avoids even that.)

### 4.5 Handling a successful outcome

For each `ActionOutcome` the runner:

1. **Records the action** in `actions_taken`, always with the full `data` payload
   so the frontend can act on it (open a specific email/document, focus an item).
2. **Extracts navigation** — `navigate_to` + `result == "ok"` deserialises
   `data.navigator` into `NavigationSession` (`src/services/navigation.rs`,
   `crates/shiny-plugin-sdk/src/navigation.rs`).
3. **Extracts focus** — `show_plugin` + `result == "ok"` sets
   `focus_plugin = data.plugin`.
4. **Saves artifacts** — `outcome.artifact` plus `outcome.extra_artifacts` are
   autosaved via `artifacts::save_artifact`, owner-tagged. Failures are logged
   (`Failed to autosave artifact`) and never fail the turn.
5. **Tracks produced owners** — a non-`core` owner that produced cards is
   remembered in `produced_owners` so the window is surfaced even if the model
   never called `show_plugin`.
6. **Tags the response payload** — each artifact value gets a `"plugin"` key
   so the UI can group surfaces by plugin without refetching.
7. **Builds the LLM note** with `describe_tool_step`.
8. **Injects plugin skills** — the first time a plugin's tool is used in a run
   (`injected_skills`), its `skills_for(owner)` docs are appended to the step
   note so the model "remembers" the plugin's tools in the slim continuation
   prompt. This is why a `calc_create` can be followed by a `calc_write` that
   actually fills the sheet.
9. **Injects activation skills** — after a successful `plugin_activate` that
   was not already active, the plugin's skills are appended so its tools are
   usable immediately in the same conversation.
10. **Pushes the note** into `completed_steps` and emits the short human "done"
    line with `ui_done_label`.

```rust
let mut note = describe_tool_step(&outcome.action, &outcome.result, &outcome.data);
if let Some(owner) = &outcome.owner {
    if owner != "core" && injected_skills.insert(owner.clone()) {
        let skills = state.plugins.skills_for(owner);
        if !skills.trim().is_empty() { note.push_str(&format!("\n\nPlugin '{owner}' tools:\n{skills}")); }
    }
}
completed_steps.push(note.clone());
on_step(&ui_done_label(state, &outcome.action, &outcome.result, &outcome.data));
```

### 4.6 Final fallbacks

After the loop:

```rust
if final_reply.is_empty() && !interrupted {
    final_reply = "Done.".into();
}
if focus_plugin.is_none() && produced_owners.len() == 1 {
    focus_plugin = produced_owners.into_iter().next();   // deterministic surface
}
```

- A stopped turn keeps an **empty** reply so the caller records "no reply was
  produced" rather than a fake `Done.`.
- Cards from exactly one plugin surface that plugin's window even if the model
  forgot `show_plugin`.

---

## 5. Prompt shapes and step notes

All of these live in `src/services/agent_steps.rs`.

### 5.1 Planning prompt

```rust
pub fn build_planning_messages(full_system: &str, user_message: &str) -> Vec<(String, String)> {
    vec![
        ("system".into(), full_system.to_string()),
        ("user".into(), user_message.to_string()),
    ]
}
```

The full system prompt is used **only** for the first model call. It carries the
whole tool reference, which is why later calls must be slim.

### 5.2 Continuation prompt

After the first tool, `build_continuation_messages` produces a tiny prompt: a
system line plus the user message plus one `[Done] …` note per completed step.

```text
You are {ai_name}. Language: {lang}. Mode: {mode} — answer fully and clearly; be concise for simple questions but give detail when helpful.
{voice_line}{doc_line}Call exactly ONE tool per turn (raw JSON line, no markdown) or reply in plain language if done.
Format: {"action":"tool_name","params":{...}}
{plugins_line}{history_line}
```

- `voice_line` re-applies the spoken-prose rules; the final reply is generated
  from **this** prompt, not the planning one.
- `doc_line` always repeats the "confirm a document in one or two short
  sentences, do not restate it" rule.
- `plugins_line` teaches `show_plugin` and `plugin_activate` from the compact
  `plugins_hint`; omitted when empty.
- `history_line` renders earlier turns as `role: content` lines; omitted when
  empty.
- Completed steps are appended as `("user", "[Done] {note}")` messages.

A unit test (`continuation_omits_full_skill_doc`) pins that the continuation
system prompt stays under 1000 chars and never repeats `## Tool call format`.

### 5.3 `describe_tool_step`

`describe_tool_step(action, result, data) -> String` builds the **full**,
LLM-facing note for a completed step. Rules:

- `result == "error"` → `"{action} failed: {error|message|unknown error}"`.
- any other non-`"ok"` result → `"{action}: {result}"`.
- known core actions get a structured, readable summary.

Known cores (selected):

| Action | Note |
|---|---|
| `plan_trip` | `Planned trip to {dest} — {n} guides ({km} km drive)` |
| `navigate_to` | `Navigating to {dest}` |
| `list_trips` | `Listed {n} trip(s)` |
| `web_search` | `Web search completed: {summary}` plus up to 4 `- title: snippet` lines |
| `youtube_search` | `YouTube results` plus up to 4 `- title — channel` lines |
| `calc_read` | Spreadsheet title + sorted `A1: value` lines |
| `calc_write` | `Wrote {n} cells to "{title}"` |
| `mail_read` | `Email from {from} ({date}): {subject}` + body + attachments |
| `mail_list` / `mail_search` | `- id=… | subject | from | date` lines |
| `doc_read` | `Document "{title}" (id=…): …` |
| `doc_create`/`doc_write`/`doc_edit`/`doc_append` | `{action} document "{title}" (id=…)` |
| `slide_read`, `studio_get` (and friends) | title + id + payload |
| `plugin_activate`/`plugin_deactivate` | activated / already active / error |
| `list_plugins` | `Plugins:` + `name (active\|inactive) — description` |
| `show_plugin` | `Showing {name}` |
| `show_artifact`/`update_artifact` | `Updated {title}` |

Unknown/plugin actions fall through to:
`"{action} complete: {serialized data}"` (or `"{action} complete"` when the data
is `{}`). This hands the **full** payload to the model so nothing is lost.

Helpers: `workspace_display_number` (0-based data → 1-based UI) and
`cell_ref_order` (A1, B1, … A2, B2 sorting for `calc_read`).

### 5.4 Voice and long-form blocks

- `voice_style_block(true)` (planning only) bans all markdown, lists, headings,
  tables, code blocks and emoji, and asks for short spoken sentences.
- `LONG_FORM_BLOCK` is a `const &str` included in **both** voice and text modes:
  write the complete deliverable inside the tool call; never stub or outline;
  invent sensible structure rather than asking; keep adding sections across
  tool calls; confirm in ≤2 sentences and never restate the document.

---

## 6. Stop, cancel and interruption

The frontend generates a `turn_id` per request. On `/api/agent` the runner
registers it in the `TurnRegistry` (`src/services/agent_cancel.rs`) and holds a
cancellation handle for the duration.

- `POST /api/agent/stop { turn_id, conversation_id? }` calls
  `TurnRegistry::cancel`. The outcome is `Cancelled` (the live turn was
  aborted), `Annotate` (the turn already finished — e.g. the user stopped while
  reading/hearing it) or neither (unknown/already annotated).
- A live cancellation drops the in-flight model call instead of letting it
  finish; the partial answer is kept.
- Either path writes the invisible `system` note
  (`chat_memory::INTERRUPTED_NOTE`) so the next request knows the reply was cut
  short. See [chat](chat.md#the-stopped-turn-note).
- The response reports `{ cancelled, annotated }`, and the turn response carries
  `interrupted: true` when it was stopped.

`voice:barge-in` (see [voice](voice.md#barge-in)) is what triggers this from the
microphone.

## 7. Artifacts, notifications & navigation

- A tool may attach an **artifact** (`with_artifact` / `with_extra_artifacts`);
  the runner collects them per turn and the UI renders them in the owning
  plugin's window/dock. Saved artifacts are tagged with the plugin name.
- `with_notification` puts a GNOME-style banner in
  `data["notification"]`; the frontend renders it with no ABI change.
- `with_navigation` attaches a `NavigationSession` that the navigator HUD
  consumes.

## 8. Providers & models

`AppState::resolve_ai(traveler_id)` returns the user's provider — the shared
Ollama client by default, or an OpenAI-compatible client when configured in the
user's Assistant settings (`src/services/ai.rs`, `ollama.rs`, `openai.rs`). A
per-request `ollama_model` overrides the persisted model for Ollama; the OpenAI
client bakes its model in. `GET /api/ai/models` lists the resolved provider's
models; `GET /api/ollama/models` reads Ollama's `/api/tags`.

## 9. Skills markdown assembly

The `## Tools` block is the concatenation of:

1. `web/skills/core-assistant.md` (always-on core tools),
2. each active plugin's `skills_md` (set with `builder.skills(...)`),
3. every registered tool's `doc_fragment()`, deduplicated by tool name.

A tool that returns no `doc_fragment` stays hidden. Deactivated plugins
contribute nothing (no persona, skills or context lines).

## 10. Source map

| Path | Role |
|---|---|
| [`src/services/agent_runner.rs`](../../src/services/agent_runner.rs) | The turn loop, parsing, dispatch, continuation. |
| [`src/services/agent_steps.rs`](../../src/services/agent_steps.rs) | Continuation prompt, `describe_tool_step`, voice/long-form blocks. |
| [`src/services/agent_tools.rs`](../../src/services/agent_tools.rs) | Core built-in actions, `AgentContext`, active-trip fetch. |
| [`src/services/agent_cancel.rs`](../../src/services/agent_cancel.rs) | `TurnRegistry` / stop semantics. |
| [`src/services/ai.rs`](../../src/services/ai.rs) | Provider resolution. |
| [`src/api/agent.rs`](../../src/api/agent.rs) | System-prompt assembly, JSON/SSE handlers, stop. |
| [`src/plugins/registry.rs`](../../src/plugins/registry.rs) | Tool dispatch + activation gate. |
| [`crates/shiny-plugin-sdk/src/tools.rs`](../../crates/shiny-plugin-sdk/src/tools.rs) | `parse_actions`, `strip_action_blocks`, `bridged`. |
| [`web/skills/core-assistant.md`](../../web/skills/core-assistant.md) | Core tool docs. |
| [`web/js/agent.js`](../../web/js/agent.js) | Frontend turn driver, barge-in, stop. |

Related: [chat](chat.md) · [voice](voice.md) · [API chat & agent](../api/chat-agent.md).
