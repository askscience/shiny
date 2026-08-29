# Plugin system

`shiny` is built around an **AI sphere** — a conversational agent driven by Ollama with the orb UI, voice (Vosk STT, Supertonic TTS), web search, and an artifact dock at its core. **Every domain beyond that lives in a plugin.**

This document explains the architecture, the trait surface, the installer workflow, and a worked example. After reading it you can write, build, package, install, and uninstall a plugin.

> **One-line summary:** drop a `.zip` or `.tar.gz` containing `plugin.toml` + `lib<my_plugin>.so` onto `POST /api/plugins/install` and the plugin's tools become callable by the live AI sphere — no restart needed.

---

## 1. Architecture

```
                  shiny (core binary)
            ┌──────────────────────────────┐
            │  AI sphere                   │
            │  ├─ orb UI / sphere          │
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
| The orb canvas and the front-end shell | Skill markdown advertised to the LLM |
| `PluginManager`, `ToolRegistry`, installer, admin API | A **persona fragment** ("…a travel navigator AI…") |
| `data/plugins/install.log` (the install audit trail) | Front-end bundle under `web/` |

### Without any plugins the app is "just the AI sphere"

Start `shiny` with an empty `PLUGINS_DIR` and you get an orb that listens, speaks, replies, and has the built-in generic tools only (`web_search`, `show_artifact`, `update_artifact`). Everything else is opt-in.

---

## 2. Workspace layout

```
shiny/
├── Cargo.toml                       # [workspace] root, lists ./, SDK and the plugins
├── crates/
│   └── shiny-plugin-sdk/            # public trait surface + shared types
│       ├── Cargo.toml
│       └── src/
│           ├── lib.rs                # re-exports
│           ├── plugin.rs              # the `Plugin` trait + PLUGIN_ENTRY_SYMBOL
│           ├── tools.rs               # `Tool` trait + `RegistryBuilder`
│           ├── outcome.rs            # `ActionOutcome`
│           ├── context.rs            # `AgentContext` (per-request agent state)
│           ├── services.rs           # `PluginCtx` + lazy `OllamaClient`, `SearchService`, `SupertonicClient`, pool
│           ├── rt.rs                 # `bridge()` — plugin-owned Tokio runtime (§15)
│           ├── artifacts.rs          # `Artifact` value type + `build_from_params`
│           ├── navigation.rs        # `NavigationSession` (optional navigator payload)
│           ├── manifest.rs           # `Manifest` struct
│           ├── routes.rs             # `RouteSpec`, `HttpMethod`
│           ├── crons.rs              # `CronSpec`
│           └── migrations.rs         # per-plugin migration runner
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
# signature = "<hex ed25519>"                  # optional; see §12
```

Fields:

| Field | Type | Required | Notes |
|---|---|---|---|
| `name` | string | yes | unique; used as the install directory name |
| `version` | semver | yes | surfaced in `/api/plugins` |
| `api_level` | u32 | yes | if `> CORE_API_LEVEL` the install is rejected |
| `entry_symbol` | string | yes | usually `"shiny_plugin_entry"` |
| `target_triple` | string | no | when present the installer refuses mismatched platforms |
| `description` | string | no | short human title |
| `author` | string | no | attribution |
| `summary` | string | no | used by admin UI to say "what this plugin adds" |
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
        └── plugin.js
```

You can produce one with any packaging tool:

```bash
zip -r hello.zip hello/
tar czf hello.tar.gz hello/
```

### Admin API

| Method | Path | Auth | Description |
|---|---|---|---|
| `GET` | `/api/plugins` | none | List installed plugins |
| `POST` | `/api/plugins/install` | `ADMIN_TOKEN` env or DB `is_admin=1` | Multipart upload `file=@archive` |
| `POST` | `/api/plugins/uninstall` | admin | JSON body `{"name":"hello"}` |

Authenticate the install:

