# API — chat, agent, search, artifacts, insights

Source: [`src/api/agent.rs`](../../src/api/agent.rs),
[`chat.rs`](../../src/api/chat.rs),
[`search.rs`](../../src/api/search.rs),
[`artifacts.rs`](../../src/api/artifacts.rs),
[`insights.rs`](../../src/api/insights.rs),
[`ai.rs`](../../src/api/ai.rs), [`ollama.rs`](../../src/api/ollama.rs).

See also [agent](../core/agent.md), [chat](../core/chat.md).

---

## `POST /api/agent`

The main agent entry point. Runs one turn. `stream: true` returns SSE instead of
JSON (same handler dispatches on the flag).

Request:

```jsonc
{
  "message": "Make me a 3-day Rome itinerary",
  "mode": "single",              // free-form mode label, default "single"
  "lang": "en",                  // reply language (ISO-639-1)
  "ai_name": "Shiny",            // persona name
  "ollama_model": "llama3",      // optional per-request model (Ollama only)
  "think": true,                 // Ollama reasoning switch (thinking models)
  "stream": false,
  "voice": false,                // true ⇒ spoken prose, no markdown
  "correction": false,           // true ⇒ this message corrects the previous one
  "turn_id": "<uuid>",           // client id, for /api/agent/stop
  "conversation_id": "<uuid>",   // continue an existing thread
  "approve_tools": true,         // this client can render the terminal approval card
  "context": { "lat": 41.9, "lon": 12.5, "heading": 180 },
  "desktop": { "active": 1, "workspaces_enabled": true,
               "workspaces": [ { "index": 1, "windows": ["word", "files"] } ],
               "focused": ["word"] }
}
```

`desktop` lets the model reorganize windows without creating empty workspaces;
`workspaces_enabled: false` marks a vertical/phone screen (one column, no
workspaces). `focused` lists the plugin windows the user has put in **focus
mode**: while it is non-empty the turn is scoped to those plugins — only web
search and their tools run, and a desktop action or another plugin's tool comes
back as an `unavailable` outcome. Everything is available again when the list is
empty. See [desktop.md § Focus mode](../core/desktop.md#focus-mode).

`correction: true` means the user stopped the previous answer to amend the
request — the voice client sets it when an interruption (barge-in, or a tap that
stops a turn) is followed by speech. The turn's prompt (`correction_block()` in
`agent_steps`) then frames the message against the request that was being
answered, so "no, intendo Milano" applies to it instead of arriving as a new,
unrelated question. The interrupted turn's invisible `system` note is already in
the history either way.

JSON response (`AgentResponse`):

```jsonc
{
  "success": true,
  "reply": "Here's your itinerary…",
  "mode": "single",
  "artifacts": [ /* tagged payloads, each with a "plugin" key */ ],
  "actions_taken": [ { "action": "create_trip", "result": "ok", "data": {…} } ],
  "steps": ["planning…", "create_trip complete"],
  "navigation": { /* NavigationSession, when the turn started navigation */ },
  "focus_plugin": "traveler",
  "conversation_id": "<uuid>",
  "interrupted": false
}
```

### SSE mode (`stream: true`)

`Content-Type: text/event-stream`. Each `data:` line is a JSON object tagged by
`type`:

```jsonc
{ "type": "step", "message": "Searching the web…", "key": "step.searchingWeb", "action": "web_search" }
{ "type": "thinking", "message": "…reasoning trace…" }   // thinking models, when `think` is on
{ "type": "approval", "request_id": "<uuid>", "action": "terminal_exec", "command": "rm -rf ./build" }
{ "type": "done", "data": { /* the full AgentResponse */ } }
{ "type": "error", "message": "…" }
```

A `step` carries the catalogue `key` (and `params` when the message has
placeholders) next to the English `message`; the client renders
`t(key, params, message)` so the bubble follows the user's interface language
(see [interface language](../core/i18n.md)). Plugin tools that do not opt in
send `message` only. `action` is the tool the line belongs to (absent for the
thinking opener and for refusals with no action behind them): the client maps it
to a catalogue icon, so Settings → Appearance → voice bar can wear the tool's
mark instead of the line (`toolIcons.js`).

