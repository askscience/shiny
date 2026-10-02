# Runtime & ABI

A plugin is a native Rust `cdylib` loaded into the server process. This brings
two hard constraints: the **SDK ABI** across `dlopen`, and the fact that a
plugin statically links its **own copies** of Tokio, sqlx/libsqlite3 and
reqwest/hyper.

Source: [`crates/shiny-plugin-sdk/src/rt.rs`](../../crates/shiny-plugin-sdk/src/rt.rs),
[`tools.rs`](../../crates/shiny-plugin-sdk/src/tools.rs),
[`routes.rs`](../../crates/shiny-plugin-sdk/src/routes.rs),
[`services.rs`](../../crates/shiny-plugin-sdk/src/services.rs),
[`db.rs`](../../crates/shiny-plugin-sdk/src/db.rs).

---

## API level

`api_level` is the version of the SDK's public trait surface. The loader accepts
plugins where `api_level ≤ CORE_API_LEVEL` (currently **1**).

- Adding fields/methods **with defaults** bumps `CORE_API_LEVEL` while existing
  plugins keep working.
- Changing a signature or adding a required method without a default bumps a
  major step and **requires a rebuild**.
- Plugins are never silently broken within an `api_level`.

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

`bridge(fut)` queues the future onto a **process-global, single-threaded**
(`current_thread`) runtime and hands the result back over an executor-agnostic
oneshot channel. The host worker never blocks.

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
  stores them in a private extension type).
- `x-shiny-os-user`, `x-shiny-os-home`, `x-shiny-os-uid` — Linux-user identity.
- `x-shiny-remote` — set by the Iroh proxy; means the request is remote.

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