```bash
curl -X POST http://localhost:8080/api/plugins/install \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
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
| `PLUGINS_DIR` | `data/plugins` | Where plugins live + the `install.log`. |
| `ADMIN_TOKEN` | unset | If set, this string is the admin bearer. Otherwise, install/uninstall require a user row with `is_admin=1`. |
| `CORE_TRAVELER_BUILTIN` | `true` | When `true`, the embedded traveler tools (`src/services/agent_tools.rs`) still answer actions the plugin didn't claim. Set `false` to make core truly sphere-only. |

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

No auth. Returns installed plugins:

```json
{
  "success": true,
  "data": [
    { "name": "hello", "version": "0.1.0", "api_level": 1, "description": "Demo plugin..." }
  ]
}
```

### `POST /api/plugins/install`

Multipart form upload. Auth: `Authorization: Bearer $ADMIN_TOKEN`, OR a user row with `is_admin=1`.

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

Keeps the plugin installed and its database tables intact, but disables its tools. The agent stops advertising the deactivated plugin's tools (their `doc_fragment` snippets are omitted from the skills markdown) and `ToolRegistry::invoke` returns `"plugin 'hello' is deactivated"` if the LLM still tries one. The disabled set is persisted to `data/plugins/disabled.json` so the state survives restarts.

### `GET /api/plugins/install.log`

No auth. Returns the last 200 lines of `PLUGINS_DIR/install.log` as `text/plain`. Useful for plugin managers and for post-incident forensics.

### Activation state

Every plugin is either **Active** (default after install) or **Off** (deactivated). Activating / deactivating:

- Updates `GET /api/plugins` `enabled` boolean.
- Persists to `data/plugins/disabled.json` (a small JSON file with one `disabled: ["name", ...]` array).
- Affects the agent immediately: deactivated plugins contribute no `doc_fragment`, no `persona` fragment, no `context_lines`, and their `Tool::invoke` short-circuits with a friendly `BadRequest`.

The separation between **deactivate** (keep install dir + tables, just turn tools off) and **uninstall** (drop install dir + library unload) mirrors WordPress's "deactivate" vs "delete" distinction. Deactivation is reversible; uninstall requires re-uploading the archive.

---

## 14. Security model

- Plugins are full Rust `cdylib`s loaded into the same process. They have full process privileges — they can read SQLite rows, make HTTP calls, spawn threads, write files. **Don't install plugins you don't trust.**
- Admin authorization only requires a single env var (`ADMIN_TOKEN`) or an `is_admin=1` user row. Protect that token as you would an SSH key.
- The `install.log` makes post-incident forensics possible but does not prevent malicious plugins.
- For multi-tenant hosting, plan to (a) require `signature` validation, (b) sandbox long-running plugins behind an IPC process boundary. Both are roadmap items; v1 is single-tenant.

---

## 15. ABI stability & the runtime bridge

`api_level` is the version of the SDK's public trait surface. Plugins declare the level they were built against. The runtime accepts plugins where `api_level ≤ CORE_API_LEVEL`. The current level is **1**.

When the SDK adds fields or methods without breaking existing trait impls, `CORE_API_LEVEL` increments but **existing plugins keep working** because `default` trait impls are used.

When a method signature changes or a required method is added without a default, `CORE_API_LEVEL` increments a major step and **existing plugins must be rebuilt**. We commit to never silently breaking plugins within an `api_level`.

### Two load-bearing rules (learned from production crashes)

A plugin cdylib statically links **its own copies** of Tokio, sqlx/libsqlite3, and reqwest/hyper. Nothing bound to a runtime or a C allocator may cross the dlopen boundary:

1. **Always wrap tools with `bridged(...)` at registration.** The adapter runs `invoke` on a runtime the *plugin* owns (`shiny_plugin_sdk::rt::bridge`) and returns the result over an executor-agnostic channel. Without it, the first sqlx/reqwest/tokio-time call inside a tool aborts the host with *"this functionality requires a Tokio context"*.
2. **Only use the async accessors on `PluginCtx`** — `ctx.pool()`, `ctx.ollama()`, `ctx.search()`, `ctx.supertonic()`. Each lazily opens a **plugin-owned** connection/client inside the plugin's runtime. Never accept a live `SqlitePool` or pre-built reqwest client from the host: values allocated by the host's libsqlite3 segfault when freed by the plugin's copy, and host-built HTTP clients panic when driven from the plugin's reactor. Migrations are the exception by design — they run host-side, with the host pool, at load time.

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
| `plugins/radio/` | Internet radio via Radio Browser — `radio_search`/`radio_play`/`radio_stop` tools plus the Radio window (`web/js/radio.js`) with a singleton `<audio>` player; AI playback arrives as `radio_station` artifacts, stops via the `agent:actions` event. |
| `plugins/word/` | Simple word processor — `doc_create`/`doc_write`/`doc_append`/`doc_read`/`doc_list`/`doc_delete` tools plus the Word window (`web/js/word.js`). Documents are real OpenDocument Text (`.odt`) bytes in the core-owned `documents` table; the ODT↔HTML codec lives in `crates/shiny-plugin-sdk/src/odt.rs`, and core serves `/api/documents` (list/create/get/save/delete/import/export) — the same interim pattern as `/api/radio/nowplaying` until plugin routes land (roadmap #2). |
| `plugins/calc/` | Simple spreadsheet — `calc_create`/`calc_write`/`calc_read`/`calc_list`/`calc_delete` tools plus the Calc window (`web/js/calc.js`). Sheets are cell grids (A1-style refs) stored as a JSON map in the core-owned `spreadsheets` table; formulas (values starting with `=`, e.g. `=SUM(A1:A3)`) evaluate live in the window with SUM/AVERAGE/MIN/MAX/COUNT, ranges, `+ − * / ^`. Full user-facing editor: toolbar (new/import/export/delete), formula bar, autosave, CSV import/export — and real **OpenDocument Spreadsheet (`.ods`) import/export** via the SDK codec (`crates/shiny-plugin-sdk/src/ods.rs`, self-contained, no office suite). Core serves `/api/spreadsheets` (list/create/get/save/delete/import/export) — the same interim pattern as `/api/documents`. |
| `plugins/keyboard/` | Pure surface plugin — a virtual multi-language keyboard bar (`web/js/keyboard.js` + `web/css/keyboard.css`) at the bottom of the screen that types into any focused input. Deliberately registers **no skills, tools or persona** — the agent never sees it; activation only mounts the UI. 8 layouts (EN/IT/ES/FR/DE/RU/EL/AR incl. RTL), touch devices suppress the native OS keyboard while it is active. |
| `plugins/youtube/` | YouTube window — an embedded player (`web/js/youtube.js`) plus `youtube_search`/`youtube_play` tools. Search scrapes YouTube's public `ytInitialData` results JSON (no API key); playback loads `youtube.com/embed/<id>` (the only YouTube path that allows iframing — watch/search pages send `X-Frame-Options: SAMEORIGIN`). Result cards carry a `youtube_play` action, so tapping one starts the video in the window. |
| `src/plugins/loader.rs` | dlopen + cdylib scanner + symbol resolution. |
| `src/plugins/registry.rs` | `ToolRegistry` — the action key → `Arc<dyn Tool>` map. |
| `src/plugins/manager.rs` | `PluginManager` — aggregates contributions, persona, skills. |
| `src/plugins/installer.rs` | zip/tar.gz unpacker, manifest validation, log writer. |
| `src/plugins/admin_api.rs` | HTTP endpoints for install/uninstall/list. |
| `src/api/agent.rs` | System prompt = `web/skills/core-assistant.md` + active plugins' skills/persona. |
| `src/services/agent_tools.rs` | `execute_action`: registry first; core built-in = `web_search` only; traveler verbs refuse cleanly without the plugin. |
| `data/plugins/install.log` | Audit trail — written on every install/uninstall/error. |

---

## 19. UI & theming

Plugins never ship CSS, and they cannot inject HTML into the frontend. Every
visual surface a plugin can reach is rendered by core through the **Shiny UI
library**, so plugin output always matches the user's theme and accent.

The same split applies to tools: **core is a simple AI assistant** — its only
built-in tool is `web_search` (`web/skills/core-assistant.md`). Artifact cards
(`show_artifact`/`update_artifact`), trips, GPS, maps, navigation and diary
all belong to the traveler plugin; without it the frontend hides the map,
dock and panels, leaving the voice/text chat over the orb.

| Layer | Location | Role |
|---|---|---|
| Component library | `web/ui/` | Theme-agnostic engine: `theme-loader`, `appearance` (accent/gradient), `icon`, `reveal`, and all `.ui-*` components (`button`, `field`, `card`, `overlay`, `feedback`, `data`, `composites`). |
| Themes | `web/themes/<name>/` | Skins: `theme.json` manifest, `tokens.css`, `components.css`, and `icons/` (SVG, `currentColor`). Installed themes are listed in `web/themes/themes.json`. See `web/themes/README.md`. |

How plugin content reaches the eye:

- **Plugin windows** — every plugin with an interface lives inside its own
  **window** in the app's tiling shell (`web/js/tiles.js`, `#tile-grid`),
  Android Auto-style: the HUD header and the AI sphere/dock are fixed chrome,
  and each plugin is an app with its own window container between them. The
  traveler plugin's window hosts the map. Windows have **no title bar**; a
  single visible window auto-fills the grid area but keeps the rounded frame.
  *Settings → Plugin Windows* picks Tile or Full screen per plugin
  (localStorage `plugin.layout.<name>.<userId>`, default `tile`), and the AI
  can surface a window with the core `show_plugin` tool (system prompt
  carries a compact catalog of active plugins — name + manifest description —
  and the response field `focus_plugin` focuses that window).
