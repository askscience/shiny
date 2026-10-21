# Plugin API reference

The public surface is [`crates/shiny-plugin-sdk`](../../crates/shiny-plugin-sdk).
Everything below is `pub` and re-exported from the crate root.

> Runtime rules that apply to all of this are in
> [Runtime & ABI](runtime-abi.md). In short: wrap tools with `bridged(...)`,
> routes with `bridged_route(...)`, and access data only via `PluginCtx`.

---

## `Plugin` trait

`shiny_plugin_sdk::plugin::Plugin` (`src/plugin.rs`):

```rust
#[async_trait]
pub trait Plugin: Send + Sync {
    fn manifest(&self) -> &Manifest;
    fn register(&self, ctx: Arc<PluginCtx>, builder: &mut RegistryBuilder<'_>);

    async fn on_load(&self, _ctx: Arc<PluginCtx>) {}
    async fn on_unload(&self, _ctx: Arc<PluginCtx>) {}
    async fn on_user_registered(&self, _ctx: Arc<PluginCtx>, _user_id: &str) {}
    fn route_handler(&self, _tag: &str) -> Option<RouteHandler> { None }
}
```

| Method | When | Notes |
|---|---|---|
| `manifest` | always | Return a `OnceLock<Manifest>`. |
| `register` | once, after dlopen + migrations | Populate the builder. |
| `on_load` | after registration | Spawn background work with `rt::spawn(...)`. |
| `on_unload` | on uninstall | Flush state. |
| `on_user_registered` | new account provisioned | E.g. create the Files home. |
| `route_handler` | building the router | Resolve a `handler_tag` to a `RouteHandler`. |

`PLUGIN_ENTRY_SYMBOL = "shiny_plugin_entry"`. Export:

```rust
#[no_mangle]
pub extern "C" fn shiny_plugin_entry() -> *mut dyn Plugin {
    Box::into_raw(Box::new(MyPlugin))
}
```

`PluginEntry = unsafe extern "C" fn() -> *mut dyn Plugin`.

---

## `Tool` trait

`shiny_plugin_sdk::tools::Tool`:

```rust
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn aliases(&self) -> &[&str] { &[] }
    fn step_label(&self) -> &str { "Working…" }
    fn humanize(&self, result: &str, data: &Value) -> String { … }
    fn doc_fragment(&self) -> Option<&str> { None }
    async fn invoke(&self, ctx: &PluginCtx, req: ToolRequest<'_>) -> Result<ActionOutcome, AppError>;
}
```

| Method | Purpose |
|---|---|
| `name` | Primary action key; unique across installed tools. |
| `aliases` | Alternate spellings the LLM may emit; registered as pointers to the tool. |
| `step_label` | One-line hint shown while it runs. |
| `humanize` | Note appended to the completed-steps log. Default `"<name> complete"` / `"<name> failed"`. |
| `doc_fragment` | Markdown advertised to the LLM. `None` keeps the tool hidden. |
| `invoke` | Run it. |

### `ToolRequest`

```rust
pub struct ToolRequest<'a> {
    pub user_id: &'a str,       // core users.id
    pub traveler_id: &'a str,   // back-compat mirror of user_id
    pub params: &'a Value,      // raw params JSON from the action block
    pub ctx: &'a AgentContext,  // lat/lon/heading/lang/model
    pub os_home: Option<&'a str> // real Linux home in Linux-user mode
}
```

### `RegistryBuilder`

```rust
pub struct RegistryBuilder<'a> {
    pub tools: Vec<Arc<dyn Tool>>,
    pub routes: Vec<RouteSpec>,
    pub crons: Vec<CronSpec>,
    pub skills_md: String,
    pub persona: String,
    pub context_lines: Vec<String>,
}
```

Builder methods: `.tool(t)`, `.tool_arc(arc)`, `.tools(arcs)` (bridges each
tool onto the plugin runtime), `.route(spec)`, `.routes(specs)`, `.cron(spec)`,
`.skills(md)`, `.persona(p)`, `.context_line(line)`.

> `.skills()` / `.persona()` **replace** the value; `.context_line()` appends.

### `ActionOutcome`

```rust
pub struct ActionOutcome {
    pub action: String,
    pub result: String,           // "ok" | "error"
    pub data: Value,
    pub artifact: Option<Artifact>,
    pub extra_artifacts: Vec<Artifact>,
    pub navigation: Option<NavigationSession>,
}
```

Constructors and builders:

```rust
ActionOutcome::ok("hello", json!({ "reply": "Hi" }))
    .with_artifact(artifact)
    .with_extra_artifacts(vec![a1, a2])
    .with_navigation(session)
    .with_notification(Notification::new("Done").title("Radio").plugin("radio"));
ActionOutcome::error("hello", "Name required");
```

