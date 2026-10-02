# Plugin architecture

This document describes how the Shiny core hosts plugins: the split of
responsibilities, the workspace layout, the load/register lifecycle, boot-time
discovery, the hot router swap, activation, and uninstall.

> Read [Runtime & ABI](runtime-abi.md) for the dlopen-boundary rules before
> writing any plugin that touches a database or the network.

---

## 1. Core vs plugin

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
  └─ skills/*.md     ─▶ ToolRegistry ─────┘
```

| Owned by core | Owned by plugins |
|---|---|
| HTTP server (axum), auth middleware, Bearer tokens | Tool implementations |
| `OllamaClient`, `SearchService`, `SupertonicClient`, voice/STT plumbing | REST routes for the plugin's domain |
| Matching between an **LLM action block** and the right tool | Migrations (one or more `.sql` files) |
| The voice bar and the front-end shell | Skill markdown advertised to the LLM |
| `PluginManager`, `ToolRegistry`, installer, admin API | A **persona fragment** ("…a travel navigator AI…") |
| `data/plugins/install.log` | A front-end bundle under `web/` |

Without plugins the app is "just the AI assistant": a voice bar that listens,
speaks, replies, and has only the built-in generic tools.

---

## 2. Workspace layout

```
shiny/
├── Cargo.toml                       # [workspace] root, lists ./, SDK and the plugins
├── crates/
│   └── shiny-plugin-sdk/            # public trait surface + shared types
│       └── src/
│           ├── lib.rs                # re-exports + CORE_API_LEVEL
│           ├── plugin.rs             # the `Plugin` trait + PLUGIN_ENTRY_SYMBOL
│           ├── tools.rs              # `Tool` trait + `RegistryBuilder` + `bridged()`
│           ├── outcome.rs            # `ActionOutcome`
│           ├── context.rs            # `AgentContext`
│           ├── services.rs           # `PluginCtx` + Ollama/Search/Supertonic + ConfigSnapshot
│           ├── rt.rs                 # `bridge()` — plugin-owned Tokio runtime
│           ├── artifacts.rs          # `Artifact` + `build_from_params`
│           ├── navigation.rs         # `NavigationSession`
│           ├── manifest.rs           # `Manifest`
│           ├── routes.rs             # `RouteSpec`, `HttpMethod`, handler types
│           ├── crons.rs              # `CronSpec`
│           ├── notification.rs       # `Notification`
│           ├── db.rs                 # synchronous plugin DB handle
│           └── migrations.rs         # per-plugin migration runner
├── plugins/<name>/                   # one self-contained plugin each
└── src/plugins/                      # loader/registry/manager/installer/admin_api
```

A plugin author needs only the **`shiny-plugin-sdk`** crate plus the host
`axum`/`tokio` ecosystem.

---

## 3. Lifecycle

### Boot discovery

[`main.rs`](../../src/main.rs) calls
`state.plugins.discover_and_install(base_ctx)`, which walks `PLUGINS_DIR` and
installs every directory containing `plugin.toml`. Backup dirs (`<name>.bak`),
hidden dirs and leftover `_staging-*` dirs are skipped.

### Runtime install

`POST /api/plugins/install` carries the same logic at runtime (no restart).

### Loading a plugin, in order

1. Parse `plugin.toml`.
2. Validate `api_level` and (optional) `target_triple`.
3. Locate the cdylib inside the archive, copy it to a versioned name so
   re-installs are safe even on Windows, and `dlopen` it.
4. Resolve `shiny_plugin_entry` and call it; receive `*mut dyn Plugin`, wrap as
   `Box<dyn Plugin>`.
5. Run any `migrations/*.sql` not yet recorded in the core
   `plugin_schema_versions` table (host pool, load time).
6. Build a `PluginCtx` and call `plugin.register(ctx, &mut builder)`.
7. Move `builder.tools` into the live `ToolRegistry`, append `skills_md` +
   `persona` + `context_lines` to the per-plugin contribution list, and resolve
   each `RouteSpec` tag to a handler.
8. Call `plugin.on_load(ctx)`.
9. Rebuild the served router.

### Every `/api/agent` call

The concatenated skills markdown + persona + context lines enter the system
prompt, and any registered tool is dispatched through `ToolRegistry::invoke`.

---

## 4. The installer

### Archive layout

A plugin archive is a directory containing **at least** `plugin.toml` and a
cdylib (`.so`, `.dylib` or `.dll`). The directory may be the archive root or a
child.

```
hello.zip
└── hello/
    ├── plugin.toml
    ├── libshiny_hello_plugin.so
    ├── migrations/001_init.sql      # optional
    ├── skills/hello.md              # optional
    └── web/plugin.js                # optional window + icon.svg
```

### Workflow (mirrors WordPress)

1. Receive the multipart upload. Sniff the first bytes: `PK` → zip, `1f 8b` →
   tar.gz, `ustar` → plain tar. Anything else → `400 Unrecognised archive
   format`.
2. Extract into a `_staging-<pid>-<ts>` directory.
3. If the archive contains exactly one subdirectory with a `plugin.toml`,
   descend into it.
4. Parse `plugin.toml`; validate `api_level ≤ CORE_API_LEVEL`; validate
   `target_triple` against the host when present.
5. Locate a cdylib; reject if none.
6. Acquire the install lock on `PLUGINS_DIR/.install.lock`.
7. If `PLUGINS_DIR/<name>/` exists, back it up as `<name>.bak`.
8. `rename` staging → `PLUGINS_DIR/<name>/`; clean staging.
9. Load (dlopen, entry, migrations, register).
10. Contributions become live.
11. `on_load(ctx)` runs.
12. On any error: roll back the install dir and return `400`/`500`.

### Hot reload

New tools are surfaced on the **next** `/api/agent` call automatically. Routes
go through an `ArcSwap<Router>` rebuild triggered by `router_rebuild`.

### Roll-back

If a prior install of the same name exists it is renamed to `<name>.bak` before
the new copy becomes live. To roll back: uninstall, move the `.bak` back, and
restart (or reinstall a fresh archive).

---

## 5. Activation vs install

- **Install** is server-wide (the cdylib is shared).
- **Activation** is **per user**, stored in `user_plugin_states`.

`session_active_set` computes the active set:

| `session.remember` | Active set |
|---|---|
| on | installed minus explicitly-disabled |
| off | only explicitly-enabled (opt-in, starts empty) |

Deactivating a plugin keeps its install dir and tables but removes its tools
from the prompt and makes `ToolRegistry::invoke` return
`"plugin '<name>' is deactivated for this user"`.

---

## 6. Uninstall

`PluginManager::uninstall(name)` removes the contribution, removes the plugin's
tools **before** retiring the library (so no `Arc<dyn Tool>` outlives its
cdylib), then unloads. The previous install stays as `<name>.bak` until the next
reinstall overwrites it. Tables are left in place — dropping them is the
operator's call.

---

## 7. Source map

| Path | Role |
|---|---|
| [`src/plugins/loader.rs`](../../src/plugins/loader.rs) | `dlopen`, cdylib scan, symbol resolution, `on_load`/`on_user_registered`, `route_handler`. |
| [`src/plugins/registry.rs`](../../src/plugins/registry.rs) | `ToolRegistry`, aliases, per-user activation gate, skills markdown. |
| [`src/plugins/manager.rs`](../../src/plugins/manager.rs) | `PluginManager`, contributions, per-user activation, uninstall. |
| [`src/plugins/installer.rs`](../../src/plugins/installer.rs) | Archive unpack, manifest validation, install log. |
| [`src/plugins/admin_api.rs`](../../src/plugins/admin_api.rs) | Plugin HTTP endpoints. |
| [`src/api/mod.rs`](../../src/api/mod.rs) | Plugin route mounting + `inject_path_params`. |
| [`src/main.rs`](../../src/main.rs) | `RouterHandle` hot swap + boot discovery. |
| [`crates/shiny-plugin-sdk/`](../../crates/shiny-plugin-sdk) | The public API. |
| [`plugins/hello/`](../../plugins/hello) | Canonical example. |

Next: [Authoring](authoring.md) · [API reference](reference.md) ·
[Runtime & ABI](runtime-abi.md).