- **Artifacts** (the `artifact` field of `ActionOutcome`) are JSON rendered
  by the `artifactPanel` composite (`web/ui/components/composites.js`) as a
  **sheet inside their plugin's own window** (`.tile-sheet` over the plugin's
  UI), so a plugin's output is contained in its container. Core tags each
  saved artifact payload with the owning plugin's name (`plugin` key in
  `payload_json`) so output stays attributable. A plugin's styling levers are
  structural only: `type` / `theme` (icon + eyebrow + dock slot), `narrative`
  vs `sections`, `days[]`, `route`, `coordinates` / `geometry`.
- **Dock icons** come from the fixed `TYPE_ICONS` / `THEME_ICONS` maps in
  `composites.js` and resolve to the active theme's `icons/artifacts/*.svg`.
- **Accent & gradient** are chosen per user in *Settings → Appearance* and
  apply everywhere, including the Leaflet map colors and the orb canvas
  (via the `appearance:change` window event).
- **When plugin web assets land** (roadmap item), plugin pages must load
  `/ui/ui.css` + `/ui/index.js` and build with those components instead of
  bundling their own styles — the theme keeps working for them for free.

---

## 20. Roadmap

The v1 plugin system delivered here implements everything described above plus the `hello` demo end-to-end. The remaining items, scheduled for follow-up work:

1. **Identity refactor** — move the `users` / `auth_tokens` / `is_admin` columns out of `travelers` so a plugin-free bootstrap is fully self-contained.
2. **Hot router swap** — wire `AppState::router_rebuild` to an `ArcSwap<Router>` so `RouteSpec`-contributed routes appear live without restart.
3. **Cron hook activation** — read `CronSpec` from `RegistryBuilder` and spawn a per-plugin scheduler that calls a pluggable `cron_handler(tag, ctx)`.
4. **Plugin web asset serving** — mount `data/plugins/<name>/web/` at `/plugins/<name>/` automatically.
5. **Signature verification** — turn on ed25519 signature enforcement behind the `signed-plugins` feature flag.
6. ~~**Traveler plugin extraction**~~ — **done**: all 22 traveler verbs (plus `show_artifact`/`update_artifact`) live in `plugins/traveler/`; core keeps only `web_search`. Traveler REST handlers + gpsd + core's `DiaryGenerator` cron remain in core until items 2/3 land.
7. **Plugin uninstall path with DB rollback** — drop plugin-owned tables when an admin request explicitly asks for it.

Nothing here changes the v1 contract — plugins written against `api_level=1` keep working across the roadmap.