`with_notification` inserts the serialized notification into
`data["notification"]`; the frontend renders it without an ABI change.

### `ParamHelpers` (implemented for `serde_json::Value`)

```rust
req.params.param_str("name")            // Option<String>
req.params.param_f64("lat")             // Option<f64>
req.params.param_u32("limit")           // Option<u32>
req.params.param_bool("all")            // Option<bool>
req.params.require_str("name")?         // Result<String, AppError>
req.params.require_f64("lat")?
```

### Parsing helpers

| Function | Purpose |
|---|---|
| `parse_actions(text) -> Vec<(String, Value)>` | Strip code fences and extract balanced `{…}` blocks with an `action`/`tool` key. |
| `strip_action_blocks(text) -> String` | Remove action JSON so only the spoken reply remains. |
| `normalize_action_name(&str) -> String` | Lowercase + alias map (e.g. `navigate`/`directions`/`go_to` → `navigate_to`). |

### The runtime bridge

| Item | Purpose |
|---|---|
| `BridgedTool(Arc<dyn Tool>)` | Adapter running `invoke` on the plugin runtime. |
| `bridged(Arc<dyn Tool>) -> Arc<dyn Tool>` | Wrap a tool at registration. |

---

## `Manifest` (`plugin.toml`)

| Field | Type | Required | Notes |
|---|---|---|---|
| `name` | string | yes | Unique; install directory name. |
| `version` | semver | yes | Shown in `/api/plugins`. |
| `api_level` | u32 | yes | Rejected if `> CORE_API_LEVEL`. |
| `entry_symbol` | string | native only | Usually `shiny_plugin_entry`; a `kind = "web"` plugin omits it. |
| `kind` | string | no | `"native"` (default) or `"web"` — a static web app: `web/index.html` is served at `/plugins/<name>/` and the shell mounts it in a window as an iframe. Host-side key, like `category`; a web app ships no cdylib and contributes no tools/routes. |
| `target_triple` | string | no | Installer refuses mismatches. |
| `description` | string | no | Short human title. |
| `author` | string | no | Attribution. |
| `summary` | string | no | "What this plugin adds". |
| `category` | string | no | Tray grouping (`Office`, `Media`, `Travel`, `System`, …). |
| `capabilities` | string[] | no | Declared capabilities. `"open-urls"` makes the plugin a link-handler candidate (the `browser` plugin is the default). Host-side key, like `category` — not part of the SDK `Manifest`. |
| `migrations_dir` | string | no | Default `migrations`. |
| `skills_dir` | string | no | Default `skills`. |
| `web_dir` | string | no | Default `web`. |
| `signature` | hex string | no | Reserved ed25519 signature. |

`CORE_API_LEVEL = 1` (`crates/shiny-plugin-sdk/src/lib.rs`).

Native plugins construct the manifest with the shared defaults:

```rust
Manifest::new("documents")
    .description("Document editor")
    .summary("Documents: create and edit")
```

`Manifest::new` fills the version, API level, entry symbol, author and the
standard directories.

---

## Routes

```rust
pub enum HttpMethod { Get, Post, Put, Delete, Patch }

pub struct RouteSpec {
    pub method: HttpMethod,
    pub path: String,        // e.g. "/api/documents/:id"
    pub auth: String,        // "auth" | "public" | "admin" (admin treated as auth)
    pub handler_tag: String, // resolved by Plugin::route_handler
}

pub type RouteHandler = Arc<
    dyn Fn(axum::extract::Request) -> Pin<Box<dyn Future<Output = Response> + Send>> + Send + Sync
>;

pub fn bridged_route<F, Fut>(f: F) -> RouteHandler
where F: Fn(axum::extract::Request) -> Fut + Send + Sync + 'static,
      Fut: Future<Output = Result<Response, AppError>> + Send + 'static;
```

Build authenticated specs with the constructors instead of the struct literal:

```rust
RouteSpec::get("/api/documents", "doc_list")
RouteSpec::post("/api/documents", "doc_create")
RouteSpec::delete("/api/documents/:id", "doc_delete").public()
```

`RouteSpec::new(method, path, tag)` remains available for a dynamic method.
`.public()` and `.admin()` override the default `"auth"`.

Always build handlers with `bridged_route`.

### Route helpers

The SDK also carries the small helpers every route module used to duplicate:

| Helper | Purpose |
|---|---|
| `routes::ok(data)` | `{ "success": true, "data": … }` response. |
| `routes::user_id(req)` | Authenticated user id, or the shared 401. |
| `util::clean_title(t)` | Trim and cap a user title at 120 characters. |
| `util::filename_for_title(t, ext, fallback)` | Filesystem-safe stem + extension. |
| `util::as_text(v)` / `util::as_opt_text(v)` | Text from an SDK DB value; the `opt` form maps NULL/empty to `None`. |
| `util::sanitize_user(id)` | A user id safe as one path segment. |

