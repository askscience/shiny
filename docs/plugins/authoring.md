# Authoring a plugin

This is a complete, runnable walkthrough. The reference implementation is
[`plugins/hello/`](../../plugins/hello); read it alongside this page.

> Before writing DB or HTTP code, read [Runtime & ABI](runtime-abi.md). The two
> rules there are not optional.

---

## 1. Cargo setup

`plugins/myplugin/Cargo.toml`:

```toml
[package]
name = "shiny-myplugin-plugin"
version = "0.1.0"
edition = "2021"

[lib]
name = "shiny_myplugin_plugin"     # → libshiny_myplugin_plugin.so
crate-type = ["cdylib", "rlib"]

[dependencies]
shiny-plugin-sdk = { path = "../../crates/shiny-plugin-sdk" }
async-trait = "0.1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
semver = { version = "1", features = ["serde"] }
```

Add the crate to the workspace `members` in the root
[`Cargo.toml`](../../Cargo.toml), or build it standalone.

---

## 2. The manifest

`plugins/myplugin/plugin.toml`:

```toml
name = "myplugin"
version = "0.1.0"
api_level = 1
entry_symbol = "shiny_plugin_entry"
description = "What this plugin does"
summary = "One line for the admin UI"
author = "you"
migrations_dir = "migrations"   # optional; default "migrations"
skills_dir = "skills"           # optional; default "skills"
web_dir = "web"                 # optional; default "web"
category = "Office"             # optional tray grouping
```

