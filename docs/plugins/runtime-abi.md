# Runtime & ABI

A plugin is a native Rust `cdylib` loaded into the server process. This brings
two hard constraints: the **SDK ABI** across `dlopen`, and the fact that a
plugin statically links its **own copies** of Tokio, sqlx/libsqlite3 and
reqwest/hyper.

Source: [`crates/shiny-plugin-sdk/src/rt.rs`](../../crates/shiny-plugin-sdk/src/rt.rs),
[`tools.rs`](../../crates/shiny-plugin-sdk/src/tools.rs),
[`routes.rs`](../../crates/shiny-plugin-sdk/src/routes.rs),
[`util.rs`](../../crates/shiny-plugin-sdk/src/util.rs),
[`services.rs`](../../crates/shiny-plugin-sdk/src/services.rs),
[`db.rs`](../../crates/shiny-plugin-sdk/src/db.rs).

---

## API level and binary compatibility

`api_level` is a source-level plugin API convention. The loader accepts plugins
where `api_level ≤ CORE_API_LEVEL` (currently **1**), but this comparison does
**not** prove binary compatibility. The entry point returns a Rust trait object;
Rust does not give it a stable C ABI, even though the entry symbol uses `extern C`.

Keep the existing `Plugin` trait, entry signature, `Manifest`, contexts and shared
SDK structures frozen for compatible host-only refactors. Adding a default trait
method or a struct field is not a safe binary-compatible change: it may change a
vtable or memory layout. Such changes need an explicit coordinated rebuild and
compatibility validation. Use matching SDK sources, compiler and target for new
native plugin builds. A stable C or process-based ABI would be a separate versioned
migration, not an incidental cleanup.

Optional build diagnostics may be included in `plugin.toml`, parsed host-side
without extending the SDK's `Manifest`:

```toml
[build]
rustc = "compiler version used for this archive"
target = "x86_64-unknown-linux-gnu"
sdk_revision = "source revision used to build the SDK"
```

Legacy archives without this table remain accepted. Metadata is diagnostic and
is not a guarantee that a library is safe to load. Native compatibility tests
compare registered tools and routes against `tests/fixtures/plugin-contracts.json`.
They stage explicit build artifacts into temporary directories and do not start
plugin `on_load` work or read the kiosk's installed copies.

---

## The two load-bearing rules

> **1. Always wrap tools with `bridged(...)` at registration.**
> **2. Only use the async accessors on `PluginCtx` for DB/HTTP.**

### Why

A plugin cdylib links its own Tokio, sqlx/libsqlite3 and reqwest/hyper.
Nothing bound to a runtime or a C allocator may cross the boundary:

- A tool invoked on the **host's** runtime hits sqlx/reqwest/tokio-time with no
  Tokio context → *"this functionality requires a Tokio context"* → abort.
- A **host-passed `SqlitePool`** carries libsqlite3 objects; freeing them with
  the plugin's copy segfaults.
- A **host-built reqwest client** captures the host reactor; driving it from the
  plugin's runtime panics (`there is no reactor running`).

Migrations are the deliberate exception: they run host-side, with the host pool,
at load time.

---

## `rt::bridge` — the plugin-owned runtime

`bridge(fut)` queues the future onto the **plugin's own single-threaded**
(`current_thread`) runtime and hands the result back over an executor-agnostic
oneshot channel. Each plugin cdylib statically links its own copy of the SDK,
so the worker and its queue are per plugin, not process-global. The host worker
never blocks.

```rust
// Wrong: aborts the host on the first sqlx/reqwest call.
builder.tool(MyTool);

// Right:
builder.tool_arc(bridged(Arc::new(MyTool)));
```

Routes use the same pattern via `bridged_route`:

```rust
fn route_handler(&self, tag: &str) -> Option<RouteHandler> {
    match tag {
        "list" => Some(bridged_route(|req| async move { Ok(list_handler(req).await?) })),
        _ => None,
    }
}
```

### Single-threaded on purpose

SQLite values are not thread-safe; a `fetch_*` future that migrates across Tokio
worker threads can call `sqlite3_value_free` on a different thread than it was
created, segfaulting the process. One thread avoids that.

### Panic isolation

Each job is awaited under `catch_unwind`. A panicking plugin future is logged
and the worker keeps running; the awaiting host gets an `AppError`
(`"plugin tool panicked before returning a result"`), never a hang or an abort.

### Lifecycle hooks

`on_load`/`on_unload`/`on_user_registered` run on the **host** executor, so a
bare `tokio::spawn` panics with *"there is no reactor running"* — and unwinding
across the boundary aborts the process. Use `rt::spawn(fut)` there instead.

---

## The SQLite caveat

A plugin `cdylib` statically links its own `libsqlite3` (via `libsqlite3-sys`,
built with `-DSQLITE_ENABLE_MEMORY_MANAGEMENT`), so the running server can hold
**multiple SQLite libraries** in one process. This is known to segfault in
`sqlite3_value_free` / `sqlite3LockAndPrepare` for plugin-owned DB access.

Mitigations in place:

- `rt::bridge` runs everything on one thread (removes the cross-thread value
  case).
- `ctx.db()` is a **synchronous** `libsqlite3-sys` connection: prepare/bind/
  step/finalize all on the one plugin runtime thread. Prefer it over
  `ctx.pool()`.

The definitive fix (roadmap) is to route **all** plugin data access through the
host's single SQLite copy, or to build SQLite without
`SQLITE_ENABLE_MEMORY_MANAGEMENT`. See [PLUGINS.md §15/§20](../../PLUGINS.md).

| Accessor | Shape | When to use |
|---|---|---|
| `ctx.db()` | synchronous `libsqlite3-sys` | **Preferred** for plugin data. |
| `ctx.pool().await` | async `sqlx::SqlitePool` | Only when you need sqlx; riskier. |
| host pool | — | Never (migrations excepted). |

---

## Identity across the boundary

Each plugin's copy of axum has different `TypeId`s than the host's, so request
extensions do not cross. Core therefore passes portable data as **headers**:

- `x-shiny-user-id`, `x-shiny-traveler-id` — identity.
- `x-shiny-path-params` — captured path params as a JSON array (because axum
  stores them in a private extension type). Core **removes any client-supplied
  value** before injecting the real params, so no plugin can be fed path params
  on a route that has none.
- `x-shiny-os-user`, `x-shiny-os-home`, `x-shiny-os-uid` — Linux-user identity.
  Client-supplied copies are stripped unconditionally; they only ever come from
  core after it resolved an OS account.
- `x-shiny-remote` — set by the Iroh proxy; `x-forwarded-for`/`-proto`/`-host`
  — added by Tailscale Serve/Funnel and other reverse proxies. Presence of any
  means the request is remote; `is_remote_request` checks all four.

Read them with `user_id_from_request`, `path_params_from_request`,
`os_identity_from_request`, `is_remote_request`, etc.

---

## ABI of shared values

`ActionOutcome`, `Artifact`, `NavigationSession`, `Notification` and
`Manifest` are `serde`-serializable value types. They travel by value (or inside
JSON `data`), not as trait objects or runtime handles. This is what keeps the
SDK surface stable and avoids passing allocator-owned objects across the
boundary.

Exception: `Arc<dyn Tool>` and `RouteHandler` are trait objects, but they are
constructed inside the plugin and only ever *called* from the host — no data is
freed across the boundary except through the plugin's own code.

---

## `ConfigSnapshot`

Plugins receive a snapshot of core config; they never reconfigure the running
core. Migrations use the host pool; runtime DB/HTTP uses the plugin accessors.