### Entry-point boilerplate

`ctx_plugin!(MyPlugin)` generates the `ctx: OnceLock<Arc<PluginCtx>>` struct,
`install_ctx`/`routes_for` plumbing, `Default`, and the `shiny_plugin_entry`
symbol. The plugin still writes its own `manifest()` and `register()`.

### Identity & path params

Because each cdylib has its own axum `TypeId`s, core passes identity and path
params as **headers**:

| Constant | Header | Read with |
|---|---|---|
| `USER_ID_HEADER` | `x-shiny-user-id` | `user_id_from_request(req)` |
| `TRAVELER_ID_HEADER` | `x-shiny-traveler-id` | `traveler_id_from_request(req)` |
| `PATH_PARAMS_HEADER` | `x-shiny-path-params` | `path_params_from_request(req)` |
| `REMOTE_HEADER` | `x-shiny-remote` (plus `x-forwarded-*`) | `is_remote_request(req)` |
| `OS_USER_HEADER` | `x-shiny-os-user` | `os_identity_from_request(req)` |
| `OS_HOME_HEADER` | `x-shiny-os-home` | `os_home_from_request(req)` |
| `OS_UID_HEADER` | `x-shiny-os-uid` | `os_identity_from_request(req)` |

`UserId` / `TravelerId` newtypes and `OsIdentity { user, home, uid }` are also
provided.

---

## Crons

```rust
pub struct CronSpec {
    pub tag: String,   // resolved by the plugin's cron_handler
    pub at: String,    // "HH:MM" local, hourly resolution
}
pub type CronEntry = Pin<Box<dyn Future<Output = ()> + Send>>;
```

> Cron **specs** can be declared, but the per-plugin scheduler that fires them
> is still roadmap (see [Architecture](architecture.md) and
> [PLUGINS.md §20](../../PLUGINS.md)). Do not rely on `at` firing yet.

---

## `Notification`

GNOME-style banner. Build with:

```rust
Notification::new("Now playing: Radio Bruno")
    .title("Radio")        // short summary
    .urgency("normal")     // "low" | "normal" | "critical"
    .icon("ui/mail")       // theme icon path
    .plugin("radio")       // plugin icon (mutually exclusive with icon)
    .app("Mail")           // app label
    .action("Stop", "stop")
    .timeout(0);           // ms; 0 = sticky
```

Fields mirror those methods plus `actions: Vec<NotificationAction { label, action }>`.
Attach with `ActionOutcome::with_notification`. From the web surface use
`notify(options)` from the UI library.

---

## `Artifact`

```rust
pub struct Artifact {
    pub id: String,
    pub artifact_type: String,          // serde rename "type"
    pub title: String,
    pub subtitle: Option<String>,
    pub coordinates: Option<Coordinates>,      // { lat, lon }
    pub sections: Vec<ArtifactSection>,        // { label, value }
    pub actions: Vec<ArtifactAction>,          // { label, tool, params }
    pub days: Vec<PlanDay>,                    // { day, title, items[] }
    pub route: Option<RouteMeta>,              // { distance_km, duration_min }
    pub geometry: Vec<[f64; 2]>,
    pub narrative: Option<String>,
    pub theme: Option<String>,
    pub destination: Option<String>,
}
```

`build_from_params(&Value) -> Artifact` builds one from a `show_artifact` params
object, defaulting `type` to `site_info` and `title` to `Place`.

---

## `NavigationSession`

```rust
pub struct NavigationSession {
    pub destination: String,
    pub to_lat: f64,
    pub to_lon: f64,
    pub geometry: Vec<[f64; 2]>,
    pub steps: Vec<RouteStepDto>,   // { distance, duration, instruction }
    pub distance_km: f64,
    pub duration_min: f64,
    pub profile: String,
}
```

Attach with `ActionOutcome::with_navigation`; the core surfaces it to the
navigator UI.

---

## `AgentContext`

```rust
pub struct AgentContext {
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub heading: Option<f64>,
    pub lang: String,
    pub ollama_model: Option<String>,
    pub workspaces_enabled: bool,   // false on a vertical phone-like screen
    pub focused: Vec<String>,       // focus mode: only web_search + these plugins run
}
```

---

## `PluginCtx` & `ConfigSnapshot`

`PluginCtx` carries the plugin's `Manifest`, a `ConfigSnapshot`, and **lazily
opened, plugin-owned** clients:

