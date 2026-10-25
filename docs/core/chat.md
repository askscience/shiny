# Chat & conversations

Shiny stores only **text** — never audio. A user's chat is organised into
**conversations** (resumable threads) rather than one flat history, so the agent
keeps each thread's context.

Source: [`src/services/chat_memory.rs`](../../src/services/chat_memory.rs),
[`src/api/chat.rs`](../../src/api/chat.rs),
migrations [`007_chat_conversations.sql`](../../migrations/007_chat_conversations.sql)
and [`008_chat_indexes.sql`](../../migrations/008_chat_indexes.sql).

---

## Tables

| Table | Columns (essence) |
|---|---|
| `chat_conversations` | `id`, `traveler_id`, `title`, `created_at`, `updated_at`, `terminal_auto_approve` |
| `chat_messages` | `id`, `traveler_id`, `conversation_id`, `role`, `content`, `thinking`, `timestamp` |

`role` is `user`, `assistant`, `system`, `approval` or `media`.
**`system` rows are invisible notes** written for the model; the conversation
list/detail endpoints filter them out, so the user never sees them as bubbles.
`approval` and `media` are mid-turn UI rows: a terminal command (JSON
`{command, decision}`) and a generated image/track (JSON `{kind, url, file,
path, title?}`). They store as `content` JSON, are rendered by the chat, and are
**excluded from the agent's memory window** so the model only reads real turns.
`GET /api/chat/conversations` previews the last `user`/`assistant` row.

`thinking` (on assistant rows) is the reasoning trace a thinking model produced
for that reply, when thinking was on. It is returned by
`GET /api/chat/conversations/:id` and rendered as a collapsed "Thinking"
disclosure above the answer — the same fold the live turn shows.

A turn's rows are inserted in visual order in one transaction: the user message,
any `approval` rows, the assistant reply, then any `media` rows. That is what
makes a reopened chat show the terminal commands and generated pictures in the
same place the live turn showed them.

---

## The two chat paths

### 1. Legacy one-shot chat (`/api/chat`)

A simple companion endpoint kept for compatibility:

- `POST /api/chat` inserts the user message, loads the 5 most recent diary
  entries as context, builds a "helpful travel companion" system prompt, sends
  the last 20 messages to the resolved AI provider, stores the reply, and
  returns `{ success, reply, diary_context_used }`.
- `GET /api/chat/history?limit=N` returns the flat history (default 50).

This path does **not** use plugins/tools. The full agent is `/api/agent`
(see [agent](agent.md)).

### 2. Agent conversations (`/api/agent` + `/api/chat/conversations`)

The real chat. Each turn is attached to a conversation:

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/api/chat/conversations` | List the caller's conversations (title, preview, updated_at). |
| `POST` | `/api/chat/conversations` | Create an empty conversation (`New chat`). |
| `GET` | `/api/chat/conversations/:id` | Messages of one conversation (system rows hidden). |
| `DELETE` | `/api/chat/conversations/:id` | Delete the conversation and its messages. |

`resolve_conversation` returns the requested id if it exists and belongs to the
user, otherwise creates a new one. `recent_history` loads the most recent
messages oldest-first, tie-breaking on `rowid` because a turn's user and
assistant rows share a second-resolution timestamp.

---

## Titles

On the **first** turn of a conversation whose title is still `New chat` (or
empty), Shiny asks the model for a three-word, lowercase title; if the AI is
unavailable it falls back to the first ~48 characters of the user message. Later
turns only bump `updated_at`.

---

## The stopped-turn note

When a user stops a reply, the conversation records an **invisible** `system`
note (`INTERRUPTED_NOTE`) so the next request knows the previous answer was cut
short. Two paths:

- The turn was stopped mid-generation → `save_turn_with_note` writes the note
  with the turn.
- The answer had already been saved and the user stopped while reading/hearing
  it (`append_note`) → the note is appended to the last conversation.

The agent reads every role back into its history prompt; the chat endpoints
exclude `system`. This is what prevents the model from being confused by a
truncated thread.

---

## Memory window

The agent sends a bounded window of recent turns (rather than the whole thread)
to the model, so long conversations stay within context. See
[agent](agent.md) for the exact window and prompt assembly. `008_chat_indexes.sql`
adds the indexes that keep conversation lookups fast.

---

## Frontend

[`web/js/chatHistory.js`](../../web/js/chatHistory.js) renders the conversation
list and message bubbles, and [`web/js/agent.js`](../../web/js/agent.js) drives
the streaming turns. The compose panel itself is
[`web/js/textInput.js`](../../web/js/textInput.js): the pending bubble's spinner
plus live step line, the collapsed "Thinking" fold, generated-image/song blocks,
the terminal **command approval card** (deny / allow once / allow for this chat),
and the close button. [`web/js/thinkToggle.js`](../../web/js/thinkToggle.js) owns
the chat head's brain button (and the voice-bar menu entry) — it shows
`ui/brain` while thinking is on and the slashed `ui/brain-off` when it is off,
and disables itself for a model that cannot think. See [web UI](web-ui.md).

The approval card is a live thread element, not a modal: it appears above the
composer exactly where the command will run, shows the command verbatim, and
becomes an inert green circle-check (allowed) or red circle-cross (denied) once
answered — no status text. Both the command and its outcome are stored as an
`approval` row, so closing and reopening the chat shows the same command block
and icon again. Generated images and songs are stored as `media` rows and
re-rendered below the answer on reopen; the reply's markdown copy is stripped
server-side so nothing appears twice.
