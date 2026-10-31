# Plugin system

`shiny` is built around an **AI assistant** — a conversational agent driven by Ollama with the voice bar UI, voice (Vosk STT, Supertonic TTS), web search, and an artifact dock at its core. **Every domain beyond that lives in a plugin.**

This document explains the architecture, the trait surface, the installer workflow, and a worked example. After reading it you can write, build, package, install, and uninstall a plugin.

> **One-line summary:** drop a `.zip` or `.tar.gz` containing `plugin.toml` + `lib<my_plugin>.so` onto `POST /api/plugins/install` and the plugin's tools become callable by the live AI assistant — no restart needed.

---

## 1. Architecture

```
                  shiny (core binary)
            ┌──────────────────────────────┐
            │  AI assistant                │
            │  ├─ voice bar UI             │
            │  ├─ Vosk STT + Supertonic TTS│
            │  ├─ OllamaClient             │
            │  ├─ SearchService            │
            │  ├─ ArtifactType             │
            │  ├─ CronContributor          │
            │  │                           │
            │  ├─ ToolRegistry ◀──┐        │
            │  └─ PluginManager   │        │
            │                    │        │
       ┌─── dlopen ──────────────┘        │
       │                                  │
       ▼                                  │
  plugins/hello/                          │
  ├─ plugin.toml                          │
  ├─ libhello.so  ─▶ shiny_plugin_entry   │
  ├─ migrations/*.sql                     │
  └─ skills/*.md     ─▶ ToolRegistry ──────┘
```

The binary's core responsibilities:

| Owned by core | Owned by plugins |
|---|---|
| HTTP server (axum), auth middleware, Bearer tokens | Tool implementations |
| `OllamaClient`, `SearchService`, `SupertonicClient`, voice/STT plumbing | REST routes for the plugin's domain |
| The matching between **an LLM action block** and the right tool | Migrations (one or more `.sql` files) |
| The voice bar and the front-end shell | Skill markdown advertised to the LLM |
| `PluginManager`, `ToolRegistry`, installer, admin API | A **persona fragment** ("…a travel navigator AI…") |
| `data/plugins/install.log` (the install audit trail) | Front-end bundle under `web/` |

### Without any plugins the app is "just the AI assistant"

Start `shiny` with an empty `PLUGINS_DIR` and you get a voice bar that listens, speaks, replies, and has the built-in generic tools only (`web_search`, `show_artifact`, `update_artifact`). Everything else is opt-in.

---

## 2. Workspace layout

```
shiny/
├── Cargo.toml                       # [workspace] root, lists ./, SDK and the plugins
├── crates/
│   ├── shiny-plugin-sdk/             # public trait surface + shared types
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs                # re-exports + ctx_plugin! entry macro
│   │       ├── plugin.rs              # the `Plugin` trait + PLUGIN_ENTRY_SYMBOL
│   │       ├── tools.rs               # `Tool` trait + `RegistryBuilder`
│   │       ├── outcome.rs            # `ActionOutcome`
│   │       ├── context.rs            # `AgentContext` (per-request agent state)
│   │       ├── services.rs           # `PluginCtx` + lazy `OllamaClient`, `SearchService`, `SupertonicClient`, pool
│   │       ├── rt.rs                 # `bridge()` — plugin-owned Tokio runtime (§15)
│   │       ├── artifacts.rs          # `Artifact` value type + `build_from_params`
│   │       ├── navigation.rs        # `NavigationSession` (optional navigator payload)
│   │       ├── manifest.rs           # `Manifest` struct + `Manifest::new`
│   │       ├── routes.rs             # `RouteSpec` constructors, `ok`/`user_id`
│   │       ├── util.rs               # title/filename/DB-value/user-id helpers
│   │       ├── crons.rs              # `CronSpec`
│   │       └── migrations.rs         # per-plugin migration runner
│   ├── shiny-osm/                    # OSM client shared by core + traveler
│   └── peakd-core/                   # display-scale state shared by the shells
├── plugins/
│   ├── hello/                        # demo plugin (see §7 worked example)
│   │   ├── Cargo.toml
│   │   ├── plugin.toml
│   │   ├── migrations/001_init.sql
│   │   ├── skills/hello.md
│   │   └── src/{lib.rs, plugin.rs, tool.rs}
│   ├── traveler/                     # extracted traveler-domain plugin (skeleton)
│   │   ├── Cargo.toml
│   │   ├── plugin.toml
│   │   ├── migrations/001_init.sql
│   │   ├── skills/traveler-api-tools.md
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── plugin.rs
│   │       ├── tools/{create_trip.rs, list_trips.rs, …}
│   └── radio/                        # internet radio (Radio Browser) + player window
│       ├── Cargo.toml
│       ├── plugin.toml
│       ├── skills/radio.md
│       └── src/{lib.rs, plugin.rs, radio_browser.rs, tools/mod.rs}
│   └── word/                         # simple word processor (.odt documents) + editor window
│       ├── Cargo.toml
│       ├── plugin.toml
│       ├── skills/word.md
│       └── src/{lib.rs, plugin.rs, tools/mod.rs}
│   └── impress/                       # presentation builder (.odp decks) + editor window
│       ├── Cargo.toml
│       ├── plugin.toml
│       ├── skills/impress.md
│       └── src/{lib.rs, plugin.rs, tools/mod.rs}
└── src/
    ├── plugins/                       # the loader/registry/installer inside the binary
    │   ├── mod.rs
    │   ├── loader.rs
    │   ├── registry.rs
    │   ├── manager.rs
    │   ├── installer.rs
    │   └── admin_api.rs
    ├── api/
    ├── services/                      # core-only services re-export from SDK
    └── …
```

A plugin author needs only **the `shiny-plugin-sdk` crate** + the host `axum`/`tokio` ecosystem.

---

## 3. The `Plugin` trait

Every plugin implements `shiny_plugin_sdk::plugin::Plugin`. The crate is compiled as a `cdylib`; the export symbol is `shiny_plugin_entry` (defined as `PLUGIN_ENTRY_SYMBOL` in the SDK).

```rust
use std::sync::Arc;
use async_trait::async_trait;
use shiny_plugin_sdk::{
    manifest::Manifest,
    plugin::{Plugin, PLUGIN_ENTRY_SYMBOL},
    services::PluginCtx,
    tools::RegistryBuilder,
};

pub struct MyPlugin;

#[async_trait]
impl Plugin for MyPlugin {
    fn manifest(&self) -> &Manifest {
        // Return a static-once Manifest instance.
    }

    fn register(&self, ctx: Arc<PluginCtx>, builder: &mut RegistryBuilder<'_>) {
        // Register tools, routes, crons, skills markdown, persona fragment.
        builder
            .persona("a travel navigator AI")
            .skills("## Tools\n- `hello` — say hello. params: `{}`")
            .tool_arc(shiny_plugin_sdk::tools::bridged(Arc::new(MyTool)));
        // NOTE: always wrap tools with `bridged(...)` — see §15.
    }

    // Optional hooks:
    async fn on_load(&self, _ctx: Arc<PluginCtx>) { /* spawn cron loops here */ }
    async fn on_unload(&self, _ctx: Arc<PluginCtx>) { /* clean shutdown */ }
    async fn on_user_registered(&self, _ctx: Arc<PluginCtx>, _user_id: &str) { /* seed profile */ }
}

/// The C entry symbol the loader transmutes + calls.
#[no_mangle]
pub extern "C" fn shiny_plugin_entry() -> *mut dyn Plugin {
    Box::into_raw(Box::new(MyPlugin))
}
```

### Lifecycle

1. **Boot discovery** — `main.rs` calls `plugins.discover_and_install(base_ctx)`, which walks `PLUGINS_DIR`, finds every `plugin.toml` and loads it with `libloading`.
2. **Admin API install** — `POST /api/plugins/install` carries the same logic at runtime (no restart).
3. **Loading a plugin** does, in order:
   1. Parse `plugin.toml`.
   2. Validate `api_level` and (optional) `target_triple`.
   3. Locate the cdylib file inside the archive, copy it to a versioned name so re-installs are safe even on Windows, `dlopen` it.
   4. Resolve `shiny_plugin_entry` and call it; receive `*mut dyn Plugin`, wrap as `Box<dyn Plugin>`.
   5. Run any `migrations/*.sql` not yet recorded in the core `plugin_schema_versions` table.
   6. Build a `PluginCtx` and call `plugin.register(ctx, &mut builder)`.
   7. Move `builder.tools` into the live `ToolRegistry`, append `skills_md` + `persona` + `context_lines` to the per-plugin contribution list.
   8. Call `plugin.on_load(ctx)` (if implemented).
4. **Every `/api/agent` call** now sees the concatenated skills markdown + persona in the system prompt and dispatches any registered tool through `ToolRegistry::invoke`.

---

## 4. The `Tool` trait

A single agent tool. The registry maps the action key (and any aliases) to `Arc<dyn Tool>` and `invoke()` is called per LLM-emitted action block.

```rust
use async_trait::async_trait;
use serde_json::Value;
use shiny_plugin_sdk::{
    errors::AppError,
    outcome::ActionOutcome,
    services::PluginCtx,
    tools::{Tool, ToolRequest, ParamHelpers},
};

pub struct HelloTool;

#[async_trait]
impl Tool for HelloTool {
    fn name(&self) -> &str { "hello" }
    fn aliases(&self) -> &[&str] { &["greet"] }       // optional
    fn step_label(&self) -> &str { "Saying hello…" } // shown in UI while running
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `hello` — Say hello. params: `{ name?: string }`")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let who = data.get("who").and_then(|v| v.as_str()).unwrap_or("world");
        format!("Said hello to {who}")
    }

    async fn invoke(&self, _ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let name = req.params.param_str("name").unwrap_or_else(|| "world".into());
        Ok(ActionOutcome::ok("hello", serde_json::json!({ "who": name, "reply": format!("Hello, {name}!") })))
    }
}
```

### `ToolRequest`

```rust
pub struct ToolRequest<'a> {
    pub user_id: &'a str,         // new core `users.id` (planned)
    pub traveler_id: &'a str,     // back-compat: mirrors user_id today
    pub params: &'a Value,        // raw JSON params block from the LLM
    pub ctx: &'a AgentContext,    // lat/lon/heading/lang/ollama_model
}
```

### `ActionOutcome` builder helpers

```rust
ActionOutcome::ok("hello", json!({ "reply": "Hi" }))
    .with_artifact(artifact)
    .with_extra_artifacts(vec![a1, a2])
    .with_navigation(session);
ActionOutcome::error("hello", "Name required");
```

### Parameter helpers

`ParamHelpers` is implemented for `serde_json::Value`:

```rust
req.params.param_str("name")           // Option<String>
req.params.param_f64("lat")            // Option<f64>
req.params.param_u32("limit")          // Option<u32>
req.params.param_bool("include_history")
req.params.require_str("name")?        // Result<String, AppError>
req.params.require_f64("lat")?
```

---

## 5. Plugin manifest (`plugin.toml`)

```toml
name = "hello"
version = "0.1.0"
api_level = 1
entry_symbol = "shiny_plugin_entry"
target_triple = "aarch64-unknown-linux-gnu"   # optional; checked when present
description = "Demo plugin registering a single 'hello' tool"
author = "shiny demo"
summary = "Demo plugin: hello tool"
migrations_dir = "migrations"
skills_dir = "skills"
web_dir = "web"
category = "Demo"                             # optional grouping hint — see §19
capabilities = ["open-urls"]                  # optional declared capabilities — see §19
# signature = "<hex ed25519>"                  # optional; see §12
```

A **web-app plugin** (`kind = "web"`) needs no `entry_symbol` and no cdylib: its
whole payload is the static site in `web/`, which the shell frames in a window:

```toml
name = "photocraft"
version = "0.10.0"
api_level = 1
kind = "web"                                  # static web app — no cdylib
description = "Image editing"
summary = "PhotoCraft — the app's own web build, installed by the Craft store"
web_dir = "web"
category = "Craft"
```

Fields:

| Field | Type | Required | Notes |
|---|---|---|---|
| `name` | string | yes | unique; used as the install directory name |
| `version` | semver | yes | surfaced in `/api/plugins` |
| `api_level` | u32 | yes | if `> CORE_API_LEVEL` the install is rejected |
| `entry_symbol` | string | native only | usually `"shiny_plugin_entry"`; a web-app plugin omits it |
| `kind` | string | no | `"native"` (default: a cdylib) or `"web"` (a static web app: `web/index.html`, served at `/plugins/<name>/` and framed in a window). Host-side key, like `category` |
| `target_triple` | string | no | when present the installer refuses mismatched platforms |
| `description` | string | no | short human title |
| `author` | string | no | attribution |
| `summary` | string | no | used by admin UI to say "what this plugin adds" |
| `category` | string | no | grouping hint for the top-bar tray (e.g. `Office`, `Media`, `Travel`, `System`) — see §19 |
| `capabilities` | string[] | no | declared capabilities, e.g. `["open-urls"]` makes the plugin a candidate link handler — see §19 |
| `migrations_dir` | string | no | default `"migrations"` |
| `skills_dir` | string | no | default `"skills"` |
| `web_dir` | string | no | default `"web"` |
| `signature` | hex string | no | optional ed25519 signature — see §12 |