| Accessor | Returns |
|---|---|
| `ctx.pool().await` | `&SqlitePool` (async, plugin-owned). |
| `ctx.db()` | `&Db` (synchronous; preferred). |
| `ctx.ollama().await` | `&OllamaClient`. |
| `ctx.search().await` | `&SearchService`. |
| `ctx.supertonic().await` | `&SupertonicClient`. |
| `ctx.config` | `ConfigSnapshot`. |
| `ctx.manifest` | `Manifest`. |

`PluginCtx::with_manifest(manifest)` clones the ctx with a fresh manifest and
fresh lazy cells (the loader uses this per plugin).

`ConfigSnapshot`: `server_host`, `server_port`, `database_url`, `ollama_url`,
`ollama_model`, `supertonic_url`, `supertonic_voice`, `web_dir`,
`vosk_models_dir`, `auto_start_supertonic`, `log_level`, `plugins_dir`,
`admin_token`.

Canonical clients re-exported from the crate root: `OllamaClient`
(`chat`, `generate`, `list_models`, `is_available`), `SearchService`
(`search`, `search_html`), `SupertonicClient` (`is_available`, `synthesize`).

---

## Synchronous DB (`Db`)

`shiny_plugin_sdk::db`:

```rust
pub enum Value { Null, Int(i64), Text(String), Blob(Vec<u8>) }

impl Db {
    pub fn open(url: &str) -> Result<Db, AppError>;
    pub fn execute(&self, sql: &str, params: &[Value]) -> Result<usize, AppError>;
    pub fn query(&self, sql: &str, params: &[Value]) -> Result<Vec<Vec<Value>>, AppError>;
}
```

Use `ctx.db()` for plugin data access rather than `ctx.pool()` — see
[Runtime & ABI](runtime-abi.md).

---

## Runtime (`rt`)

| Function | Purpose |
|---|---|
| `bridge(fut) -> Result<T, AppError>` | Run a future on the plugin's single-threaded runtime; await from any executor. |
| `spawn(fut)` | Fire-and-forget a future on the plugin runtime (use in `on_load`; a bare `tokio::spawn` panics on the host executor). |

Both isolate panics so one bad job cannot wedge the worker.

---

## Admin endpoints

| Method | Path | Auth | Purpose |
|---|---|---|---|
| `GET` | `/api/plugins` | Bearer (any user) | Installed plugins + per-user `enabled`. |
| `GET` | `/api/plugins/active` | Bearer | Plugins active for the session. |
| `POST` | `/api/plugins/install` | Bearer | Multipart `file=@archive` (limit 64 MB). |
| `POST` | `/api/plugins/uninstall` | Bearer | `{ "name": "..." }`. |
| `POST` | `/api/plugins/activate` | Bearer | `{ "name": "..." }`. |
| `POST` | `/api/plugins/deactivate` | Bearer | `{ "name": "..." }` — turns the plugin off, closes its window and stops any process it kept alive (a Craft app's MCP server, gracefully). |
| `POST` | `/api/plugins/window/closed` | Bearer | `{ "name": "..." }` — the shell closed a plugin window: stop its process now (graceful MCP lifecycle). Does **not** change `enabled`. |
| `POST` | `/api/plugins/session/reset` | Bearer | Fresh-session boot (`session.remember` off): drop this user's session activations and stop every Craft MCP server. |
| `GET` | `/api/plugins/install.log` | Bearer | Last 200 lines of the install log. |
| `GET` | `/api/plugins/store` | Bearer | Craft App Suite catalog: installed/latest versions, install state and progress, and per app `mcp_enabled` (the AI-control decision), `mcp_installed`, `mcp_pending`. |
| `POST` | `/api/plugins/store/refresh` | Bearer | Re-check every app's latest release (cached, ETag). |
| `POST` | `/api/plugins/store/install` | Bearer | `{ "id": "photocraft" }` — download, verify (SHA-256) and install one app's web build **and its command-line tool (AI control)**, in the background. |
| `POST` | `/api/plugins/store/mcp` | Bearer | `{ "id": "photocraft", "enable": true }` — the AI-control switch: records the decision and installs/removes the app's CLI (its MCP server). On by default; off is remembered (`.craft-mcp/<id>/disabled`), so installs and updates respect it. |
| `GET` | `/api/plugins/craft/control/events` | Bearer | SSE, the page's side of a live Craft app link (`?app=<id>`): each message is one control request the page runs through `window.<app>.request`. |
| `POST` | `/api/plugins/craft/control/reply` | Bearer | `{ "app": "vectorcraft", "id": 42, "ok": true, "result": … }` — the page's answer to one forwarded request. |

There is **no admin role** — any logged-in user can install/uninstall; see
[Security](security.md).