See the [manifest reference](reference.md#manifest-plugin-toml) for every field.

---

## 3. The plugin

`plugins/myplugin/src/plugin.rs`:

```rust
use std::sync::Arc;
use async_trait::async_trait;
use shiny_plugin_sdk::{
    manifest::Manifest,
    plugin::{Plugin, PLUGIN_ENTRY_SYMBOL},
    services::PluginCtx,
    tools::{bridged, RegistryBuilder},
};

pub struct MyPlugin;

#[async_trait]
impl Plugin for MyPlugin {
    fn manifest(&self) -> &Manifest {
        static M: std::sync::OnceLock<Manifest> = std::sync::OnceLock::new();
        M.get_or_init(|| Manifest {
            name: "myplugin".into(),
            version: semver::Version::new(0, 1, 0),
            api_level: 1,
            entry_symbol: PLUGIN_ENTRY_SYMBOL.into(),
            target_triple: None,
            description: Some("What this plugin does".into()),
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
            .persona("a helpful assistant with a greeting tool")
            .skills("## My tools\n- `hello` — say hello. params: `{ name?: string }`")
            .tool_arc(bridged(Arc::new(crate::tool::HelloTool)));
    }

    // Optional hooks:
    // async fn on_load(&self, _ctx: Arc<PluginCtx>) {}
    // async fn on_unload(&self, _ctx: Arc<PluginCtx>) {}
    // async fn on_user_registered(&self, _ctx: Arc<PluginCtx>, _user_id: &str) {}
}

#[no_mangle]
pub extern "C" fn shiny_plugin_entry() -> *mut dyn Plugin {
    Box::into_raw(Box::new(MyPlugin))
}
```

Note `bridged(...)` around every tool. See
[Runtime & ABI](runtime-abi.md).

---

## 4. A tool

`plugins/myplugin/src/tool.rs`:

```rust
use async_trait::async_trait;
use serde_json::{json, Value};
use shiny_plugin_sdk::{
    errors::AppError,
    outcome::ActionOutcome,
    services::PluginCtx,
    tools::{ParamHelpers, Tool, ToolRequest},
};

pub struct HelloTool;

#[async_trait]
impl Tool for HelloTool {
    fn name(&self) -> &str { "hello" }
    fn aliases(&self) -> &[&str] { &["greet"] }
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

A tool that returns `doc_fragment() == None` stays hidden from the LLM — useful
for internal/helper tools.

---

## 5. Migrations, skills, web

- **Migrations** (`migrations/00X_*.sql`) run once, host-side, at load; see
  [Migrations](migrations.md) for the immutability rule.
- **Skills** (`skills/<name>.md`) is what the model reads about your tools; you
  can also set it inline with `builder.skills(...)`.
- **Web** (`web/plugin.js`, `web/icon.svg`) is the optional window surface; see
  [Window surface](#7-window-surface).

---

## 6. Build, package, install

```bash
# Build the workspace (binary and every plugin cdylib).
cargo build --release --workspace

# Package the plugin.
mkdir -p /tmp/pkg/myplugin/skills /tmp/pkg/myplugin/migrations
cp plugins/myplugin/plugin.toml            /tmp/pkg/myplugin/
cp plugins/myplugin/skills/myplugin.md     /tmp/pkg/myplugin/skills/
cp plugins/myplugin/migrations/*.sql       /tmp/pkg/myplugin/migrations/ 2>/dev/null || true
cp target/release/libshiny_myplugin_plugin.so /tmp/pkg/myplugin/
( cd /tmp/pkg && zip -r myplugin.zip myplugin )

# Install on a running server (any logged-in user's bearer token).
TOKEN=...   # from POST /api/auth/login
curl -X POST http://localhost:8080/api/plugins/install \
  -H "Authorization: Bearer $TOKEN" \
  -F "file=@/tmp/pkg/myplugin.zip"
# {"success":true,"data":{"installed":"myplugin"}}

# Verify.
curl -H "Authorization: Bearer $TOKEN" http://localhost:8080/api/plugins
```

The agent sees the tool on its next `/api/agent` call. No restart.

### How the agent calls it

```json
{"action":"hello","params":{"name":"Alice"}}
```

`ToolRegistry::invoke` runs `HelloTool::invoke`, returning:

```json
{"action":"hello","result":"ok","data":{"who":"Alice","reply":"Hello, Alice!"}}
```

The runner records it in `actions_taken`; the LLM writes the final reply.

---

## 7. Window surface

A plugin with a UI ships `web/plugin.js` whose default export core imports. The
[`word`](../../plugins/word/docs/window.md) plugin is the reference:

```js
export default {
  name: 'myplugin',
  mount: mountTile,      // returns a section.tile element (or null)
  unmount: unmountTile,
  getElement: getTileElement,
  wireEvents: wireEvents,
  contextMenu: contextMenu,   // optional right-click entries
};
```

Rules:

- `mount()` returns a `section.tile <name>-tile` element with
  `data-plugin="<name>"`. Core tiles it, applies focus/fullscreen, and calls
  `unmount()` on deactivation.
- **Plugins never ship CSS.** Build with the core UI library
  (`import { button, icon, notify } from '../../../ui/index.js'`) and core's
  `web/css/tiles.css` keyed by your class prefix.
- **Icons**: use names from [`web/ui/icons/INDEX.md`](../../web/ui/icons/INDEX.md);
  never hand-draw SVG inside the window.
- **Identity icons** resolve through the active icon set (`apps/<name>` mapped
  in `web/js/pluginIcon.js`). Ship `web/icon.svg` only if your plugin is not
  mapped — bundled plugins no longer need to carry one.
- **A window must not cache server-owned addresses** (random ports, signed
  URLs): return the address with the payload and re-read it.
- Ship `web/icon.svg` for the tray/launcher, or map to an existing
  `apps/<name>` icon in `web/js/pluginIcon.js`.

Development gotcha: the app serves the **installed** copy at
`data/plugins/<name>/web/`, not `plugins/<name>/web/`. Copy (or reinstall) after
editing.

### Top-bar chips (`web/hud.js`)

A window surface is loaded only while the plugin is *active*. A plugin that
needs a **persistent** top-bar presence (a status badge, an always-visible
button) ships `web/hud.js` instead — core loads it for the plugin simply being
*installed*, active or not:

```js
// plugins/myplugin/web/hud.js
export function install({ plugin } = {}) {
  // create your DOM (append to #hud-top, etc.)
}
export function uninstall() {
  // remove it
}
export default { install, uninstall };
```

Core calls `install()` once per page load and `uninstall()` when the plugin is
uninstalled. The plugin owns its DOM; reuse core classes (`icon-btn` etc.) and
the UI library rather than shipping CSS. `GET /api/plugins` exposes a `hud`
boolean when the file exists.

---

## 8. Authoring checklist

- [ ] `plugin.toml` parses to `Manifest`; `api_level` matches the SDK.
- [ ] `shiny_plugin_entry` is `#[no_mangle] pub extern "C"`.
- [ ] Every advertised tool has `name`, `step_label`, `doc_fragment`, `invoke`.
- [ ] Every tool is registered via `builder.tool_arc(bridged(tool))`.
- [ ] All DB/HTTP work uses `ctx.pool()`/`ctx.db()`/`ctx.ollama()`/`ctx.search()`/`ctx.supertonic()`.
- [ ] Migrations are idempotent and **never edited after being applied**.
- [ ] `skills_md`/`persona` are set if the plugin contributes them.
- [ ] `web/icon.svg` follows the plugin icon style, or an `apps/<name>` mapping exists.
- [ ] Window surfaces have a smoke test (`node --check web/plugin.js`; see
      `plugins/browser/web/plugin.smoke.mjs`).
- [ ] `cargo check` passes; install via `/api/plugins/install` and `GET /api/plugins` returns it.

---

Next: [API reference](reference.md) · [Runtime & ABI](runtime-abi.md) ·
[Migrations](migrations.md) · [Troubleshooting](troubleshooting.md).