`CORE_API_LEVEL` is defined in `shiny-plugin-sdk::CORE_API_LEVEL`. The current value is **1**. Bumping core's API level one major version at a time is the policy; plugins with `api_level ≤ CORE_API_LEVEL` are accepted.

---

## 6. The installer

### Archive layout

A plugin archive is just a directory containing **at least** `plugin.toml` and a cdylib file (`.so`, `.dylib`, or `.dll`). The directory may be the root of the archive or a child.

```
hello.zip
└── hello/
    ├── plugin.toml
    ├── libshiny_hello_plugin.so          # any .so/.dylib/.dll is accepted
    ├── migrations/
    │   └── 001_init.sql
    ├── skills/
    │   └── hello.md
    └── web/                               # optional
        ├── plugin.js                      # window surface (if the plugin has one)
        └── icon.svg                       # plugin icon — see §19 "Plugin icon"
```

You can produce one with any packaging tool:

```bash
zip -r hello.zip hello/
tar czf hello.tar.gz hello/
```

### Admin API

| Method | Path | Auth | Description |
|---|---|---|---|
| `GET` | `/api/plugins` | Bearer (any logged-in user) | List installed plugins, with per-user `enabled` flags |
| `POST` | `/api/plugins/install` | Bearer (any logged-in user) | Multipart upload `file=@archive` |
| `POST` | `/api/plugins/uninstall` | Bearer (any logged-in user) | JSON body `{"name":"hello"}` |

> There is **no admin role** in the current auth model: any logged-in user can
> install/uninstall a plugin, but the install now lands in **that user's own**
> `PLUGINS_DIR` (per-user servers), not in a shared directory. The read-only
> `SYSTEM_PLUGINS_DIR` baseline is loaded by everyone and cannot be uninstalled
> (only deactivated); a user plugin of the same name overrides it.
> **Activation** is per-user (stored in `user_plugin_states`). The `ADMIN_TOKEN`
> env var and the `is_admin` column are read but not currently enforced — see §14.

Authenticate with a logged-in user's bearer token (register or login first — see §7):

```bash
curl -X POST http://localhost:8080/api/plugins/install \
  -H "Authorization: Bearer $TOKEN" \
  -F "file=@hello.zip"
```

Response on success:

```json
{ "success": true, "data": { "installed": "hello" } }
```

### Installer workflow (mirrors WordPress)

1. Receive multipart upload. Sniff the first few bytes — PK header → `zip`, `1f 8b` → `tar.gz`, `ustar` magic → plain `tar`. Anything else → `400 Unrecognised archive format`.
2. Extract into a `_staging-<pid>-<ts>` directory.
3. If the archive contains exactly one subdirectory with a `plugin.toml`, descend into it.
4. Parse `plugin.toml`. Validate `api_level ≤ CORE_API_LEVEL`. Validate `target_triple` matches the host if present.
5. Locate a cdylib in the archive. Reject if none.
6. Acquire install lock on `PLUGINS_DIR/.install.lock`.
7. If `PLUGINS_DIR/<name>/` exists, back it up as `<name>.bak`.
8. `rename` staging → `PLUGINS_DIR/<name>/`. Clean staging.
9. **Loader** is called: dlopen, call `shiny_plugin_entry`, run migrations (gated by the core `plugin_schema_versions` table), call `Plugin::register`.
10. Plugin's tools, persona fragment, skills markdown slice, and `context_lines` become live in `ToolRegistry` / `PluginManager`.
11. `on_load(ctx)` is awaited. Plugins start cron loops here.
12. On any error: roll back the install directory, return 400/500 with the error message.

### Hot-reload

The new tools are surfaced by the agent on the **next** `/api/agent` call automatically — no restart needed for tool/skills/persona changes. HTTP routes contributed via `RouteSpec` go through an `ArcSwap<Router>` rebuild callback that `AppState::router_rebuild` triggers; the implementation of the live router swap is the hot-reload step that wraps each request to observe the current router snapshot.

### Roll-back

If a prior install of the same plugin name exists, it is renamed to `<name>.bak` before the new copy becomes live. To roll back:

```bash
POST /api/plugins/uninstall {"name":"hello"}    # because admin_token matches
mv data/plugins/hello.bak data/plugins/hello
# next server restart picks it up; or curl install a fresh archive
```

---

## 7. Worked example: the `hello` plugin

Full source under `plugins/hello/`. Here is the entire surface:

**`plugins/hello/Cargo.toml`**

```toml
[package]
name = "shiny-hello-plugin"
version = "0.1.0"
edition = "2021"

[lib]
name = "shiny_hello_plugin"
crate-type = ["cdylib", "rlib"]

[dependencies]
shiny-plugin-sdk = { path = "../../crates/shiny-plugin-sdk" }
async-trait = "0.1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
semver = { version = "1", features = ["serde"] }
```

**`plugins/hello/src/plugin.rs`**

```rust
use std::sync::Arc;
use async_trait::async_trait;
use shiny_plugin_sdk::{
    manifest::Manifest,
    plugin::{Plugin, PLUGIN_ENTRY_SYMBOL},
    services::PluginCtx,
    tools::RegistryBuilder,
};

pub struct HelloPlugin;

#[async_trait]
impl Plugin for HelloPlugin {
    fn manifest(&self) -> &Manifest {
        static M: std::sync::OnceLock<Manifest> = std::sync::OnceLock::new();
        M.get_or_init(|| Manifest {
            name: "hello".into(),
            version: semver::Version::new(0, 1, 0),
            api_level: 1,
            entry_symbol: PLUGIN_ENTRY_SYMBOL.into(),
            target_triple: None,
            description: Some("Demo plugin registering a single 'hello' tool".into()),
            author: None,
            summary: None,
            migrations_dir: "migrations".into(),
            skills_dir: "skills".into(),
            web_dir: "web".into(),
            signature: None,
        })
    }

    fn register(&self, _ctx: Arc<PluginCtx>, builder: &mut RegistryBuilder<'_>) {
        builder
            .skills("- `hello` — Say hello. params: `{ name?: string }`")
            .tool(crate::tool::HelloTool);
    }
}

#[no_mangle]
pub extern "C" fn shiny_plugin_entry() -> *mut dyn Plugin {
    Box::into_raw(Box::new(HelloPlugin))
}
```

**`plugins/hello/src/tool.rs`**

```rust
use async_trait::async_trait;
use serde_json::{json, Value};
use shiny_plugin_sdk::{
    errors::AppError,
    outcome::ActionOutcome,
    services::PluginCtx,
    tools::{Tool, ToolRequest, ParamHelpers},
};

pub struct HelloTool;

#[async_trait]
impl Tool for HelloTool {
    fn name(&self) -> &str { "hello" }
    fn step_label(&self) -> &str { "Saying hello…" }
    fn doc_fragment(&self) -> Option<&str> {
        Some("- `hello` — Say hello. params: `{ name?: string }`")
    }
    fn humanize(&self, _r: &str, data: &Value) -> String {
        let who = data.get("who").and_then(|v| v.as_str()).unwrap_or("world");
        format!("Said hello to {who}")
    }

    async fn invoke(&self, _ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError> {
        let name = req.params.param_str("name").unwrap_or_else(|| "world".into());
        Ok(ActionOutcome::ok("hello", json!({ "who": name, "reply": format!("Hello, {name}!") })))
    }
}
```

**`plugins/hello/plugin.toml`**

```toml
name = "hello"
version = "0.1.0"
api_level = 1
entry_symbol = "shiny_plugin_entry"
description = "Demo plugin registering a single 'hello' tool"
```

**`plugins/hello/migrations/001_init.sql`**

```sql
CREATE TABLE IF NOT EXISTS hello_pings (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    who TEXT NOT NULL,
    at TEXT NOT NULL DEFAULT (datetime('now'))
);
```

### Build + package + install

```bash
# Build the whole workspace (the binary and every plugin cdylib).
cargo build --release --workspace

# Zip the plugin directory.
mkdir -p /tmp/pkg/hello
cp plugins/hello/plugin.toml /tmp/pkg/hello/
cp plugins/hello/skills/hello.md /tmp/pkg/hello/skills/      # after `mkdir -p skills`
cp plugins/hello/migrations/001_init.sql /tmp/pkg/hello/migrations/
cp target/release/libshiny_hello_plugin.so /tmp/pkg/hello/
( cd /tmp/pkg && zip -r hello.zip hello )

# Install on a running server.
ADMIN_TOKEN=test123
curl -X POST http://localhost:8080/api/plugins/install \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -F "file=@/tmp/pkg/hello.zip"
# {"data":{"installed":"hello"},"success":true}

# Verify.
curl http://localhost:8080/api/plugins
curl -X POST http://localhost:8080/api/auth/register \
  -H "Content-Type: application/json" \
  -d '{"name":"Demo","username":"demo","email":"demo@x.com","password":"pw"}'
# → use the returned bearer token to call /api/agent
```

### The agent seeing the plugin's tool

When the user says "Use the hello tool to greet Alice", the agent calls:

```json
{"action":"hello","params":{"name":"Alice"}}
```

`ToolRegistry::invoke` runs `HelloTool::invoke`, which returns:

```json
{"action":"hello","result":"ok","data":{"who":"Alice","reply":"Hello, Alice!"}}
```

The runner stores this in `actions_taken` and the LLM produces the final reply: "I've sent a friendly greeting to Alice for you!" with `steps: ["hello complete"]`.

### Uninstall

```bash
curl -X POST http://localhost:8080/api/plugins/uninstall \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"name":"hello"}'
```

`/api/plugins` then returns `[]` and the next agent invocation no longer recognises the `hello` action.

---

## 8. The skill markdown system

The agent's system prompt is built like this:

```
You are <ai_name>, <persona_concat>. Reply in language code '<lang>'.
…
## Tools
<skill_md>

## Context
User name: <first>
<location_line>
<trip_line>
Diary: <diary_line>
<context_lines from plugins>
Mode: <mode>
```

`skill_md` is the concatenation, in this order, of:

1. The legacy `web/skills/traveler-api-tools.md` file (kept for backwards compatibility with embedded traveler code).
2. The plugin `skills_md` field each plugin sets via `builder.skills("…")` in `register()`.
3. Every `Tool::doc_fragment()` from every registered tool, deduplicated by tool name, separated by `\n---\n`.

This guarantees that what the LLM sees about available tools is the union of what every installed plugin advertises. **A tool that doesn't surface a `doc_fragment` will not be revealed to the LLM** — use this to keep internal tools hidden.

Item 2/3 come from the plugins **active for the session** — except in **focus mode**: when the user has focused plugin windows, the scope narrows to exactly those plugins for that turn and the prompt gains a `## Focus mode (ON)` block. Focus mode is enforced by `execute_action`, not by the prompt: a plugin tool is allowed when its owner is focused (so a focused plugin is always usable), core desktop actions are refused from an explicit list, and everything else fails open. Only the user changes the focus. See `docs/core/desktop.md` § Focus mode and `tests/focus_mode_scope.rs`.

### Persona fragment

`builder.persona("a travel navigator AI")` adds a phrase inserted into `You are {ai_name}, {persona_concat}.` — multiple plugins concatenate with spaces. When no plugin supplies a persona, the default generic phrase is `"a helpful AI assistant"`.

### `context_lines`

`builder.context_line("Active trip: {trip.name}")` lets a plugin arrange that context lines (e.g. "No active trip") always appear in the per-request system prompt, regardless of whether the model is about to call one of its tools. Useful when the plugin owns tables the LLM should always be aware of.

---

## 9. Migrations

Migrations are per-plugin SQL files in `plugins/<name>/migrations/`. The runner `shiny_plugin_sdk::migrations::run_plugin_migrations` ensures a `plugin_schema_versions(plugin, file)` table exists and applies any `.sql` not yet recorded.

```sql
-- 001_init.sql
CREATE TABLE IF NOT EXISTS tips (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    body TEXT NOT NULL
);
```

Migrations are idempotent (use `IF NOT EXISTS`, or the table-recreate-and-copy pattern for `ALTER TABLE` since SQLite has limited `ALTER` support). Numbered files are applied in lexicographic order.

### Important notes

- The `plugin_schema_versions` table is **core-owned**; plugins don't manage migration state themselves.
- Re-installing a plugin runs only **new** migration files (files already in `plugin_schema_versions` are skipped).
- **An applied migration file is immutable.** The runner records it by *name*; editing its contents afterwards does not re-run it and does not warn. Two consequences that have both bitten this repo:
  - A fix written into an already-applied file is silently lost on every install that has applied it (fresh installs get it, existing ones never do). Add a **new, higher-numbered** file instead.
  - A later file's `CREATE TABLE IF NOT EXISTS` is a **no-op** if an earlier file already created that table. If the first version of the table lacked a column the code now writes, the code fails at runtime against the old shape — and a plugin that swallows DB errors (§15) will fail *silently*. Check that the replay of your files produces every column your code names, and when a repair genuinely cannot be expressed as DDL (SQLite has no conditional `ALTER TABLE … ADD COLUMN`), do it in code from `on_load` where a "duplicate column" result can be treated as success — `plugins/browser/src/history.rs::ensure_schema` is the reference.
