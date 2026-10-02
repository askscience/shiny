# hello — demo plugin

> Part of the Shiny plugin set · [Plugins overview](../../../docs/plugins/README.md)

`hello` is the **canonical reference plugin**. It is deliberately tiny: one
trivial agent tool, one migration file, no REST routes and no window surface.
Its job is to prove the whole plugin pipeline end-to-end — manifest parsing,
`dlopen`, `register()`, tool registration, migration application, and agent
invocation — and to serve as the smallest template a new plugin can be copied
from.

| | |
|---|---|
| **Category** | `System` |
| **Version** | `0.1.0` |
| **API level** | `1` |
| **Author** | `shiny demo` (runtime manifest) |
| **Rust crate** | `shiny-hello-plugin` → `libshiny_hello_plugin.so` |
| **Agent tools** | 1 — [`hello`](./tools.md) |
| **REST routes** | none |
| **Window surface** | none (`web/` ships only `icon.svg`) |
| **Database tables** | `hello_pings` (+ SDK meta `plugin_schema_versions`) |
| **Runtime dependencies** | `shiny-plugin-sdk`, `async-trait`, `serde`, `serde_json`, `semver` |

## Manifest

`plugins/hello/plugin.toml` is what the installer reads; `src/plugin.rs` builds
the `Manifest` the host holds at runtime. They differ slightly — worth knowing
when copying this plugin as a template.

| Field | `plugin.toml` | Runtime `Manifest` (`src/plugin.rs:14`) |
|---|---|---|
| `name` | `hello` | `hello` |
| `version` | `0.1.0` | `semver::Version::new(0,1,0)` |
| `api_level` | `1` | `1` |
| `entry_symbol` | `shiny_plugin_entry` | `PLUGIN_ENTRY_SYMBOL` |
| `description` | `Demo plugin registering a single 'hello' tool` | `A demo plugin that adds a single \`hello\` tool.` |
| `summary` | `Demo plugin: hello tool` | `Demo plugin: adds one \`hello\` agent tool.` |
| `author` | *absent* | `shiny demo` |
| `migrations_dir` | `migrations` | `migrations` |
| `skills_dir` | `skills` | `skills` |
| `web_dir` | `web` | `web` |
| `category` | `System` | *(not part of the runtime `Manifest`; read from TOML by the host)* |

Because `web/plugin.js` is absent, `GET /api/plugins` reports this plugin with
`surface: false` even though `web_dir` is configured.

## What it adds

At `Plugin::register` (`plugins/hello/src/plugin.rs:39`):

```rust
builder
    .persona("") // hello plugin adds no persona fragment.
    .skills("- `hello` — Say hello to someone. params: `{ name?: string }`")
    .tool_arc(shiny_plugin_sdk::tools::bridged(
        std::sync::Arc::new(crate::tool::HelloTool),
    ));
```

| Contribution | Value |
|---|---|
| **Persona** | none (`""`) |
| **Skills markdown** | one inline line (the shipped `skills/hello.md` is *not* used — the builder string is inline) |
| **Context line** | none |
| **Tool** | `hello` (wrapped in `bridged(...)`) |
| **Route** | none |
| **Cron** | none |

The shipped `plugins/hello/skills/hello.md` documents the same tool for humans;
the LLM sees the inline `.skills(...)` string above. See
[PLUGINS.md §8](../../../PLUGINS.md) for how skills markdown is folded into the
system prompt.

## Database

`migrations_dir = "migrations"` ships `plugins/hello/migrations/001_init.sql`.
The core installer runs plugin SQL with `shiny_plugin_sdk::migrations`; applied
files are recorded per-plugin in `plugin_schema_versions(plugin, file,
applied_at)`.

```sql
CREATE TABLE IF NOT EXISTS hello_pings (
    id   INTEGER PRIMARY KEY AUTOINCREMENT,
    who  TEXT NOT NULL,
    at   TEXT NOT NULL DEFAULT (datetime('now'))
);
```

The demo tool does **not** currently write to `hello_pings` — the table exists to
demonstrate the migration layout. A plugin that does persist should use
`ctx.db()` (the synchronous, plugin-owned connection) rather than `ctx.pool()`.

| Table | Columns | Written by |
|---|---|---|
| `hello_pings` | `id`, `who`, `at` | *nothing yet* (layout demo) |
| `plugin_schema_versions` | `plugin`, `file`, `applied_at` | SDK migration runner |

## Source map

| File | Purpose |
|---|---|
| `plugins/hello/plugin.toml` | Installer manifest: name, version, dirs, category |
| `plugins/hello/Cargo.toml` | `cdylib` + `rlib` crate `shiny-hello-plugin` |
| `plugins/hello/src/lib.rs` | Module root; re-exports `HelloPlugin` |
| `plugins/hello/src/plugin.rs` | `Plugin` impl, runtime `Manifest`, `register()`, C entry symbol |
| `plugins/hello/src/tool.rs` | `HelloTool` — the single `Tool` impl |
| `plugins/hello/skills/hello.md` | Human-facing skill note (same contract as the tool) |
| `plugins/hello/migrations/001_init.sql` | `hello_pings` migration |
| `plugins/hello/web/icon.svg` | Plugin icon (no `plugin.js` → no window) |

## Build, package, install

The crate is a workspace member (root `Cargo.toml`), so a normal workspace build
produces the cdylib:

```bash
# From the repo root.
cargo build --release -p shiny-hello-plugin
# → target/release/libshiny_hello_plugin.so
```

Package it exactly as the installer expects (manifest + skills + migrations +
cdylib at the archive root or one level deep):

```bash
mkdir -p /tmp/pkg/hello/{skills,migrations}
cp plugins/hello/plugin.toml        /tmp/pkg/hello/
cp plugins/hello/skills/hello.md    /tmp/pkg/hello/skills/
cp plugins/hello/migrations/*.sql   /tmp/pkg/hello/migrations/
cp target/release/libshiny_hello_plugin.so /tmp/pkg/hello/
( cd /tmp/pkg && zip -r hello.zip hello )

curl -X POST http://localhost:8080/api/plugins/install \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -F "file=@/tmp/pkg/hello.zip"
```

## Dev notes

- **Add a tool:** copy `src/tool.rs`, implement `Tool`, then add it in
  `register()` with `builder.tool_arc(bridged(Arc::new(MyTool)))`. Always wrap
  with `bridged(...)` — it moves the call onto the plugin-owned runtime (see
  [PLUGINS.md §15](../../../PLUGINS.md)).
- **C entry symbol:** `shiny_plugin_entry()` must return
  `Box::into_raw(Box::new(HelloPlugin))`; the loader transmutes and calls it.
- **Migrations are append-only** by filename order; never edit an already
  applied `.sql` file — add `002_*.sql` instead.
- **Persistence:** prefer `ctx.db()` (synchronous, single plugin thread) over
  `ctx.pool()` to avoid cross-thread SQLite frees in a cdylib.

## Tools summary

| Tool | Aliases | Step label | Params | Returns |
|---|---|---|---|---|
| [`hello`](./tools.md) | — | `Saying hello…` | `{ name?: string }` | `{ who, reply }` |

Full detail (params, returns, errors, code refs) in **[tools.md](./tools.md)**.

## Routes summary

None. `hello` registers no `RouteSpec`s and overrides no `route_handler`.