The stream sends keep-alives. The final `done` carries everything the JSON mode
returns; `AgentResponse.thinking` holds the turn's full trace (saved with the
reply, so the chat can fold it under the answer again later).

No thinking model — or thinking off — means no `thinking` events.

`approval` means the turn is **parked**: it is waiting for the user to allow or
deny a `terminal_exec` command. Answer it with `POST /api/agent/approve` (below).
Only a request sent with `approve_tools: true` can receive one; both the JSON
transport and voice turns without that flag refuse the command instead of
running it unapproved. Commands the hard-coded guard considers destructive never
produce an `approval` event at all — they are refused outright. See
[agent § Terminal command approval](../core/agent.md#46-terminal-command-approval).

---

### Language

Send `Accept-Language: <code>` (the frontend sends the user's interface
language on every request). Server-produced user-facing text — error messages,
plugin descriptions in `/api/plugins` — is translated for that language when a
catalogue ships it, and stays English otherwise.

### System prompt assembly

Built per request by `prepare_agent`:

- `web/skills/core-assistant.md` (always-on core tools), plus every active
  plugin's skills markdown joined with `\n---\n` — in focus mode, only the
  focused plugins' (see the `desktop.focused` field above).
- Persona: the concatenation of active plugins' persona fragments, or
  `a helpful AI assistant`.
- `## Context`: user first name, location (or "unknown"), active trip, recent
  diary summaries (when the `traveler` plugin is active), plus plugin
  `context_lines`.
- `## Conversation history`: the last 40 turns of the thread.
- `## Plugin windows` and `## Plugins`: the active catalog and the inactive
  catalog (so the model can `plugin_activate` on demand).
- `## Desktop (current layout)`: the workspace/window snapshot.
- Strict tool protocol: exactly one raw JSON action per turn.

The model is resolved per user (Ollama, or an OpenAI-compatible provider when
configured); a requested `ollama_model` overrides the persisted choice.

---

## `POST /api/agent/stop`

Stop an in-flight turn, or annotate one that already finished.

```jsonc
{ "turn_id": "<uuid>", "conversation_id": "<uuid>" }
```

Response:

```json
{ "success": true, "data": { "cancelled": true, "annotated": false } }
```

- `cancelled: true` — the live turn was aborted; the partial answer is kept and
  an invisible `system` note is recorded with the turn.
- `annotated: true` — the turn had already finished; an invisible note is
  appended to the conversation.
- Neither — unknown turn, no-op.

---

## `POST /api/agent/approve`

Answer the `approval` event a running turn is parked on.

```jsonc
{
  "turn_id": "<uuid>",        // the turn that sent the event
  "request_id": "<uuid>",     // from the `approval` event
  "decision": "allow"         // "allow" | "deny" | "allow_chat"
}
```

Response:

```json
{ "success": true, "data": { "allowed": true, "remembered": false } }
```

- `allow` runs this one command; `deny` refuses it (the model is told nothing
  ran and not to retry).
- `allow_chat` runs it **and** sets `chat_conversations.terminal_auto_approve`,
  so every later `terminal_exec` in that conversation skips the prompt. Other
  conversations still ask. Destructive commands are still refused by the
  hard-coded guard.
- A request that is unknown, already answered, or belongs to another traveler
  returns `{ "success": false, "data": { … } }` and changes nothing. Always a
  `200`: answering a prompt that expired with its turn is not an API error.

---

## Chat endpoints

See [chat](../core/chat.md) for semantics.

| Method | Path | Body / query | Response |
|---|---|---|---|
| `POST` | `/api/chat` | `{ "message": "…" }` | `{ success, reply, diary_context_used }` (legacy one-shot, diary-aware, no tools). |
| `GET` | `/api/chat/history?limit=N` | — | `{ success, data: [{ role, content, timestamp }] }` |
| `GET` | `/api/chat/conversations` | — | `{ success, data: [{ id, title, updated_at, preview }] }` |
| `POST` | `/api/chat/conversations` | — | `{ success, data: { id } }` |
| `GET` | `/api/chat/conversations/:id` | — | `{ success, data: [{ role, content, timestamp }] }` (system rows hidden; `approval` / `media` rows are JSON payloads the chat renders) |
| `DELETE` | `/api/chat/conversations/:id` | — | `{ success, data: { deleted: true } }` |

---

## `POST /api/search`

Authenticated. Runs a web search (DuckDuckGo) and, when Ollama is available, an
AI summary. Returns results as `{ success, data: … }`.

---

## Models

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/api/ai/models` | Models from the user's resolved provider (Ollama or OpenAI). |
| `GET` | `/api/ai/thinking` | Whether the resolved provider/model can emit a reasoning trace: `{provider, model, supported}`. Gates the chat's brain button and the voice-bar menu. |
| `GET` | `/api/ollama/models` | Ollama's `/api/tags` list, sorted. |

---

## Image generation

Local text-to-image with Z-Image Turbo or Qwen-Image 2.1 Turbo
(stable-diffusion.cpp). The assistant drives it with the core `generate_image`
tool; these routes back Settings → Images and the chat's `<img>` endpoint.

| Method | Path | Body / query | Response |
|---|---|---|---|
| `GET` | `/api/imagegen/status` | — | `{ success, data: { inventory, downloads } }` — per-model tiers, on-disk presence, live download bytes. |
| `POST` | `/api/imagegen/download` | `{ "quality": "fast" \| "balanced" \| "high" \| "max" \| "all", "model": "z-image" \| "qwen-image-2.1" }` (model optional, defaults to Z-Image) | `{ success, data: { status: "downloading" } }` — background; poll status. |
| `GET` | `/api/imagegen/images/:name` | — | The PNG itself (`image/png`; cookie or Bearer auth). Files live in the user's Files home under `Pictures/Generated/`. |

The default model is `z-image`; the default tier is `fast` (Q3_K) for both
models. Qwen-Image 2.1 weights are under the Qwen Research License
(non-commercial). Without downloaded models the tool returns a `needs_download`
result and the assistant asks in chat before fetching. Step caching (Cache-DiT)
is on by default at threshold 0.30 — see [configuration](../configuration.md).

---

## Song generation

Local text-to-song with YuE2 (HOT-Step-CPP's `ace-server`). The assistant
drives it with the core `generate_song` tool; these routes back Settings →
Music and the audio endpoint the chat's track player embeds.

| Method | Path | Body / query | Response |
|---|---|---|---|
| `GET` | `/api/songgen/status` | — | `{ success, data: { inventory, downloads } }` — engine path, per-tier presence, live download bytes. |
| `POST` | `/api/songgen/download` | `{ "quality": "q4" \| "q8" \| "all" }` | `{ success, data: { status: "downloading" } }` — background; poll status. |
| `GET` | `/api/songgen/tracks/:name` | — | The WAV itself (`audio/wav`; cookie or Bearer auth; range requests served as `206` windows). Files live in the user's Files home under `Music/Generated/`. |

The default tier is `q4` (Q4_K_M-imat). One `ace-server` process runs per song
(nothing stays resident), a render takes minutes, and without downloaded models
the tool returns a `needs_download` result — the assistant asks in chat before
fetching. The engine binary itself is a one-time install
(`scripts/install-yue2-engine.sh` or `YUE2_ENGINE`).

---

## Artifacts

Saved artifact cards, per user. Payloads are tagged with the owning plugin's
name (`plugin` key).

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/api/artifacts` | List saved artifacts. |
| `POST` | `/api/artifacts` | Create/save one. |
| `GET` | `/api/artifacts/:id` | Get one. |
| `PUT` | `/api/artifacts/:id` | Update one. |

See [agent](../core/agent.md) for how artifacts are emitted by tools.

---

## `GET /api/insights/context`

Authenticated. Destination insight cards (weather, local events, trip context)
for the current user/location. Component: [`src/services/insights/`](../../src/services/insights).