- Uninstalling leaves the plugin's tables in place — rolling back structural schema changes is the admin's responsibility. Drop the install directory then `DROP TABLE my_plugin_xxx;` if you want to actually clean up.

---

## 10. The install log

Every install attempt — successful or failed — is appended to **`PLUGINS_DIR/install.log`**. This is the audit trail admins use to diagnose failures offline.

Format: `[YYYY-MM-DD HH:MM:SS] <event-tag> key=value…`

```
[2026-07-12 20:33:50] install-begin bytes=1326462
[2026-07-12 20:33:50] format-detected format=TarGz
[2026-07-12 20:33:51] manifest-ok name=hello version=0.1.0 api_level=1
[2026-07-12 20:33:51] install-ok name=hello
[2026-07-12 20:33:52] install-begin bytes=15
[2026-07-12 20:33:52] reject-format unknown-bytes
[2026-07-12 20:35:01] uninstall-ok name=hello
```

Event tags:

| Tag | Meaning |
|---|---|
| `install-begin` | Upload received; logging starts. |
| `format-detected` | Magic-byte sniff succeeded. |
| `reject-format` | Unrecognised archive bytes. |
| `manifest-ok` | `plugin.toml` parsed successfully. |
| `manifest-read-failed` | No `plugin.toml` at archive root. |
| `manifest-parse-failed` | toml::from_str error. |
| `reject api_level` | Plugin requires newer API than running core. |
| `cdylib-missing` | Archive has no `.so`/`.dylib`/`.dll`. |
| `rename-failed` | Couldn't move staging dir into final location. |
| `extract-failed` | zip/tar unpack error. |
| `register-failed` | dlopen, missing symbol, migration, or `register()` error. |
| `install-ok` | Plugin loaded and registered. |
| `uninstall-ok` | Plugin name was installed and is now absent. |
| `uninstall-missing` | Uninstall requested for a plugin that wasn't installed. |

The log lives wherever `PLUGINS_DIR` points (default `data/plugins/install.log`).

---

## 11. Configuration

The plugin system reads these env vars in `Config::from_env`:

| Var | Default | Purpose |
|---|---|---|
| `PLUGINS_DIR` | `data/plugins` | Writable plugin directory: uploads + the `install.log`. |
| `SYSTEM_PLUGINS_DIR` | unset | Optional read-only system baseline loaded for every user; a user plugin of the same name overrides it. |
| `ADMIN_TOKEN` | unset | Read into `Config::admin_token` and the plugin `ConfigSnapshot`, but **not currently enforced** by the auth middleware — plugin management is gated by a logged-in user token (see §13/§14). |
| `CORE_TRAVELER_BUILTIN` | `true` | When `true`, the embedded traveler tools (`src/services/agent_tools.rs`) still answer actions the plugin didn't claim. Set `false` to make core a pure assistant. |

All other env vars come through "as-is" to plugins via `PluginCtx::config` (`ConfigSnapshot`).

---

## 12. Signature verification (optional)

The manifest supports an optional `signature` field: a hex-encoded ed25519 signature of `plugin.toml + lib<name>.{so|dylib|dll}`. Coexistence plan:

1. Generate a keypair and publish the public key in the running server's `TRUSTED_PUBKEYS` env (semicolon-separated hex).
2. Author signs `plugin.toml` plus the cdylib contents with their private key; stores the hex in `plugin.toml`'s `signature` field.
3. The installer rejects if `signature` is present but doesn't verify, unless `--insecure` is passed via the `X-Shiny-Insecure` header.

