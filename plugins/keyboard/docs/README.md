# keyboard — virtual keyboard

> Part of the Shiny plugin set · [Plugins overview](../../../docs/plugins/README.md)

`keyboard` is a **pure surface plugin**: a virtual multi-language keyboard bar
fixed to the bottom of the screen that types into whatever editable element
currently has focus. The Rust half is intentionally empty; the UI is shipped as
**core chrome** (`web/js/keyboard.js`) rather than a plugin `plugin.js`.

| | |
|---|---|
| **Category** | `System` |
| **Version** | `0.1.0` |
| **API level** | `1` |
| **Author** | `shiny` |
| **Rust crate** | `shiny-keyboard-plugin` → `libshiny_keyboard_plugin.so` |
| **Agent tools** | **none** — the AI never sees this plugin |
| **REST routes** | none |
| **Window surface** | none (`web/` ships only `icon.svg`; `plugin.js` is absent → `surface: false`) |
| **Frontend** | core chrome: `web/js/keyboard.js` + `web/css/keyboard.css` |
| **Database tables** | none |
| **Runtime deps** | `shiny-plugin-sdk`, `async-trait`, `semver` |

## Manifest

`plugins/keyboard/plugin.toml` is minimal — it declares no `skills_dir`,
`web_dir` or `migrations_dir`, so only the SDK defaults (`skills`, `web`,
`migrations`) would apply. The runtime `Manifest` in `src/plugin.rs:16` still
populates all three defaults, matching every other plugin.

| Field | Value |
|---|---|
| `name` | `keyboard` |
| `version` | `0.1.0` |
| `api_level` | `1` |
| `entry_symbol` | `shiny_plugin_entry` |
| `description` | `Virtual multi-language keyboard at the bottom of the screen — types into any focused input` |
| `summary` | `Virtual keyboard: types into any text input (8 language layouts)` |
| `author` | `shiny` |
| `category` | `System` |
| `migrations_dir` / `skills_dir` / `web_dir` | *(absent in TOML; runtime defaults to `migrations`/`skills`/`web`)* |

## What it adds

`Plugin::register` is a deliberate no-op (`plugins/keyboard/src/plugin.rs:36`):

```rust
/// The keyboard is a pure UI surface — deliberately no persona, no skills
/// and no tools. The AI must never see it.
fn register(&self, _ctx: Arc<PluginCtx>, _builder: &mut RegistryBuilder<'_>) {}
```

| Contribution | Value |
|---|---|
| Persona | none |
| Skills markdown | none |
| Context line | none |
| Tools | none |
| Routes | none |
| Crons | none |

Activation is therefore only observed by the frontend: `GET /api/plugins/active`
includes `keyboard`, and `web/js/keyboard.js` mounts the bar. The plugin's only
shipped asset is `plugins/keyboard/web/icon.svg`.

## How the frontend is wired

Unlike `radio`/`youtube`/`terminal`, there is no `plugins/keyboard/web/plugin.js`
for `tiles.js` to dynamically import. Core owns the keyboard:

1. `web/js/keyboard.js` exports `initKeyboard()`, called during app startup.
2. `refreshKeyboard()` (`web/js/keyboard.js:660`) asks
   `refreshActivePlugins()` whether `keyboard` is active and mounts/unmounts the
   bar accordingly.
3. `refreshKeyboard()` is also called on the `plugins:changed` window event, so
   activating/deactivating the plugin (via the Plugins page, the HUD tray, or an
   agent `activate_plugin` action) shows/hides the keyboard live.
4. The `pluginIcon.js` mapping renders `keyboard → apps/keyboard` for the
   launcher/HUD, falling back to `web/icon.svg`.

The full surface contract — DOM, CSS classes, events, HUD toggle, layouts,
input-target binding and persistence — is documented in
**[window.md](./window.md)**.

## Source map

| File | Purpose |
|---|---|
| `plugins/keyboard/plugin.toml` | Installer manifest (category `System`) |
| `plugins/keyboard/Cargo.toml` | `cdylib` + `rlib` crate `shiny-keyboard-plugin` |
| `plugins/keyboard/src/lib.rs` | Module root + doc comment stating it contributes no agent surface |
| `plugins/keyboard/src/plugin.rs` | `Plugin` impl (empty `register`), C entry symbol |
| `plugins/keyboard/web/icon.svg` | Plugin icon |
| `web/js/keyboard.js` | **Core** frontend: layout tables, DOM, events, HUD toggle, public API |
| `web/css/keyboard.css` | **Core** styling for `#keyboard-bar` and keys |
| `web/js/touchbar.js` | Touch Bar `Keys` button → `toggleKeyboard()` |
| `web/js/pluginIcon.js` | `keyboard → apps/keyboard` icon mapping |

## Build, install, dev notes

```bash
cargo build --release -p shiny-keyboard-plugin
# → target/release/libshiny_keyboard_plugin.so
```

Package `plugin.toml` + the cdylib (+ `web/icon.svg`) and POST to
`/api/plugins/install` exactly as for any plugin. Because the UI is core chrome,
**there is no `plugin.js` to edit inside the plugin**; changes to the keyboard
behaviour go in `web/js/keyboard.js`. Adding a language is a one-line-layout
addition to `LAYOUTS`; adding a key means touching `render()`/`pressKey()`.

- The keyboard suppresses the native OS keyboard on touch devices by setting
  `inputmode="none"` (and `readOnly` on coarse pointers) on every input.
- It must never leak into the agent: keep `register()` empty and do not add a
  `skills`/`persona` contribution.

## Tools summary

None.

## Routes summary

None.
