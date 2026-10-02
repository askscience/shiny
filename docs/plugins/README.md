# Plugin system

Shiny is built around an **AI assistant** — a conversational agent driven by Ollama
with the voice bar UI, voice input/output, web search, and an artifact dock at
its core. **Every domain beyond that lives in a plugin.**

This directory is the source of truth for the plugin system. It supersedes the
root `PLUGINS.md` while preserving its hard-won operational detail.

> **One-line summary:** drop a `.zip` or `.tar.gz` containing `plugin.toml` +
> `lib<my_plugin>.so` onto `POST /api/plugins/install` and the plugin's tools
> become callable by the live AI assistant — no restart needed.

## What a plugin is

A plugin is a Rust `cdylib` that implements the
[`Plugin`](reference.md#the-plugin-trait) trait from the
[`shiny-plugin-sdk`](../../crates/shiny-plugin-sdk/src/lib.rs) crate. The core
binary loads it with `libloading`, calls its exported `shiny_plugin_entry`
symbol, runs its migrations, and asks it to `register()` tools, HTTP routes,
cron specs, skills markdown, and a persona fragment. Everything a plugin
contributes becomes live in the running server.

A plugin archive is a directory containing, at minimum, `plugin.toml` and one
cdylib (`.so` / `.dylib` / `.dll`). It may also ship `migrations/`, `skills/`,
and `web/` (a window surface + icon).

## Core vs plugin responsibilities

| Owned by core | Owned by plugins |
|---|---|
| HTTP server (axum), auth middleware, Bearer tokens | Tool implementations |
| `OllamaClient`, `SearchService`, `SupertonicClient`, voice/STT plumbing | REST routes for the plugin's domain |
| Matching between an **LLM action block** and the right tool | Migrations (one or more `.sql` files) |
| The voice bar and the front-end shell | Skill markdown advertised to the LLM |
| `PluginManager`, `ToolRegistry`, installer, plugin admin API | A **persona fragment** ("…a travel navigator AI…") |
| `data/plugins/install.log` (the install audit trail) | A front-end window bundle under `web/` |

Without any plugins the app is "just the AI assistant": a voice bar that listens,
speaks, replies, and has only the built-in generic tools. Everything else is
opt-in.

## Document index

| Document | What it covers |
|---|---|
| [Architecture](architecture.md) | Core vs plugin split, load/register lifecycle, hot router swap, discovery, uninstall. |
| [Authoring](authoring.md) | Step-by-step authoring with a complete worked `hello` example, build, package, install. |
| [API reference](reference.md) | Every trait, struct, field, method, and helper in the SDK. |
| [Runtime & ABI](runtime-abi.md) | The dlopen boundary, `api_level`, `bridged()`, the SQLite caveat, accessor rules. |
| [Migrations](migrations.md) | Per-plugin schema, `plugin_schema_versions`, immutability, idempotency, repair patterns. |
| [Security](security.md) | Trust model, missing admin role, planned signatures, install log format. |
| [Troubleshooting](troubleshooting.md) | Symptom → cause → fix table, install log, debugging surfaces. |

## Quick start

```bash
# Build the whole workspace (binary + every plugin cdylib).
cargo build --release --workspace

# Package the reference plugin.
mkdir -p /tmp/pkg/hello/skills /tmp/pkg/hello/migrations
cp plugins/hello/plugin.toml          /tmp/pkg/hello/
cp plugins/hello/skills/hello.md      /tmp/pkg/hello/skills/
cp plugins/hello/migrations/001_init.sql /tmp/pkg/hello/migrations/
cp target/release/libshiny_hello_plugin.so /tmp/pkg/hello/
( cd /tmp/pkg && zip -r hello.zip hello )

# Install on a running server (any logged-in user's bearer token).
curl -X POST http://localhost:8080/api/plugins/install \
  -H "Authorization: Bearer $TOKEN" \
  -F "file=@/tmp/pkg/hello.zip"
# {"success":true,"data":{"installed":"hello"}}

# Verify.
curl http://localhost:8080/api/plugins
```

Full walkthrough: [Authoring](authoring.md).

## The two load-bearing rules

A plugin cdylib statically links **its own copies** of Tokio, sqlx/libsqlite3,
and reqwest/hyper. Nothing bound to a runtime or a C allocator may cross the
dlopen boundary:

1. **Always wrap tools with `bridged(...)` at registration** so `invoke` runs on
   a runtime the *plugin* owns. Without it, the first sqlx/reqwest/tokio-time
   call inside a tool aborts the host with *"this functionality requires a
   Tokio context"*.
2. **Only use the plugin-owned accessors on `PluginCtx`** —
   `ctx.pool()`, `ctx.ollama()`, `ctx.search()`, `ctx.supertonic()`, and the
   synchronous `ctx.db()`. Never accept a live `SqlitePool` or pre-built reqwest
   client from the host. Migrations are the exception: they run host-side with
   the host pool at load time.

Read [Runtime & ABI](runtime-abi.md) before writing any DB or HTTP code.

## Key facts

| Fact | Value |
|---|---|
| Core API level | `CORE_API_LEVEL = 1` (`crates/shiny-plugin-sdk/src/lib.rs`) |
| Entry symbol | `shiny_plugin_entry` |
| Manifest file | `plugin.toml` → `shiny_plugin_sdk::manifest::Manifest` |
| Install log | `PLUGINS_DIR/install.log` (default `data/plugins/install.log`) |
| Default plugin dir | `PLUGINS_DIR`, default `data/plugins` |
| Auth | Any logged-in user; **no admin role** (see [Security](security.md)) |
| Deactivated state | Per-user in the `user_plugin_states` table |

## Source map

| Path | Role |
|---|---|
| `crates/shiny-plugin-sdk/` | The public trait surface every plugin compiles against. |
| `plugins/hello/` | The canonical worked example. |
| `src/plugins/loader.rs` | `dlopen` + cdylib scanner + symbol resolution. |
| `src/plugins/registry.rs` | `ToolRegistry` — action key → `Arc<dyn Tool>`. |
| `src/plugins/manager.rs` | `PluginManager` — contributions, persona, skills, activation. |
| `src/plugins/installer.rs` | zip/tar unpacker, manifest validation, install log. |
| `src/plugins/admin_api.rs` | Plugin HTTP endpoints. |
| `src/api/mod.rs` | Plugin route mounting + `inject_path_params`. |
| `src/main.rs` | `RouterHandle` hot swap + boot discovery. |
| `web/js/pluginIcon.js` | Plugin icon resolution. |
| `web/js/activePlugins.js` | Frontend active-plugin set. |
| `PLUGINS.md` | The original root document (kept; this set supersedes it). |