> This step is **off by default** in the v0 system (the signature decoder isn't yet wired in the loader). The manifest field exists so plugins can prepare for it; expect it to flip to enforced in api_level 2.

---

## 13. Admin endpoints reference

### `GET /api/plugins`

Bearer (any logged-in user). Returns installed plugins, each with the caller's own `enabled` flag:

```json
{
  "success": true,
  "data": [
    {
      "name": "hello", "version": "0.1.0", "api_level": 1,
      "description": "Demo plugin...", "summary": "...", "enabled": true,
      "surface": true, "web_app": false, "hud": false, "category": "Demo",
      "capabilities": []
    }
  ]
}
```

`surface` is true when the plugin ships a `web/plugin.js` window module;
`web_app` is true for a `kind = "web"` plugin, whose window the shell mounts as
an iframe over `/plugins/<name>/` instead.

### `POST /api/plugins/install`

Multipart form upload. Auth: any logged-in user (Bearer token). Installation is shared server-side; the newly installed plugin is enabled by default for every existing user.

```
curl -X POST http://localhost:8080/api/plugins/install \
  -H "Authorization: Bearer test123" \
  -F "file=@hello.zip"
```

Handlers trigger after the loader finishes: tools are in the registry, skills markdown is updated, persona is updated. The agent sees them on the very next call.

### `POST /api/plugins/uninstall`

```json
POST /api/plugins/uninstall
{ "name": "hello" }
```

Removes the plugin's install directory, calls `on_unload`, removes contributions. The previous install is preserved as `data/plugins/<name>.bak` until the next reinstall overwrites that backup.

### `POST /api/plugins/activate`

```json
POST /api/plugins/activate
{ "name": "hello" }
```

Re-enables a deactivated plugin. Tools come back online immediately; the agent sees them on the next call. No-op if already active.

### `POST /api/plugins/deactivate`

```json
POST /api/plugins/deactivate
{ "name": "hello" }
```

Keeps the plugin installed and its database tables intact, but disables its tools. The agent stops advertising the deactivated plugin's tools (their `doc_fragment` snippets are omitted from the skills markdown) and `ToolRegistry::invoke` returns `"plugin 'hello' is deactivated"` if the LLM still tries one. The choice is stored per user in `user_plugin_states` (the row `enabled` boolean), so it survives restarts. Deactivating also closes the plugin's window and stops any process the window kept alive (a Craft app's MCP server, gracefully — see `POST /api/plugins/window/closed`); **deactivate is not the window's X** (see Activation state below).

### `POST /api/plugins/window/closed`

```json
POST /api/plugins/window/closed
{ "name": "photocraft" }
```

The shell closed a plugin window. Windows are client state, but a Craft app's MCP server belongs to its window: this stops the app's CLI through the MCP lifecycle (`shutdown` request → `exit` notification → EOF on stdin, with a kill only after the grace period) and unregisters its tools immediately, instead of waiting for the next agent turn. A no-op for plugins that own no process. **Closing a window is not deactivation**: `enabled` is untouched and the tray can reopen the window.

### `POST /api/plugins/session/reset`

Called at boot (and sign-in) when `session.remember` is off. Drops the previous sign-in's session activations (this user's `user_plugin_states` rows with `enabled = 1`) and stops every running Craft MCP server, so a fresh session starts with no windows and every plugin closed. `enabled = 0` rows are remember-mode choices and are left alone.

### `GET /api/plugins/install.log`

Bearer (any logged-in user). Returns the last 200 lines of `PLUGINS_DIR/install.log` as `text/plain`. Useful for plugin managers and for post-incident forensics.

### Craft store

The core carries a catalog of the [Craft App Suite](https://getartcraft.com/apps)
and installs each app's own **web build** as a web-app plugin, verified against
the SHA-256 its GitHub release publishes. Downloads are restricted to GitHub
hosts; installs land in `PLUGINS_DIR/<app-id>/` like any other plugin.

| Endpoint | Body | Does |
|---|---|---|
| `GET /api/plugins/store` | — | The catalog plus each app's installed/latest version, `update_available`, install `state` (`idle`, `downloading`, `installing`, `failed`) and `progress`. Answered from cache; a background refresh is scheduled when an entry is stale. |
| `POST /api/plugins/store/refresh` | — | Re-checks every app's latest release, ignoring the cache (GitHub rate-limits unauthenticated callers, so entries are ETag-cached for an hour and the last good answer is kept when a check fails). |
| `POST /api/plugins/store/install` | `{"id":"photocraft"}` | Downloads, verifies and installs (or updates) one app. Returns immediately; the work runs in the background and is visible through `GET /api/plugins/store` and the install log. Uninstall uses `POST /api/plugins/uninstall` as usual. |
| `POST /api/plugins/store/mcp` | `{"id":"photocraft","enable":true}` | Installs (or removes) the app's command-line tool, which is its MCP server; see AI control below. |

The catalog and the release/verification rules follow
[ArtCraft Launcher](https://github.com/storytold/craft-launcher) (MIT OR
Apache-2.0, © ArtCraft Team and the contributors): `crates/core/src/catalog.rs`
for the suite table and `release.rs` for picking an asset out of a release. We
install the `-web-` asset and keep the whole pipeline inside the plugin system
(`src/plugins/store.rs`).

#### AI control (MCP)

Every Craft app ships `<app>-cli`, whose `mcp` subcommand is a Model Context
Protocol server over stdio that drives the app's engine. **AI control is on by
default and installs with the app**: installing or updating an app pulls the
release's Linux package, verifies it, and installs just the CLI at
`PLUGINS_DIR/.craft-mcp/<id>/bin/<id>-cli` (a dot-directory: the plugin scanner
skips it, and updating the web app never touches it) — so the pair is always in
step and there is no second download to remember. The card's **AI control**
switch (`POST /api/plugins/store/mcp`) is the opt-out: turning it off removes the
CLI and records the choice in `.craft-mcp/<id>/disabled`, which later installs
and the background updater respect; turning it back on clears the marker and
installs the CLI (when the app is installed — otherwise the app's own install
carries it). An app installed without its CLI (a failed download, or one
installed before this rule) shows `mcp_pending` in `GET /api/plugins/store` and
is finished by the updater or the card's own button. One rule decides what an
install/update does with the CLI (`cli_plan` in `src/plugins/store.rs`):
opted out → nothing; already at this release → nothing (no 60–124 MB
re-download); otherwise install it, or say the release has none. `mcp_enabled`
in the store snapshot is the decision, so the switch reads the user's choice and
not whether a file happens to exist yet.

The capability follows the app's **window**, enforced in code:
`src/plugins/craft_mcp.rs` starts one server per *open* Craft app at the beginning
of each agent turn and stops every server whose window is closed, registering the
app's MCP tools under its plugin id (`photocraft_command_list`, …; the doc fragment
is the tool's description plus a compact argument list). **Focus mode narrows that
same surface**: with one or more windows focused, only those apps keep a server, so
focusing is how the user points the AI at one activity (or a few) instead of
everything on screen. With no window open, no server runs and no tool exists; with
another plugin focused, the ordinary focus gate refuses them. Opening an app
mid-turn (`show_plugin`) starts its server and hands the model its tools in the same
conversation. Turning AI control off (the CLI removed) stops a running server at the
next turn as well. Closing the window or deactivating the plugin stops it **now**:
`POST /api/plugins/window/closed` (and the deactivate path) ask the CLI to leave
through the MCP lifecycle — a `shutdown` request, an `exit` notification, then EOF
on stdin — and only kill a server that ignores all three (`McpClient::stop` in
`src/plugins/craft_mcp.rs`). None of this depends on prompt wording.

The app CLI drives the app's own engine: in headless mode it opens, edits and
saves documents itself (`--automation-read-root` / `--automation-write-root`
scope its file access to the user's home). Those two flags **imply `--headless`**
(the CLI's own rule), so the server the agent talks to is a private engine
process — **not** the window on screen, which is the vendor's static web build
and has no control channel. The vendor's live channel, `<app> --control 7979`, is
a native-app feature; the web builds can't host it. Two consequences, both in the
prompt so the model cannot pretend otherwise: it must **save or export** the
document and give the user the path, and unsaved work dies with the turn's
process (the host stops the server gracefully when the window closes or the
focus changes).

**Live control, when the app's build can take it.** The Craft web builds
document a page API — `window.<app>.request(method, params)`, the same methods
their desktop control channel takes. It is detected per app id, never per app
name: `web/js/webAppSurface.js` looks up `window[<plugin id>]`, and when the
functions are there it attaches the page to the core's control-channel proxy
(`src/plugins/craft_control.rs`) over SSE. The proxy opens a loopback port
speaking the vendor's line protocol and forwards every request to the page, which
runs it in the live document and answers; the MCP host then starts `<app>-cli mcp
--connect 127.0.0.1:<port>` instead of the headless server, so the agent's tools
change **the window the user sees**. A build without the API attaches nothing and
keeps the headless fallback, so the feature lights up per app as its releases
ship the API — for the whole suite, including apps installed later. The prompt
says which mode each open app is in; focus mode still decides *which* apps have a
server at all. Routes: `GET /api/plugins/craft/control/events?app=<id>` (SSE,
page side) and `POST /api/plugins/craft/control/reply`.

**Opening files inside an app.** The web builds ask for files with a classic
`<input type=file>`, which the kiosk's web engine cannot answer on its own (it is
embedder business, and no handler was wired). `web/js/webAppSurface.js` therefore
shims the frame's file inputs: a click opens the shell's picker
(`web/js/filePicker.js`), which browses the home through the Files plugin, and
the bytes come back as `File` objects in the frame's own realm with `input` and
`change` dispatched — the app's own Open… then works, for every web app, present
or installed later, with no per-app code. The picker is a **compact floating
window** (`ui/floating`): the desktop's own slim title bar (draggable, only a
close control), the Files window's grid with live thumbnails and its icon
language, and the theme's window surface — no backdrop, not a modal, and outside
the tiling, so it never disturbs the desktop layout. The shell's own
`pickFiles()` uses the same window. Files → “Open with app” for a web app still
needs the app's page API (`openFile`), so it follows the same gate as live
control.

### Activation state

Every plugin is either **Active** (its tools are on for the session) or **Off**
(deactivated). The per-user `user_plugin_states` table stores the choice; it is
what `GET /api/plugins` reports as `enabled`. What a sign-in *starts from*
depends on `session.remember`:

- **remember off** (default) — session-scoped. The shell calls
  `POST /api/plugins/session/reset` before the app starts, so the previous
  sign-in's activations are dropped and every Craft MCP server stops: no plugin
  window exists and the desktop starts clean. Activating a plugin lasts for the
  session; deactivating **deletes** the row instead of writing `enabled = 0`, so
  a fresh-session choice never turns into a permanent disable later.
- **remember on** — the previous session is restored: installed minus explicitly
  disabled (`enabled = 0`), plus the saved workspaces (windows, positions,
  focus, fullscreen). Only windows the saved workspace lists are reopened —
  being active never opens a window by itself.

Activating / deactivating:

- Updates `GET /api/plugins` `enabled` boolean.
- Affects the agent immediately: deactivated plugins contribute no `doc_fragment`, no `persona` fragment, no `context_lines`, and their `Tool::invoke` short-circuits with a friendly `BadRequest`.
- Deactivating also closes the plugin's window and stops any process it kept alive (a Craft app's MCP server, gracefully). Its window **X** is a different gesture: it closes the window and stops its process, but the plugin stays active — the tray can reopen it, and its tools keep working while it is closed.

The separation between **deactivate** (keep install dir + tables, just turn tools off) and **uninstall** (drop install dir + library unload) mirrors WordPress's "deactivate" vs "delete" distinction. Deactivation is reversible; uninstall requires re-uploading the archive.

---

## 14. Security model

- Plugins are full Rust `cdylib`s loaded into the same process. They have full process privileges — they can read SQLite rows, make HTTP calls, spawn threads, write files. **Don't install plugins you don't trust.**
- Plugin management (install/uninstall/activate/deactivate) is gated only by a **logged-in user token** — there is currently **no admin role**. The `ADMIN_TOKEN` env var and the `is_admin` column are read but not enforced. Isolation comes from the deployment instead: with one server per OS user, each account has its own `PLUGINS_DIR` under `~/.local/share/shiny/`, so a plugin is loaded only by that account's server. `SYSTEM_PLUGINS_DIR` is a read-only baseline shared by all. A single server with several mutually distrusting accounts breaks that separation. Real admin gating, signature verification, and process isolation are roadmap work.
- The `install.log` makes post-incident forensics possible but does not prevent malicious plugins.
- For multi-tenant hosting, plan to (a) require `signature` validation, (b) sandbox long-running plugins behind an IPC process boundary. Both are roadmap items; v1 is single-tenant.

---

## 15. ABI stability & the runtime bridge

`api_level` describes the SDK's source-level trait surface; the loader accepts
levels up to the current **1**. This check does not prove binary compatibility:
the entry returns a Rust trait object, which has no stable C ABI.

Host-only refactors must freeze the entry signature, trait layout, `Manifest`,
contexts and shared SDK structures. Adding default methods or struct fields can
change vtables or memory layouts and requires a coordinated rebuild and explicit
compatibility testing. Build new native plugins with matching SDK sources,
compiler and target. See [Runtime & ABI](docs/plugins/runtime-abi.md) for optional
host-side `[build]` diagnostic metadata and the compatibility fixture checks.

### Two load-bearing rules (learned from production crashes)

A plugin cdylib statically links **its own copies** of Tokio, sqlx/libsqlite3, and reqwest/hyper. Nothing bound to a runtime or a C allocator may cross the dlopen boundary:

1. **Always wrap tools with `bridged(...)` at registration.** The adapter runs `invoke` on a runtime the *plugin* owns (`shiny_plugin_sdk::rt::bridge`) and returns the result over an executor-agnostic channel. Without it, the first sqlx/reqwest/tokio-time call inside a tool aborts the host with *"this functionality requires a Tokio context"*.
2. **Only use the async accessors on `PluginCtx`** — `ctx.pool()`, `ctx.ollama()`, `ctx.search()`, `ctx.supertonic()`. Each lazily opens a **plugin-owned** connection/client inside the plugin's runtime. Never accept a live `SqlitePool` or pre-built reqwest client from the host: values allocated by the host's libsqlite3 segfault when freed by the plugin's copy, and host-built HTTP clients panic when driven from the plugin's reactor. Migrations are the exception by design — they run host-side, with the host pool, at load time.

> **Known caveat (plugin DB access):** a plugin `cdylib` statically links its **own** `libsqlite3` (via `libsqlite3-sys`, compiled with `-DSQLITE_ENABLE_MEMORY_MANAGEMENT`), so in the running server there are *multiple* SQLite library copies in one process. Plugin-owned DB access — both the async `ctx.pool()` (sqlx worker threads) and the newer synchronous `ctx.db()` (`libsqlite3-sys` FFI, unit-tested) — segfaults in the server (`sqlite3_value_free` / `sqlite3LockAndPrepare`). This is a deep multi-copy SQLite issue, not the runtime-bridge shape. The definitive fix is to route **all** plugin data access through the host's single SQLite copy (a host-provided DB API), or to build SQLite without `SQLITE_ENABLE_MEMORY_MANAGEMENT` — see §20 item 8.

---

## 16. Troubleshooting

| Symptom | Likely cause | Fix |
|---|---|---|
| `Unrecognised archive format` | Upload wasn't a zip/gz/tar | `file archive.zip` |
| `Invalid plugin.toml: ...` | Missing required field or wrong types | Re-serialize with `toml::to_string(&manifest)` |
| `Plugin api_level X > core Y` | Old binary, plugin built against newer SDK | Upgrade the binary |
| `Plugin built for 'x86_64-unknown-linux-gnu' but host is 'aarch64-unknown-linux-gnu'` | Wrong platform binary | Rebuild the cdylib on the target host |
| `No cdylib found` | You forgot to ship `.so/.dylib/.dll` in the zip | `ls` the archive; ensure layout includes the cdylib |
| `Missing symbol shiny_plugin_entry` | The `#[no_mangle] extern "C" fn shiny_plugin_entry()` is missing or wrapped differently | Add it exactly as shown in §7 |
| 401 on install | Wrong `ADMIN_TOKEN` or no `is_admin=1` user | `curl -H "Authorization: Bearer $ADMIN_TOKEN"` with the right env value |
| Tool not registered | `register()` didn't call `builder.tool_arc(bridged(tool))` | Re-trigger `register` and verify it's added to the builder |
| LLM doesn't call the tool | `doc_fragment()` returns `None` | Set a doc fragment so the tool is advertised in the system prompt |
| Host aborts: `this functionality requires a Tokio context` | Tool `invoke` ran on the host runtime (sqlx/reqwest/tokio-time inside plugin code) | Wrap the tool with `bridged(...)` (§15) |
| Host SIGSEGV in `sqlite3_value_free` / `ValueHandle::drop` | A `SqlitePool` crossed the dlopen boundary | Use `ctx.pool().await` — the plugin-owned pool (§15) |
| Host aborts in `hyper-util … dns.rs`: `there is no reactor running` | A host-built reqwest client was driven from the plugin runtime | Use `ctx.ollama()/ctx.search()/ctx.supertonic()` accessors (§15) |

Always inspect **`PLUGINS_DIR/install.log`** first — every step is timestamped there.

---

## 17. Plugin authoring checklist

Before publishing a plugin:

- [ ] `plugin.toml` parses to `Manifest`
- [ ] `api_level` equals the SDK you compiled against
- [ ] `shiny_plugin_entry` is exported via `#[no_mangle] pub extern "C"`
- [ ] Every tool has a `name()`, `step_label()`, `doc_fragment()`, and `invoke()`
- [ ] Every advertised tool registers via `builder.tool_arc(bridged(tool))` (§15)
- [ ] All DB/HTTP work goes through the async accessors `ctx.pool()/ollama()/search()/supertonic()` — never a host-passed pool or client (§15)
- [ ] Every migration file is idempotent (`CREATE TABLE IF NOT EXISTS`)
- [ ] `skills_md` and `persona` are set if the plugin contributes persona/skills
- [ ] `web/icon.svg` ships a 24×24 `currentColor` icon in the plugin icon style — bold `stroke-width="2"`, round caps/joins, tight framing (§19 "Plugin icon" → "Icon style") — **or** the plugin is mapped to a shared `apps/<name>` icon in `web/js/pluginIcon.js` (bundled plugins). Prefer reusing an existing name from `web/ui/icons/INDEX.md` for anything inside the window.
- [ ] Any icon referenced inside the window is a **name from the shared catalog** (`web/ui/icons/INDEX.md`), not a hand-drawn SVG — so it follows the theme and accent.
- [ ] User-visible events use the core notification system (`notify()` / `with_notification`) — §19 "Notifications"
- [ ] **Window surfaces have a JS smoke test.** `import()` of `web/plugin.js` is what core does to mount a window. A syntax error there is not fatal to the app — core logs one `console.warn` — but the window it was meant to be is simply absent, which reads as "the plugin does nothing". `node --check web/plugin.js` catches most of it, and `plugins/browser/web/plugin.smoke.mjs` is the reference for the rest: it mounts the real surface against a small DOM shim and asserts the generated srcdoc, the card markup and the postMessage contract.
- [ ] Every migration file is idempotent (`CREATE TABLE IF NOT EXISTS`) **and never edited after it has been applied** — the runner records files by name and will not re-run one, so an edit is silently lost (see §9 "Important notes")
- [ ] zip / tar.gz test build is reproducible
- [ ] `cargo check` succeeds for the plugin crate
- [ ] Build smoke test: install via `/api/plugins/install` and `GET /api/plugins` returns it

---

## 18. Companion artifacts in this repo

| Path | Purpose |
|---|---|
| `crates/shiny-plugin-sdk/` | SDK crate — depends on this only. |
| `plugins/hello/` | Demo plugin from this doc, fully runnable. |
| `plugins/traveler/` | The traveler domain plugin — 22 tools (trips, GPS, maps, navigation, diary, planning, artifact cards), its own OSM client, navigation builder, diary writer, and prose pipeline. |
| `plugins/radio/` | **Self-contained** — `radio_search`/`radio_play`/`radio_stop` tools, the ICY `nowplaying` route (`RouteSpec`), and the Radio window (`web/plugin.js`) all ship in the plugin folder. The window keeps a singleton `<audio>` player; AI playback arrives as `radio_station` artifacts, stops via the `agent:actions` event. |
| `plugins/word/` | **Self-contained** — `doc_*` tools, its own `documents` table (`migrations/`), the `/api/documents` REST routes (`RouteSpec` + `route_handler`), and the Word window (`web/plugin.js`) all live in the plugin folder. Documents are real OpenDocument Text (`.odt`) bytes via the SDK codec (`crates/shiny-plugin-sdk/src/odt.rs`). Zero core footprint. |
| `plugins/calc/` | **Self-contained** — `calc_*` tools, its own `spreadsheets` table, `/api/spreadsheets` routes, and the Calc window (`web/plugin.js`) all in the plugin folder. Cell grids (A1 refs) stored as a JSON map; formulas (`=SUM(A1:A3)`) evaluate live in the window; real **`.ods` import/export** via `crates/shiny-plugin-sdk/src/ods.rs`. |
| `plugins/impress/` | **Self-contained** — `slide_*` tools, its own `presentations` table, `/api/presentations` routes, and the Impress window (`web/plugin.js`) all in the plugin folder. Decks are a JSON array of slides (six layouts, five themes, per-slide `transition` + `reveal` build); real **`.odp` import/export** via `crates/shiny-plugin-sdk/src/odp.rs`, which writes transitions as ODF `presentation:transition-style`. |
| `plugins/pdf/` | **Self-contained** — write documents and read PDFs. Documents made here are HTML on A4 pages (`web/editor.js`); their PDF is an *export* printed by the PEAK'D shell with Chromium (`peakd:print:` → `peakd_qt_print_html`), so it matches the editor exactly (server fallback: `src/compose.rs`). Any PDF opens in pdf.js; comments (highlight, notes, text boxes, ink, shapes, links), header & footer, watermark, page organisation and reading-order text go through **PdfCraft** (`src/craft.rs`, pinned git dependency, MIT OR Apache-2.0, forks patched in the root `Cargo.toml`). Every edit is undoable (`pdf_revisions`). |
| `plugins/keyboard/` | Pure surface plugin — virtual multi-language keyboard bar (8 layouts). Still **chrome-integrated** (`web/js/keyboard.js` + HUD toggle live in core, not the plugin folder): it is not a tile, so it awaits the "chrome surface" contract (see §20 item 7). |
| `plugins/youtube/` | **Self-contained** — `youtube_search`/`youtube_play` tools, the `/api/youtube/search` route (`RouteSpec`), and the YouTube window (`web/plugin.js`) all in the plugin folder. Search scrapes YouTube's public `ytInitialData` JSON (no API key); playback loads `youtube.com/embed/<id>`. |
| `plugins/calendar/` | **Self-contained** — `calendar_*` tools, its own `calendar_events` table (`migrations/`), the `/api/calendar/events` routes (`RouteSpec`), and the Calendar window (`web/plugin.js`) all in the plugin folder. Events are `YYYY-MM-DD` dates with optional `HH:MM` times; the window is a month grid + day detail. `src/date.rs` normalizes friendly dates/times ("today", "9:30am"). |
| `plugins/calculator/` | **Self-contained** — `calculator_*` tools, its own `calculator_history` table (`migrations/`), the `/api/calculator/eval` routes (`RouteSpec`), and the Calculator window (`web/plugin.js`) all in the plugin folder. One dependency-free scientific expression evaluator (`src/eval.rs`) is shared by the AI tool and the window, so both always agree. |
| `plugins/image/` | **Self-contained** — `image_*` tools (4), its own `images` table (`migrations/`), the `/api/images` routes (`RouteSpec`, 8, including the look-preview strip), and the Image window (`web/plugin.js` + `web/editor.js` + `web/api.js`) all in the plugin folder. A **minimal photo editor**: window bar + borderless canvas + bottom dock — a filmstrip of 16 curated looks (each previewed on the open image, no labels) and 8 adjustment sliders with live preview, centre-fill tracks and one-click tone actions — built from the core UI library with no plugin CSS, plus a DOM-shim smoke test (`web/plugin.smoke.mjs`). One shared Rust engine (`src/ops.rs`) powers the window's live sliders and the AI tools: tone/colour (`src/adjust.rs`), blur/sharpen/vignette (`src/filter.rs`), the 16 looks (`src/looks.rs`, built on photon-rs duotone/halftone/oil/pixelize plus film-style recipes) and document geometry. Single-image documents: `images.bytes` holds the current pixels and `images.original` the upload (the `reset` target); migration 006 drops the historical layer table. |
| `plugins/studio/` | **Self-contained** — `studio_*` tools, its own `studio_tracks` table (`migrations/`), the `/api/studio` routes (`RouteSpec`, incl. `GET /:id/audio` serving WAV bytes), and the Studio window (`web/plugin.js`) all in the plugin folder. Patterns (explicit `x..x` rhythms + Euclidean fills, voice kinds kick/snare/hat/bass/pluck/lead, plus swing/accent/velocity groove) render to WAV by its own offline DSP engine (`src/engine.rs` + `src/voices.rs` + `src/wav.rs`), built on `fundsp` for band-limited oscillators and filters with a `rustysynth` SoundFont sampler for user-supplied sampled instruments; the window is an Ableton/Bitwig-style DAW — Arranger timeline + Clip Launcher, a detail panel (Editor step-grid/piano-roll, Devices, Mixer), DAW keyboard shortcuts (Space/⌘S/⌘E/Del), ruler-click play start, piano-key audition and live scope/spectrum meters. |
| `plugins/browser/` | **Self-contained** — the in-app **Browser**: `browser_open`/`browser_search`/`browser_read` tools, `/api/browser/*` routes (`state`, `navigate`, `history`, `news`, `news/click`, `preview`, `settings`, `filter`, `filter/refresh`, `downloads`, `downloads/event`, `downloads/remove`, `downloads/clear`, sessions), its own `peakd_history` + `browser_settings` + `browser_downloads` tables (`migrations/`), the window (`web/plugin.js`), and the **whole filtering engine** (`crates/shiny-filter` + `crates/shiny-filter-core`). The window renders each tab in a **native child web view** owned by the shell (`crates/peakd` on Linux, `crates/peakd-mac` on macOS) at the page's true origin — no iframe, no filtering proxy in the page path. Ad blocking is **in-process**: the server compiles EasyList/EasyPrivacy into a cache the shell restores, and a `QWebEngineUrlRequestInterceptor` asks the engine per request; a **shield toggle** on the right of the toolbar turns it on/off live (persisted per user). The toolbar also has a **downloads manager** (every download from every site, with progress, pause/resume/cancel/retry/open, saved into the user's Files `Downloads`) and **incognito** tabs backed by an off-the-record profile. Google sign-in works via a per-host Firefox User-Agent override for `accounts.google.com`. Its `about:home` surface is a **related-news shelf ranked from what the user searches for** (`src/news.rs`): a decayed profile over `peakd_history` (typed searches weigh more than URL visits, `news_click` more still, and the word pairs of real queries are kept track of) → each interest searched as the *phrase the user typed*, not one word out of it → the same search engine the address bar uses → score by relevance, freshness tier, publisher and cross-interest overlap, with junk hosts, non-article URLs and one-publisher dominance filtered out → thumbnail cards. There is **no core fallback**: the browser, its filtering engine and its news shelf exist only when this plugin is installed. It is also the **default link handler** (`capabilities = ["open-urls"]`, §19): an `http(s)` link clicked anywhere in the app opens here as a new tab, unless the user picks another plugin that declares the capability in Settings → System → Links. |
| `plugins/files/` | **Self-contained** — a GNOME-style file browser over the user's home. Owns no database: each account's home is `$HOME/.shiny/home/<user-id>/` with the classic folders, provisioned by the `on_user_registered` hook (wired through `src/plugins/{loader,manager}.rs` + `src/api/auth.rs`) and lazily on first use. With `SHINY_LINUX_USERS=true` and `SHINY_HOME_MODE=real` the same browser operates on the account's **real Linux home** instead (core passes the resolved OS home as `x-shiny-os-home` on routes and `ToolRequest::os_home` for tools; classic folders are created only when missing). `file_*` tools + `/api/files/*` routes (`list`, `read`, `text`, `raw` with byte ranges, `thumb`, `download`, `upload`, `write`, `mkdir`, `rename`, `delete`→Trash, `restore`, `empty-trash`, `search`), a window (`web/plugin.js`) with lazy thumbnails (server-cached image PNG via `photon-rs`, PDF via vendored PDF.js, **video frames via server-side `ffmpeg`** — not a host `<video>`/QuickTime decode, so the same frame appears on macOS and Linux — text minis) and a Sushi-style Space quick-look with an app-styled streaming video player (byte-ranged from `/raw`, no re-encode), plus a `.Trash`. Deletes are recoverable; `fs_util::resolve` sandboxes every path to the user's home. Other apps save their exports here through the shared `/js/files.js` helper (`saveOrDownload`), which POSTs to `/api/files/upload` instead of triggering a browser download. **Double-clicking** a file opens it in the owning app (`/js/files.js` `openWithPlugin`/`onOpenFromFiles` — Word/Calc/Impress/PDF/Image); **Space** opens the quick-look. The quick-look renders Office documents through `/api/files/render` (`.odt`→HTML via the SDK, `.ods`→cell grid, `.odp`→slide data), not a raw text dump. |
| `web/js/files.js` | Shared desktop-save helper for every plugin window: `saveOrDownload(blob, {name, dir, app})` writes an export into the user's home via `/api/files/upload` and falls back to a browser download when the Files plugin is not installed. |
| `web/js/linkRouter.js` | Kiosk link routing: one delegated click listener routes app-drawn `http(s)` links to the user's link-handler plugin (`link.plugin` preference, `browser` default, any `open-urls` candidate otherwise) with the `openUrl`/`onOpenUrl` hand-off — see §19 "Opening links". |
| `src/plugins/loader.rs` | dlopen + cdylib scanner + symbol resolution. |
| `src/plugins/registry.rs` | `ToolRegistry` — the action key → `Arc<dyn Tool>` map. |
| `src/plugins/manager.rs` | `PluginManager` — aggregates contributions, persona, skills. |
| `src/plugins/installer.rs` | zip/tar.gz unpacker, manifest validation, log writer. |
| `src/plugins/admin_api.rs` | HTTP endpoints for install/uninstall/list. |
| `src/api/agent.rs` | System prompt = `web/skills/core-assistant.md` + active plugins' skills/persona. |
| `src/services/agent_tools.rs` | `execute_action`: registry first; core built-in = `web_search` only; traveler verbs refuse cleanly without the plugin. |
| `data/plugins/install.log` | Audit trail — written on every install/uninstall/error. |
| `web/ui/icons/` | The **base icon catalog**: every glyph the app and plugins resolve by name (`icon('ui/save')`, `<span data-icon="apps/files">`). Catalog in `web/ui/icons/INDEX.md`; prose in `web/ui/icons/README.md`. Resolution is active icon set → theme override (`web/themes/<theme>/icons/`) → this base. |
| `web/ui/iconsets/` | Selectable sets (Infinity default, auto light/dark; Slot-Beauty is the base). Each set ships `index.json` (names + tint palettes) and only the glyphs it draws differently. |
| `scripts/kde-icons/` | Curator for both sets: `convert.py --set base\|infinity\|infinity-dark` (sanitize + normalize + index), `mapping.json` / `mapping-infinity.json`, `fetch-infinity.py`, and `source/{dark,light}` (the committed Slot-Beauty curated SVGs). Raw sets live in `assets/iconsets/` (GPL-3.0, not committed). |
| `assets/iconsets/` | Raw GPL-3.0 icon sets — Slot-Beauty and Infinity (Breeze-derived) — a large pool of extra app, mimetype, category, emblem and place glyphs. Source for new icons; see `web/ui/icons/README.md`. |

---

## 19. UI & theming

The [shared component inventory](docs/core/ui-components.md) documents the control
APIs, mobile portrait/landscape rules, system-font policy and browser tests. Use
`element`, `toolbar`, `toolButton`, `menuButton`, `menuSurface`, `menuItem`, `selectControl`, `dial` and
`mediaCard` for common plugin UI; extend the library instead of duplicating them.

The shared **Flow** layer owns ordinary-control interaction across core and
plugins. Import `button`/`iconButton`, or `controlButton()` for an unskinned domain
button, from core's relative `ui/index.js`. Use the shared `element()` helper
instead of a plugin-local DOM factory. Use `controlGrid`, `controlCard`,
`controlGroup`, and `segmented` for reusable arrangements and choices.

Consume `--motion-feedback`, `--motion-reveal`, `--motion-connect`, and
`--motion-ease` in layout CSS. Use `enterSurface` for a positioned popup or
`revealRegion` for small dependent fields; dispose transitions with their owner.
Ordinary controls share motion; terminals, editors, canvases and media retain
their specialized behavior. Respect reduced motion, every Appearance option,
and translated labels. See [the shared interface contract](docs/plugins/authoring.md#shared-interface-contract)
and [Flow lifecycle rules](docs/core/web-ui.md#flow-shared-layout-and-motion).

Plugins build their window DOM using the **Shiny UI library**. Plugin-specific
CSS may arrange domain content, but control styling and motion stay in core so
plugin output follows the user's theme and accent. Tool output remains data;
core renders its artifacts through shared components.

The same split applies to tools: **core is a simple AI assistant** — its only
built-in tool is `web_search` (`web/skills/core-assistant.md`). Artifact cards
(`show_artifact`/`update_artifact`), trips, GPS, maps, navigation and diary
all belong to the traveler plugin; without it the frontend hides the map,
dock and panels, leaving the voice/text chat over the voice bar.

| Layer | Location | Role |
|---|---|---|
| Component library | `web/ui/` | Theme-agnostic engine: `theme-loader`, `appearance` (accent/gradient), `icon`, `reveal`, and all `.ui-*` components (`button`, `field`, `card`, `overlay`, `feedback`, `data`, `composites`). |
| Shared icons | `web/ui/icons/` | **The one icon library** — HUD, core windows, Files and every plugin's identity glyph. Catalog in `web/ui/icons/INDEX.md`, prose in `web/ui/icons/README.md`. |
| Themes | `web/themes/<name>/` | Skins: `theme.json` manifest, `tokens.css`, `components.css`, and `icons/` (SVG, `currentColor`) as an **override set** over the active icon set and base catalog. Installed themes are listed in `web/themes/themes.json`. See `web/themes/README.md`. |

### Icons

Every icon in the app resolves by **name** through `web/ui/icon.js`, which
fetches the active theme's override (`/themes/<theme>/icons/<name>.svg`) first
and then the shared icon (`/ui/icons/<name>.svg`). Use it from a plugin's
window code:

```js
import { icon, setIcon } from '/ui/index.js';   // when plugin web assets land

const wrap = document.createElement('span');
wrap.appendChild(icon('ui/folder-open', { size: 16 }));

// or fill an existing element:
await setIcon(el, 'ui/save', { size: 16 });
```

In static HTML, `<span data-icon="ui/search" data-icon-size="16">` is hydrated
at boot.

**Always reuse a name from the catalog** (`web/ui/icons/INDEX.md`) instead of
drawing new SVG — symbolic icons are `currentColor` and follow the theme for
free; coloured artwork (folders, plugin apps, file types) follows the accent
when *Settings → Appearance → Accent-tinted icons* is on. The names cover core
UI (`ui/settings`, `ui/puzzle`, `ui/close`, `ui/chevron-*`, `ui/search`,
`ui/list`, `ui/grid`, `ui/power`, …), the Files UI (`ui/folder` plus the
well-known variants `ui/folder-documents`, `ui/folder-music`, …; `ui/file`,
`ui/doc`, `ui/image`, `ui/video`, `ui/music`, `ui/archive`, `ui/download`,
`ui/home`, `ui/trash`, `ui/monitor`), the HUD (`hud/wifi-0…4`,
`hud/ethernet`, `hud/bluetooth`, `hud/volume-*`, `hud/battery-0…4`,
`hud/battery-charging`, `hud/clock`) and the app icons (`apps/<plugin>`).

The icon sets are curated from GPL-3.0 KDE sources: **Infinity**
(rogts/infinity-icon-theme, the default) and **Slot-Beauty**
(L4ki/Slot-Plasma-Themes, the base catalog), with in-house line art mixed in.
The raw sets live in `assets/iconsets/`, and `web/ui/icons/README.md` explains
how to pull one more into a set.

How plugin content reaches the eye:

- **Plugin windows** — every plugin with an interface lives inside its own
  **window** in the desktop shell (`web/js/tiles.js`, `#tile-grid`): the HUD
  header and the voice bar/dock are fixed chrome, and each plugin is an app
  with its own window container between them. The traveler plugin's window
  hosts the map. Every window has a slim **title bar**: **close** closes the
  window for this session (the plugin stays active, its tools keep working, and
  the tray can reopen it; a Craft app's engine stops gracefully), **fullscreen**
  fills the desktop without covering the HUD, and **focus** toggles focus mode.
  Deactivate lives in the Plugins page, the context/tray menus and the AI
  tools — never on the X. Opening a window is not activation: with
  `session.remember` on, only the windows the saved workspaces list are
  reopened. *Settings → Desktop → Layout* picks the desktop style — **Tiling**
  or **Columns**, or floating, draggable **Windows**. The AI can surface a
  window with the core `show_plugin` tool — only when the user asks to open,
  show or focus it, never just because it used the plugin's tools. Activating a
  plugin does not open its window either. The system prompt carries a compact
  catalog of active plugins (name + manifest description) and the response
  field `focus_plugin` focuses the explicitly requested window.
- **Artifacts** (the `artifact` field of `ActionOutcome`) are JSON rendered
  by the `artifactPanel` composite (`web/ui/components/composites.js`) as a
  **sheet inside their plugin's own window** (`.tile-sheet` over the plugin's
  UI), so a plugin's output is contained in its container. Core tags each
  saved artifact payload with the owning plugin's name (`plugin` key in
  `payload_json`) so output stays attributable. A plugin's styling levers are
  structural only: `type` / `theme` (icon + eyebrow + dock slot), `narrative`
  vs `sections`, `days[]`, `route`, `coordinates` / `geometry`.
- **Dock icons** come from the fixed `TYPE_ICONS` / `THEME_ICONS` maps in
  `composites.js` and resolve to the active theme's `icons/artifacts/*.svg`
  (falling back to `/ui/icons/`). For everything else — buttons, menus, status —
  reuse a name from the shared catalog (`web/ui/icons/INDEX.md`); never draw new
  SVG.
- **Accent & gradient** are chosen per user in *Settings → Appearance* and
  apply everywhere, including the Leaflet map colors and the voice bar glow
  (via the `appearance:change` window event).
- **When plugin web assets land** (roadmap item), plugin pages must load
  `/ui/ui.css` + `/ui/index.js` and build with those components instead of
  bundling their own styles — the theme keeps working for them for free.

### Window surface & top bar conventions

A plugin with a window ships `web/plugin.js`, whose default export is the
window surface (the `word` plugin is the reference):

```js
export default {
  name: 'word',           // matches plugin.toml `name`
  icon: 'ui/doc',         // legacy theme-icon hint; the window/tray icon now comes from web/icon.svg
  mount: mountWordTile,   // returns the window element (or null)
  unmount: unmountWordTile,
  getElement: getWordTileElement,
  wireEvents: wireWordEvents,
};
```

`mount()` returns a `section.tile <name>-tile` element carrying
`data-plugin="<name>"`; core tiles it, applies focus/fullscreen, and calls
`unmount()` on window close, deactivation or uninstall. Window styles live in
core's `web/css/tiles.css`
keyed by the plugin's class prefix (`word-*`, `calc-*`, `studio-*`, …) —
plugins build DOM with shared controls. Optional plugin stylesheets arrange
domain content without replacing shared control styling or motion.

#### Web-app plugins (`kind = "web"`)

A web-app plugin ships **no** `web/plugin.js`: its payload is a static site
(a wasm build of a desktop app, usually) whose `index.html` is the document
root of `/plugins/<name>/`. The shell mounts it for you through the same
window contract (`web/js/webAppSurface.js` builds one `section.tile webapp-tile`
holding an `<iframe>`), so the tray, launcher, workspaces, focus and fullscreen
all treat it like any plugin window — there is nothing to implement.

What the core does for these routes:

- **Framing:** `X-Frame-Options: SAMEORIGIN` with CSP `frame-ancestors 'self'`,
  instead of the `DENY`/`'none'` every other response carries. The document's
  own policy additionally allows `'unsafe-inline'` (the trunk-built page boots
  through an inline module script) and `'wasm-unsafe-eval'`/`'unsafe-eval'`
  (instantiating the wasm).
- **Caching:** content-hashed assets are `immutable` for a year and the entry
  document is `no-cache`, so a reload does not re-download tens of megabytes.
- **No sandbox attribute:** sandboxing same-origin content is a no-op without
  dropping `allow-same-origin`, and dropping it would take the app's storage
  (IndexedDB/OPFS, where its files live) with it. A web app is installed from
  its vendor's release and verified against the release's SHA-256 — the same
  trust class as a plugin cdylib.
- **Host context (language and theme):** the shell tells the app which
  interface language and theme the user chose in PEAK'D — not the OS language
  and not the browser's own color scheme. On every frame `load` it posts
  `{ type: "shiny:context", version: 1, theme, mode, locale, language, dir }`
  to the frame (`mode` is `"dark"` or `"light"`), and re-posts on
  `theme:change` / `locale:change`. The same values are mirrored on the frame
  element (`lang`, `data-shiny-theme`, `data-shiny-mode`,
  `data-shiny-locale`, `data-shiny-dir`), and `color-scheme` is set from
  `mode`, so an app that only reads `prefers-color-scheme` follows the
  shell's dark/light choice with no extra code. An app that wants the exact
  locale listens for the message:

  ```js
  window.addEventListener('message', (event) => {
    if (event.origin !== location.origin) return;
    const ctx = event.data;
    if (ctx?.type !== 'shiny:context') return;
    // ctx.locale / ctx.language / ctx.mode / ctx.theme / ctx.dir
  });
  ```

The app shares the shell's origin, so it also shares its storage namespace;
two apps that store under the same keys would collide (none do today).

### Opening links (`open-urls`)

In the kiosk the shell refuses page popups on purpose (a link must never leave
the shell), so core routes app-drawn links itself: `web/js/linkRouter.js`
installs one delegated click listener and sends every external `http(s)` click
to the user's **link handler** — the `browser` plugin by default, or any
installed plugin that declares `capabilities = ["open-urls"]` and ships a
window (the choice lives in Settings → System → Links, preference
`link.plugin`). Same-origin, `#`, relative and `mailto:` links keep their
default behaviour, and outside the kiosk the router is inert.

A plugin becomes a link handler with three things:

```toml
# plugin.toml
capabilities = ["open-urls"]
```

```js
// web/plugin.js — inside wireEvents()
import { onOpenUrl } from '../../js/linkRouter.js';
onOpenUrl('my-browser', (url) => openMyWindow(url));
```

Core activates the plugin, reveals its window (raised, deliberately not
focused — like every other "open with plugin") and then dispatches a `url:open`
window event with `{ plugin, url }`. `onOpenUrl` drains a hand-off that arrived
before the window existed (`window.__pendingUrlOpen`), so a link clicked while
the plugin is still mounting is not lost.

### Context menus (right-click)

Core owns one right-click menu engine (`web/js/contextMenu.js`). A right-click
inside the app resolves to a menu by context:

| Right-click target | Menu |
|---|---|
| A plugin window — **title bar or content** | Window menu: Focus, Full screen, Move to workspace, **the app's own entries**, Close window, Deactivate |
| A plugin icon in the top-bar tray | Plugin menu: Focus window / Full screen — or **Open window** when the plugin is active but its window is closed — then Activate or Deactivate |
| The top bar itself (HUD pill / workspace bar background) | Desktop menu (new/remove workspace, switch workspace, layout) |
| A workspace dot | Workspace menu (new / remove) |
| The empty desktop | Desktop menu |
| A text field (`input` / `textarea` / `contenteditable`) | Edit menu: Cut, Copy, Paste, Select All (core's own, through the clipboard service) |

Any other right-click inside the shell is swallowed — the app never shows the
platform's own menu (QtWebEngine's Back/Forward/Reload menu would otherwise
appear over chat compose, overlays or an open HUD popover). A right-click
inside a Browser-plugin **page** opens this same engine over the page: the
shell forwards the request and the page side opens `openContextMenu()` at the
cursor.

**An app contributes its own entries.** A plugin's window surface may export a
`contextMenu(ctx)` function. Core calls it when the window menu opens and
splices the returned entries between "Move to workspace" and the window
controls ("Close window" / "Deactivate"):

```js
// plugins/word/web/plugin.js — the reference implementation
export function wordContextMenu() {
  const hasDoc = !!currentDoc;
  return [
    { type: 'item', label: 'New document', icon: 'ui/plus', onClick: () => void newDocument() },
    { type: 'item', label: 'Save now', icon: 'ui/save', disabled: !hasDoc, onClick: () => void persist() },
    { type: 'separator' },
    { type: 'item', label: 'Delete document', icon: 'ui/trash', danger: true, disabled: !hasDoc, onClick: () => void removeCurrent() },
  ];
}

export default {
  name: 'word',
  mount: mountWordTile,
  unmount: unmountWordTile,
  getElement: getWordTileElement,
  wireEvents: wireWordEvents,
  contextMenu: wordContextMenu,   // ← core picks this up for right-click
};
```

Entry shapes — the same ones the desktop menus use:

| Shape | Fields |
|---|---|
| `{ type: 'item' }` | `label`, `onClick`, optional `icon`, `checked`, `disabled`, `danger` |
| `{ type: 'separator' }` | — |
| `{ type: 'heading' }` | `label` |
| `{ type: 'submenu' }` | `label`, `items`, optional `icon` |

Rules:

- `contextMenu(ctx)` receives `{ plugin, tile, target, source }`, where `target`
  is the element that was right-clicked. It may be called at any time — guard on
  the plugin's own state (mark items `disabled`, or omit them) instead of
  assuming a document is open.
- Return `[]` when nothing applies. Core wraps the call in `try/catch`, so a
  throw can never break right-click — but a `[]` keeps the console quiet.
- Core already brackets your entries with separators: don't add leading or
  trailing ones.
- `icon` is a theme icon name from `web/themes/<theme>/icons/ui/` (e.g.
  `'ui/plus'`, `'ui/save'`, `'ui/play'`); omit it if unsure.
- A plugin that needs a fully custom popup (not the window menu) can call the
  exported `openContextMenu(entries, x, y)` from `web/js/contextMenu.js`.
- A surface that copies or pastes should use the **core clipboard service**
  (`web/js/clipboard.js`): `copyText(text, { source: pluginName })` records the
  copy in the top-bar Clipboard history, `readClipboardText()` reads the OS
  clipboard (mirror fallback when the webview refuses the async read). See
  `docs/core/clipboard.md`.
- The **traveler** window is core-hosted (it is the Leaflet map and ships no
  `web/plugin.js`), so core supplies its app entry — *Center on my position*
  (`refreshGpsPosition()`).

### Plugin icon

Every plugin's identity icon comes from **`web/js/pluginIcon.js`**, which
resolves a plugin name in this order:

1. the **shared library icon** mapped to it (`PLUGIN_ICONS` → an
   `apps/<name>` glyph in `/ui/icons/apps/`),
2. the plugin's own **`web/icon.svg`**, served at `/plugins/<name>/icon.svg`,
3. the **core-window icon** (Settings → `ui/settings`, Plugins → `ui/puzzle`),
4. the fallback `ui/puzzle`.

Bundled plugins all have a curated `apps/<name>` icon (from the KDE
Slot-Beauty set), so they show that everywhere. A **third-party plugin** that
ships no icon falls back to `ui/puzzle`; ship a `web/icon.svg` (see *Icon
style* below) to have a real glyph, or a core mapping can be added.

The resolved icon is shown in:

- **Top-bar plugin tray** (`#hud-plugins`, to the right of the weather widget) —
  one icon per installed plugin, active or inactive. Icons are grouped by the
  plugin's `category` manifest field and the groups are separated by a thin
  divider. Clicking an inactive icon activates the plugin; clicking an active
  icon focuses its window.
- **Launcher** (the app grid) and the window/tile **dot** on the desktop.
- **Plugins window** — the plugin manager shows each plugin's icon instead of
  the generic puzzle glyph.

Plugins that don't declare a `category` are grouped under **Other**.

#### Adding or changing a plugin icon

Bundled plugins: add or edit an entry in `PLUGIN_ICONS` in
`web/js/pluginIcon.js` pointing at an `apps/<name>` icon, and make sure that
icon exists in `web/ui/icons/apps/` (see `web/ui/icons/README.md` for how to
curate one from the KDE set). The full catalog is
[`web/ui/icons/INDEX.md`](web/ui/icons/INDEX.md).

#### Icon style

A plugin's **own** `web/icon.svg` (fallback tier 2) shares the visual language
of the app mark (`web/favicon.svg`, the **PEAK'D!** logo) rather than the
thinner line style of the theme icons, because these surfaces paint them small —
18px in the HUD tray, 20px on the Plugins window. Follow these rules when
adding or restyling one:

- **Bold strokes.** `stroke-width="2"` on a 24×24 grid — the app mark's weight,
  sized so the strokes still read at 18px.
- **Rounded everything.** `stroke-linecap="round"` and `stroke-linejoin="round"`,
  with the same attributes on the root `<svg>` so glyph geometry stays soft.
- **Soft corners.** Prefer `rx="3"`–`"4"` on plates and frames; avoid sharp
  rectangles.
- **Fill small accents.** Draw dots, dials and play triangles with
  `fill="currentColor" stroke="none"` (like the mark's solid eye socket) so each
  icon carries one solid anchor. Everything else stays `fill="none"`.
- **Tight framing.** Push geometry out to roughly the `2.5 … 21.5` range so the
  glyph fills the box; do not leave the loose `2 … 22` padding of a thin-stroke
  icon.

Keep the icon single-color and script-free: core inlines the file into the DOM,
so `pluginIcon.js` rejects any SVG containing `<script>`, an `on*=` handler or a
`javascript:` URL, and hardcoded `fill`/`stroke` colors would break theme
inheritance. One recognizable glyph per plugin, not a wordmark or lettering.

> **Scope.** This style applies to `web/icon.svg` only — the HUD tray
> (`#hud-plugins`) and the Plugins window. Icons *inside* a plugin window are the
> regular theme icons from `web/themes/<name>/icons/` and keep their existing
> `stroke-width="1.5"` style; this restyle does not change them.

### Typography & the font picker

The app never fetches a webfont. The UI faces are **installed on the host** by
`scripts/install-fonts.sh` (Roboto + Inter from Debian, DM Sans / Space Grotesk /
Instrument Serif from the `google/fonts` repo), and the theme tokens name them
directly. There is no `@import url(fonts.googleapis.com…)` in `web/themes/*/`.

- **Global font** (*Settings → Appearance*) is written by `appearance.js`
  (`getFont`/`setFont`/`applyFont`) onto `--font-body` and `--font-display`;
  `--font-editorial` is left to the theme. `'theme'` clears the override.
- The picker is **not a baked-in list**: it reads `GET /api/fonts`
  (`src/api/fonts.rs` runs `fc-list`, shared through `web/js/fonts.js`), so a
  font the user installs later appears with no code change.

**The Writer's font menu.** `plugins/word/web/plugin.js` adds a font `select`
fed by the same `listFonts()`. With a non-collapsed selection it wraps the run in
`<span style="font-family:…" data-font="…">`; with a collapsed caret it sets the
document default (the editor plus its block children). The choice survives the
`.odt` round trip because the SDK codec carries `font-family` as an automatic
character style: `crates/shiny-plugin-sdk/src/odt.rs` registers a `TF<n>` style
per family and emits `fo:font-family` on export, then maps style-name →
`font-family` back to an inline `style` on import. Block-level and legacy
`<font face>` sources are handled too.

### Window background: the ambient blurred mirror

Every plugin window carries a **soft ambient glow** behind its content — a
blurred mirror of the window's subject. Core installs and paints it
(`web/ui/components/glow.js` + the `.tile-glow` block in `web/css/tiles.css`),
so plugins ship no CSS and do no blur work. Two tiers:

- **Tier 0 — free for every window.** When the window chrome is created,
  `tiles.js → ensureWindowChrome()` calls `installGlow(el, name)`, which paints a
  seeded two-blob radial gradient from the user's **accent** plus a per-window
  **partner hue** (the accent hue rotated by a name-seeded angle, so each window
  keeps its colour personality across reloads). No image, no blur pass.
- **Tier 1 — the plugin's real subject.** A window overrides the gradient with
  its actual content (album artwork, PDF page, photo, map view, track colour)
  from its `web/plugin.js`:

  ```js
  // Relative module specifiers: `plugins/<name>/web/plugin.js` is three
  // levels below the app root. Prefer relative imports over absolute `/ui/…`
  // paths so the window keeps working wherever the app is mounted.
  import {
    setTileGlow, setTileGlowFromUrl, glowGradient, glowFromDrawable,
  } from '../../../ui/index.js';

  setTileGlow(tileEl, glowGradient(colorA, colorB));   // sampled colours
  setTileGlow(tileEl, glowFromDrawable(canvasOrImg));  // canvas / photo frame
  await setTileGlowFromUrl(tileEl, artworkUrl);        // remote artwork / page
  setTileGlow(tileEl, null);                           // back to the Tier 0 colour glow
  ```

**Why the blur is cheap.** The layer deliberately has **no CSS spatial blur** —
a `blur()` at window size re-rasterizes on every resize frame, which was the
entire cost of this effect. Instead the blur is *baked once at thumbnail size*:
`glowFromImageUrl()` downscales to ~320px and bakes `blur(7px)`;
`glowFromDrawable()` uses 64px + `blur(2.5px)`. Scaled up to a window that reads
as a strong blur while costing nothing per frame. Tier 0 gradients are inherently
soft (a multi-stop falloff approximating a Gaussian), so they need no blur at all.

**Fallback when the content can't be sampled.** Cross-origin images without CORS
(and tainted canvases) must not break a window:

- `glowFromDrawable()` / `glowFromImageUrl()` return `null` on a tainted canvas
  or a failed load.
- `setTileGlowFromUrl()` then falls back to the **raw image URL plus a modest
  live `blur(12px)`** (the `.tile-glow--soft` class) instead of showing the image
  sharp. This is the *only* live-blur path, and only for that window.
- Passing `null` (or an empty URL) restores the Tier 0 colour glow. **The glow is
  never fully off** — every window always has some ambient light.
- Out-of-order loads are ignored (`tileEl.__glowSeq`), so flipping quickly
  through artwork/pages is safe.

**Placement, rim variant, theming.** `.tile` is an isolated stacking context and
`.tile-glow` sits at `z-index: -1` — above the window surface, below every child
— so it works in every plugin untouched. The map is opaque and full-bleed, so it
uses the **rim** variant (`createRim()` → `.tile-glow--rim`): the same glow drawn
*on top* at `z-index: 3` and masked to the window edges so the centre stays
readable. Light themes brighten instead of darken
(`glow.js` sets `data-glow-light` on the tile), because a dark blur turns white
pages gray. A theme tunes the effect from its `app.css` through the `.tile`
tokens `--glow-brightness`, `--glow-opacity`, `--glow-permeability` and
`--glow-veil`. Colours repaint
automatically on `theme:change` / `appearance:change` (`refreshGlow()`).

**User switch.** Settings → Appearance → *Window surface* has a **Blurred window
background** toggle (`desktop.surface.window_background`, default on).
`applyDesktopSurface()` writes `data-window-background="0"` on `<html>` and the
single rule `html[data-window-background="0"] .tile-glow { display: none }`
removes the mirror for every window at once. Plugins never check the flag — they
keep calling `setTileGlow()`/`setTileGlowFromUrl()` as documented above and the
global gate decides whether the layer is visible, so toggling it back on restores
every window with no re-render. The switch covers the colour glow *and* the
subject image; turning it off is what makes a lowered Window opacity read as a
plain translucent pane.

### PDF window

The PDF window renders **the real document in the browser**, not a picture of
it. This matters for the things users actually expect from a PDF viewer, and it
is the plugin most likely to be copied by another document-style window, so the
shape is worth stating.

**Why not server-rendered images.** An earlier version asked the server for a
PNG per page (`/api/pdfs/:id/pages/:page?dpi=…`) and put it in an `<img>`. That
gives you a bitmap you cannot select, cannot search, and can only "zoom" by
re-rasterising the whole page on the server — so text never gets sharper and
every zoom step costs a round-trip and a fresh image in memory.

**How it works now.**

- The plugin's own route `GET /api/pdfs/:id/file` serves the stored bytes
  inline (`application/pdf`, `no-store`) — deliberately **separate** from
  `GET /api/pdfs/:id/export`, which sets `Content-Disposition: attachment` for
  downloading. Keep them distinct: the viewer must never trigger a download.
- `web/plugin.js` loads PDF.js from the vendored `web/vendor/pdfjs/`, paints
  each page to a `<canvas>` at `zoom × devicePixelRatio`, and lays PDF.js's
  **text layer** over it. Text is therefore selectable and copyable, and zoom is
  crisp at every step with no server involvement.
- `cMapUrl` / `standardFontDataUrl` point at the vendored `cmaps/` and
  `standard_fonts/` so CID-keyed documents and PDFs without embedded fonts still
  render. Those directories are part of the vendored dependency — don't drop
  them.
- Thumbnails come from the same loaded document (two passes: quick, then sharp),
  so the rail costs no extra network requests.

**Gotchas learned here.**

- **Never let a flex parent size the page.** `.pdf-canvas` and `.pdf-page` set
  `align-items: flex-start`, and the viewer pins the page box from the PDF.js
  viewport. Flex's default `stretch` otherwise overrides the page height and
  silently changes its aspect ratio — pages render squashed, circles become
  ellipses.
- **Set `--scale-factor`** on the page element (the value PDF.js positions text
  spans with); it must equal the viewport scale or the text layer drifts.
- **The annotation gesture is Shift+drag.** A plain drag belongs to text
  selection now. Region selection converts to PDF points via the viewport scale
  and posts the rect in points (A4 = 595×842), which is what
  `/api/pdfs/:id/annotate` expects. The bar reads the current text selection
  *before* dismissing it, so **Note**, **Text box** and the extra
  **Find & replace…** entry arrive pre-filled with the words the user pointed
  at; `showAnnotBar()` must capture the selection before anything clears it.
- **Capture `annotRect` before hiding the action bar.** The bar's
  document-level `pointerdown` handler runs *before* a button's `click`, so
  hiding the bar first leaves the rect null and every annotation silently
  no-ops.
- **After any server-side edit**, call `dropPdfDoc()` and re-render. The bytes
  changed, so the parsed document and its pages are stale. `rotate`,
  `reorder`, `delete-pages`, `merge`, `replace-text`, `annotate` and `watermark`
  all take this path — including when the agent triggers them (`agent:actions`).

**Documents are edited as documents; a PDF is their export.** Rewriting a
PDF's text in place (pdf_oxide's `modify_text`, then a hand-written
content-stream editor, then PdfCraft's own `replace_block`) was never reliable
enough to be the way users edit what they wrote — especially on Chromium-made
PDFs, where a rewritten paragraph landed on top of the table below it. The
window therefore has a real editor (HTML on A4 paper) and **exports** it: the
shell prints the same HTML with Chromium (`peakd:print:` →
`window.__peakdPrinted`, capability flag `window.__peakdCaps.print`), so the
PDF is exactly what was on screen. PDFs from elsewhere get comments, marks,
page operations and find & replace — not in-place retyping.

PdfCraft supplies the PDF-side engine (annotations with appearance streams,
removable marks, page organisation, reading-order text); its patched
hayro / vello / lopdf forks are pinned in the workspace `[patch]` to the same
commit as the plugin's `pdfcraft-*` dependencies. Every edit from the window or
the agent is undoable (`POST /api/pdfs/:id/undo` / `redo`, the `pdf_undo`
tool).

Style changes (bold, size, colour) are **not** supported, and the toolbar controls
for them were removed rather than left pretending to work. This is worth spelling
out, because an attempt to add them looked successful and was not:

- A run's `Tf` (font, size) and `rg` (colour) are **inherited graphics state**.
  They stay in force for every following text object, so restyling one run
  repaints everything after it unless the previous state is explicitly restored.
- **Most runs never set their own `Tf`.** In the sample document 850 of 890 text
  objects inherit it, so a style edit aimed at one of them silently matches
  nothing while still reporting success.
- Consequently an "apply style" that reports `applied: 1` can be a **no-op**, and
  the naive fix leaks. Both were observed; the feature was reverted rather than
  shipped. Doing it properly needs save/restore of the inherited state around the
  edited run.

**This was attempted four more times and every attempt failed verification**, so
it is withdrawn: `stream_edit::style_text` exists and is documented, but no route
is registered for it and the toolbar has no formatting controls. If you pick it
up, the traps are known rather than hypothetical:

- **A single edit works.** Verified: `size`/`bold`/`colour` apply to one run with
  exactly one changed band, neighbours pixel-identical, logo and page intact.
- **It is not idempotent, and that is the blocker.** Each call appends another
  `Tf` plus its undo instead of reusing the existing slot, so a second call on
  the same document grows the stream (observed `Tf` 41→43→45→47→49→51) and the
  later value stops winning. A stream cannot tell our injected `Tf` from the
  document's own, so idempotence needs either marked injections or the run's byte
  offsets recorded alongside the stored document. Neither is implemented.
- **Never insert a `Tf` in front of the object's own one.** Every earlier
  variation of this failed: the object's `Tf` wins, and each insert puts the
  *older* edit closer to the text, so `size:22` could never undo a `size:30`.
- **Do not overwrite the object's `Tf` in place either.** That operator is what
  every following sibling block inherits, so changing it restyles the rest of the
  block (one attempt restyled all five address lines and garbled them).
- **`q`/`Q` around the text object is not a fix.** `Q` restores the state from
  *before* the object, discarding a `Tf` a preceding sibling set.
- **`TJ` numbers are already em-relative** — thousandths of a unit of text space,
  scaled by `Tf`. Rewriting them (scaling by `new/old`, or substituting one
  nominal correction) double-applies and visibly mis-spaces the word.
- **When editing the content-stream code, make sure the pass that copies the
  untouched operators survives your edit.** One attempt removed that copy loop,
  the rebuilt stream began mid-object, and the page silently lost its header —
  while the API still returned success. Every change here needs a render
  comparison against the original, not an API "success".

Matching is also by plain glyph bytes, so replacement is correct for simple fonts
(e.g. WinAnsi) but would need ToUnicode/CMap mapping for CID-keyed fonts.

### Notifications

The core provides a unified, GNOME-style notification banner system
(`web/ui/components/notifications.js`). Banners stack top-center just below the
HUD, show an icon, an app label, a title (summary), a body, and optional action
buttons, and dismiss on click or after their urgency timeout: **low** ≈ 4s,
**normal** ≈ 7s, **critical** stays until dismissed. Every plugin should use
this for user-visible events (a track started, a message arrived, a file
exported, an error needing attention) instead of ad-hoc toasts or in-window
text.

**From a plugin web surface** — import `notify` from the UI library:

```js
import { notify } from '../../../ui/index.js';

notify({
  app: 'Mail',               // app label shown above the title
  title: 'New message',      // GNOME "summary"
  body: 'Alice: "See you at 8"',
  icon: 'ui/mail',           // theme icon path, or a plugin icon element
  urgency: 'normal',         // 'low' | 'normal' | 'critical'
  actions: [{ label: 'Open', action: 'open' }],
  timeout: 0,                // ms; 0 = sticky until dismissed
});
```

Pressing an action button dispatches `app:notification-action` with
`{ id, action, notification }`. The same options object can be sent as the
global `app:notify` CustomEvent.

**From a plugin tool** — attach a `Notification` to the tool's `ActionOutcome`:

```rust
use shiny_plugin_sdk::{ActionOutcome, Notification};

Ok(ActionOutcome::ok("radio_play", json!({ "station": "Radio Bruno" }))
    .with_notification(
        Notification::new("Now playing: Radio Bruno")
            .title("Radio")
            .urgency("normal")
            .plugin("radio")            // banner shows the plugin's web/icon.svg
            .action("Stop", "stop"),
    ))
```

The notification is serialized into the outcome's `data["notification"]` and
the core frontend renders it automatically — no `ActionOutcome` ABI change is
needed. `Notification` builders: `.title()`, `.urgency()`, `.icon()` (theme
icon path), `.plugin()` (plugin name), `.app()`, `.action(label, key)`,
`.timeout(ms)`.

**When to use `notify` vs `toast`.** `toast()` stays for immediate, synchronous
feedback on an action the user just performed ("Saved", "Imported", "Could not
open file"). `notify()` is for things that happen in the background or deserve
the user's attention even when they are looking elsewhere: a new message, an
event about to start, a track change. Built-in examples: **calendar** raises an
"Event starting soon" notification ~10 minutes before a timed event; **mail**
raises "New mail" when the unread count grows; **radio** raises "Now playing"
on track changes. A window plugin should start its polling in `mount()` and
stop it in `unmount()`, exactly like calendar/mail/radio do.

The top bar follows one convention (Studio sets the standard):

- **Single top bar, working tools inline.** Every window renders exactly one
  top bar (`*-bar`); its working tools — formatting, navigation, zoom, view
  toggles, transport — sit directly in it. No second toolbar row below the
  bar: the bar wraps (`flex-wrap`) on narrow widths instead. Document windows
  (Word, Impress, PDF) keep only one grey **New** pill of the file actions in
  the bar; import / save / export / delete live in the document menu's footer
  (`appendMenuFooter` in `web/js/officeMenu.js`), in one order everywhere:
  **new, import, save, export, delete** (import `ui/folder-open`, save
  `ui/save`, export `ui/upload` — never the same icon for save and export).
- **Order, left → right.** Menu button (icon + chevron, opens the documents /
  sheets / decks / images list), title, primary editing actions, then the
  New pill and the save state.
- **One height.** All bars share Studio's metrics: `padding: 6px 10px`,
  `gap: 8px`, `32×32` icon buttons, title at 15px/600. Keep these values so
  every window's bar is the same height.
- **Buttons.** Square icon buttons built with `button({ icon, variant: 'ghost' })`
  plus the plugin's own `*-tool` class (e.g. `word-tool`, `calc-tool`,
  `studio-transport`), each with a `title` and `aria-label`. Menu buttons
  (icon + chevron) are 32px tall.
- **Save state.** The editorial windows (Word, Impress, PDF) show it as small
  mono capitals — `SAVED 14:02` / `UNSAVED` (Word's `.word-state`, Impress's
  bar meta, PDF's status line) — not a glowing dot. Windows not yet moved to
  the editorial language still use a small round dot (`*-save-dot`, e.g.
  `calc-save-dot` / `studio-save-dot`): muted when saved, accent (`.is-active`)
  while there are unsaved changes.
- **Merged into the window controls.** Mark a window's top bar with
  `bar.dataset.windowBar = ''` (the `data-window-bar` attribute) and the host
  merges it with the window's close/fullscreen controls into a single row — no
  separate title bar, so the window spends one row of chrome instead of two.
  The host paints the row's plate and bottom border: the bar must paint no
  background of its own (a second glass layer over the header reads as a
  differently toned patch beside the controls in glass themes), and its
  classes must never reuse the core `tile-header*` names — injected chrome is
  marked `data-window-chrome`, but a misnamed bar still takes the wrong
  styling. Windows whose top row is content rather than tools (the map, files,
  an idle YouTube player) omit the marker and keep the centered title bar.

#### A window must not cache server-owned addresses

Any plugin that hands a client a server-owned URL which can change (a random
loopback port, a signed URL) must return the address with the payload and have
the window re-read it rather than remember it from mount: a stale address reads
as "the feature is broken", not "the address changed".

> **The Browser no longer renders through a proxy.** It used to: the window
> showed a proxied iframe whose origin was a loopback filter proxy, and that
> address changed on every server start. Now each tab is a native child web
> view owned by the shell (`crates/peakd` on Linux, `crates/peakd-mac` on
> macOS), at the page's true origin — there is no proxy address to cache, and
> Cloudflare sees an ordinary browser. `crates/shiny-filter` still runs
> server-side for the news shelf and link previews; it is not in the page path.
>
> ```bash
> cargo build --release && cargo build --release -p shiny-browser-plugin
> cp target/release/libshiny_browser_plugin.dylib data/plugins/browser/
> ```

> **Editing a window surface during development** — the app serves
> `data/plugins/<name>/web/` (the installed copy), not `plugins/<name>/web/`
> (the source). After editing `plugins/<name>/web/plugin.js` or
> `plugins/<name>/web/icon.svg`, copy it to `data/plugins/<name>/web/`
> (or reinstall) or the browser keeps loading the stale file.

---

### Translating a plugin (i18n)

The shell's UI is translated through JSON catalogues
([interface language](docs/core/i18n.md)); a plugin joins in the same way.

**Window surface (`web/plugin.js`, `web/hud.js`)**

```js
import { t, tp } from '../../js/i18n.js';   // relative: works through the kiosk proxy

t('plugin.hello.toolbar.greet', null, 'Greet');          // key, params, English fallback
tp('plugin.hello.count', n, { count: n }, `${n} items`); // plurals: key.one / key.other
```

- Catalogues live at `plugins/<name>/web/i18n/<code>.json` (flat JSON, keys
  fully qualified with the plugin name). The core loads the active plugin's
  catalogue before importing its module and reloads it when the interface
  language changes.
- **Always pass the English fallback** — a plugin with no catalogues then
  behaves exactly as before.
- Add `plugin.<name>.meta.description` / `.meta.summary` / `.meta.category` to
  the catalogue: `/api/plugins` serves the localized card text from there
  (falling back to `plugin.toml`), so the Plugins window and the launcher show
  translated cards even while the plugin is inactive.
- `web/js/tests/i18n.test.mjs` checks every `plugin.<name>.*` key used in your
  JS against your catalogue — a missing entry fails the build.

**Status lines (agent tools)**

`Tool::step_label()` / `humanize()` are English. To have the dock's status line
follow the user's language, return a keyed `UiText` from the additive hooks:

```rust
fn step_ui(&self) -> Option<UiText> {
    Some(UiText::keyed("steps.listing", "Listing files…"))
}
fn done_ui(&self, _result: &str, data: &Value) -> Option<UiText> {
    Some(UiText::keyed("steps.listed", "Listed the files")
        .param("count", data.get("count").cloned().unwrap_or(0.into())))
}
```

The key is **relative to the plugin**; the host prefixes it with
`plugin.<name>.` before it goes on the wire, so the catalogue entry is
`plugin.<name>.steps.listing`. `None` (the default) keeps the English label.

**Tool errors** are translated server-side by matching the English message
against the catalogues' values — add an entry for a user-facing error (e.g.
`plugin.hello.error.noName` = `"a name is required"`) and it comes back in the
caller's language. Dynamic messages without an entry stay English.

## 20. Roadmap

The v1 plugin system delivers a **self-contained plugin model**: a plugin ships `plugin.toml` + cdylib + `migrations/` + `skills/` + `web/` (+ `RouteSpec` routes) and the core needs zero edits to host it — tools, schema, REST routes and window UI all live in the plugin folder. `word`, `calc`, `impress`, `radio` and `youtube` are migrated to this model; `hello` needs nothing; `keyboard` and `traveler` remain chrome-integrated (item 7).

Remaining follow-up work:

1. **Identity refactor** — move the `users` / `auth_tokens` / `is_admin` columns out of `travelers` so a plugin-free bootstrap is fully self-contained.
2. ~~**Hot router swap**~~ — **done**: `main.rs` wraps the router in an `ArcSwap` + `RouterHandle` make-service; `RouteSpec` routes mount live on install/uninstall via `router_rebuild`, with auth middleware per `RouteSpec.auth`.
3. **Cron hook activation** — read `CronSpec` from `RegistryBuilder` and spawn a per-plugin scheduler that calls a pluggable `cron_handler(tag, ctx)`.
4. ~~**Plugin web asset serving**~~ — **done**: `data/plugins/<name>/web/` is served at `/plugins/<name>/`, and the frontend dynamically loads each enabled plugin's `web/plugin.js` window surface (no hardcoded tile registry).
5. **Signature verification** — turn on ed25519 signature enforcement behind the `signed-plugins` feature flag.
6. **Traveler extraction (partial)** — the 22 traveler verbs (plus `show_artifact`/`update_artifact`) live in `plugins/traveler/`; core keeps only `web_search`. Traveler's REST handlers, gpsd, `DiaryGenerator` cron, and the map/dock/HUD chrome remain in core.
7. **Chrome surface contract** — `keyboard` (bottom bar + HUD toggle) and `traveler` (map, dock, saved-trips HUD) are *chrome*, not tiles. Define a `chrome.js` surface contract so those too can ship entirely inside their plugin folders.
8. **Safe plugin data access** — `rt::bridge` now uses a per-plugin single-thread `current_thread` dispatcher, and a synchronous `ctx.db()` (`libsqlite3-sys` FFI) accessor is implemented and unit-tested. Both still segfault in the server because the plugin links its **own** `libsqlite3` (compiled with `-DSQLITE_ENABLE_MEMORY_MANAGEMENT`), so multiple SQLite copies coexist in one process. Fix by (a) routing plugin DB through the host's single SQLite copy, or (b) building SQLite without `SQLITE_ENABLE_MEMORY_MANAGEMENT` (see §15 caveat).
7. **Plugin uninstall path with DB rollback** — drop plugin-owned tables when an admin request explicitly asks for it.

Roadmap changes must preserve the v1 binary layout or use an explicit coordinated SDK rebuild; `api_level` alone does not establish binary compatibility.
