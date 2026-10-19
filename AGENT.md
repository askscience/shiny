# AGENT.md — orientation for AI agents working on Shiny / PEAK'D!

This file is for AI coding agents. It maps the repository: where the documentation
is, where the code is, how to build and test, and the rules that are easy to break
from a cold start. Read [docs/README.md](docs/README.md) for the human index and
[docs/architecture.md](docs/architecture.md) for the conceptual picture.

## What this is, in one paragraph

Shiny (shipped as **PEAK'D!**) is a local-first desktop assistant/OS shell: a Rust
core process (`src/`, axum + SQLite) that serves a **no-build vanilla ES-module web
UI** (`web/`) over loopback HTTP, plus native kiosk shells that render that UI in a
webview (`crates/peakd` Qt6 on Linux, `crates/peakd-mac` wry/WKWebView at the same
origin, `crates/shiny-server-mode` a small GTK window). **Everything beyond the voice
bar + chat assistant is a plugin** (`plugins/<name>/`): a Rust cdylib loaded at
runtime via `dlopen` that contributes agent tools, HTTP routes, migrations, skills
and an optional web surface.

## Documentation map

| Doc | Read it for |
|---|---|
| [docs/README.md](docs/README.md) | The full index — start here |
| [docs/getting-started.md](docs/getting-started.md) | Prereqs, `cargo run`, first login, installing a plugin |
| [docs/architecture.md](docs/architecture.md) | Process/layers, how the pieces talk |
| [PLUGINS.md](PLUGINS.md) | The plugin system end to end: traits, installer, worked example |
| [docs/plugins/](docs/plugins/) | `authoring.md`, `reference.md`, `migrations.md`, `runtime-abi.md`, `security.md`, `architecture.md`, `troubleshooting.md` |
| [API_DOCS.md](API_DOCS.md) + [docs/api/](docs/api/) | HTTP API conventions and every endpoint group |
| [docs/core/](docs/core/) | Subsystem docs: `web-ui.md`, `i18n.md`, `voice.md`, `agent.md`, `chat.md`, `desktop.md`, `themes-icons.md`, `data-and-auth.md`, `travel.md`, `clipboard.md` |
| [docs/host/](docs/host/) | Audio, network, Bluetooth, battery, power panels |
| [docs/deployment/](docs/deployment/) | Kiosk shell, sidecars, server mode, greeter |
| [docs/reference/](docs/reference/) | `crate-map.md`, `env-vars.md`, `glossary.md` |
| [docs/configuration.md](docs/configuration.md) | Server configuration |
| [ICONSETS_PLAN.md](ICONSETS_PLAN.md), [web/ui/icons/INDEX.md](web/ui/icons/INDEX.md) | Icon names/catalog (never hand-draw SVGs in UI code) |

## Code map — where to edit what

| Path | What lives there |
|---|---|
| `src/main.rs`, `src/api/` | Axum server: routes, middleware, auth (`mod.rs` mounts core + plugin routes and serves `web/` as the fallback) |
| `src/services/` | Core services: agent runner/tools, voice glue, host panels (audio/network/bluetooth/battery/power), image/song generation, whisper |
| `src/plugins/` | Plugin manager, loader, registry/bridge (`registry.rs` adapts plugin tools into the agent's tool registry), installer, admin API, Craft store (`store.rs`: installs the Craft App Suite's web builds as `kind = "web"` plugins), Craft MCP hosts (`craft_mcp.rs`: each open app's CLI as agent tools; focus mode narrows them to the focused apps) |
| `src/db/` | SQLite pool + **core migrations embedded with `include_str!`** (a new file in `migrations/` also needs its line here) |
| `migrations/` | Core schema migrations (numbered; never edit an applied one) |
| `web/index.html` | The single shell page (boot splash, login, HUD skeleton) |
| `web/js/` | UI modules (~90): desktop/tiles, HUD chips + menus, settings window, chat/voice, agent, i18n, prefs |
| `web/ui/` | The shared component library (`index.js` re-exports) + icons/theme loaders |
| `web/css/`, `web/themes/` | Shell styles and theme token/component CSS |
| `web/i18n/` | UI translation catalogs (`en.json` is the reference; `index.json` lists languages) |
| `web/js/tests/` | Node-based frontend tests (plain `node file.mjs`, no framework) |
| `plugins/<name>/` | A plugin: `plugin.toml`, `Cargo.toml`, `src/` (Plugin + Tool impls), `web/` (plugin.js/css/hud.js/i18n), `migrations/`, `skills/` (prompt skills), `docs/`, `web/icon.svg` |
| `crates/shiny-plugin-sdk/` | The public plugin ABI: `Plugin`/`Tool` traits, contexts, routes, crons, migrations |
| `crates/peakd/`, `crates/peakd-mac/`, `crates/shiny-server-mode/` | Native shells (kiosk on Linux / macOS / the server-mode GTK window) |
| `scripts/` | Installers and session scripts (distro setup, sidecars, Touch Bar, T2 hardware) |
| `data/` | Runtime state: `data/traveler.db`, **installed plugin copies** under `data/plugins/<name>/` (gitignored) |
| `voice/`, `greeter/`, `benchmarks/`, `vendor/` | Sidecars/models, greeter assets, benchmark scaffolding (gitignored), patched vendored crates |

## Build, run, test

```bash
cargo run                         # server on http://localhost:8080 (serves web/)
cargo build --release             # server + all workspace members (incl. plugin cdylibs)
cargo build -p peakd              # kiosk shell (Linux; needs Qt6 WebEngine dev packages)
cargo build -p peakd-mac          # macOS shell
cargo build -p shiny-server-mode  # server-mode GTK window

# Tests (same commands CI runs)
scripts/test-web.sh                                    # frontend
scripts/test-rust.sh                                   # explicit plugin fixtures + Rust tests
cargo clippy --workspace --exclude peakd --exclude peakd-mac --all-targets
```

Heavy builds: run them in the background — this session may be running *inside* the
app's Terminal plugin, so a blocking build freezes the user's own terminal.

## Recipes

### Change the UI
The shared **Flow** layer (`web/ui/flow.css`, loaded after theme overrides) owns
ordinary control motion and settings grouping. Reuse the UI factories, including
`controlButton()` for unskinned plugin buttons. Consume `--motion-*` tokens;
dispose finite transitions with their owner and preserve reduced-motion behavior.
Keep every user Appearance option and put visible text in translation catalogs.

1. Edit `web/index.html` / `web/js/**` / `web/css/**` — there is **no bundler and no
   npm step**; the server serves the tree as-is with `Cache-Control: no-store`, so a
   reload picks changes up.
2. Build UI from `web/ui/index.js` components and theme tokens (`.ui-*` classes,
   `--token` variables). Names of icons come from the icon catalog.
3. Any user-visible string goes through `t()`/`tp()` with a key added to
   `web/i18n/en.json` (see [docs/core/i18n.md](docs/core/i18n.md)); strings in
   `index.html` keep their English text and carry a `data-i18n*` key.
4. Run the frontend tests — `web/js/tests/i18n.test.mjs` fails the build when a used
   key is missing from `en.json`.

### Add or change a core endpoint
1. Handler in `src/api/<area>.rs`, route registered in `src/api/mod.rs` (protected
   routes go through the auth middleware; static files are served unauthenticated).
2. Document it in `docs/api/<area>.md` and the [API_DOCS.md](API_DOCS.md) index.
3. Errors: return `AppError` (from `shiny-plugin-sdk`) — the wire shape is
   `{ success, data, error }` with the status mapped centrally.

### Build a plugin
1. Copy the shape of `plugins/hello/` (the reference) or any real plugin; add the
   crate to the workspace members in the root `Cargo.toml`.
2. Implement the `Plugin` trait and one `Tool` per capability
   (`crates/shiny-plugin-sdk/src/{plugin,tools}.rs`); declare routes, migrations and
   skills. `plugin.toml` carries `name`, `version`, `api_level`, `description`,
   `summary`, and the host-side `category`/`capabilities` hints.
3. The web surface (optional) is `web/plugin.js` default-exporting
   `{ name, mount, unmount, getElement, wireEvents, contextMenu }`; it imports core
   modules **relatively** (`../../js/…`, `../../ui/index.js`) so it also works through
   the kiosk's filtering proxy. Styles belong in core CSS (`web/css/tiles.css`); a
   plugin's own CSS is layout only.
4. Build and install:
   ```bash
   cargo build --release --workspace
   mkdir -p /tmp/pkg/myplugin && cp plugins/myplugin/plugin.toml /tmp/pkg/myplugin/
   cp target/release/libshiny_my_plugin.so /tmp/pkg/myplugin/
   # + skills/ migrations/ web/ as needed
   ( cd /tmp/pkg && zip -r myplugin.zip myplugin )
   curl -X POST http://localhost:8080/api/plugins/install \
        -H "Authorization: Bearer $TOKEN" -F "file=@/tmp/pkg/myplugin.zip"
   ```
   Installation is hot — no restart. `PLUGINS.md` §7/§12 has the long form.
5. **Served-copy gotcha:** the app serves the *installed* copy, never
   `plugins/<name>/web/`. Two roots are scanned — the read-only
   `SYSTEM_PLUGINS_DIR` baseline (the repo's `data/plugins`) and the writable
   `PLUGINS_DIR` (`~/.local/share/shiny/plugins` in a kiosk install). A
   same-named plugin in `PLUGINS_DIR` **overrides** the baseline, so when both
   exist the copy that must be refreshed is `PLUGINS_DIR/<name>/web/`; copying
   to `data/plugins` alone leaves the stale window in front of it.
6. Plugin migrations are recorded by filename and never re-run: never edit an applied
   file, add a new one.

### Update docs
Docs are part of the change, not an afterthought: when a feature, setting, env var,
route, tool or module changes, update the matching page under `docs/` (and the
tables in `docs/api/`, `docs/reference/env-vars.md`, `PLUGINS.md` where they apply),
then verify the cross-links still resolve.

## Hard rules and gotchas

- **No build step for the web app.** No npm/bundler/framework; plain ES modules with
  relative imports, served from disk. Do not introduce a build pipeline.
- **CSP forbids inline scripts** (`src/api/mod.rs` sets a strict policy). Pre-paint
  bootstraps live in their own files (`web/js/early-theme.js`, `early-i18n.js`).
  `eval`/inline handlers will not run.
- **Missing static files do not 404** — `ServeDir` falls back to `index.html` with
  HTTP 200, so validate parsed JSON/content, never the status code.
- **Theme + i18n are runtime-swapped** through `<html>` data attributes and CSS
  variables; keep new UI keyed off tokens and translated keys, not literals.
  User-visible strings go through `t()`/`tp()` with a key in `web/i18n/en.json`
  (or the plugin's own `web/i18n/en.json`) — `web/js/tests/i18n.test.mjs` fails
  when a used key is missing. See [docs/core/i18n.md](docs/core/i18n.md).
- **Do not hardcode machine-specific paths or user names** — this tree ships as a
  distro image; per-user state belongs in `data/`, `$XDG_RUNTIME_DIR` or the user's
  config, and code must resolve them at runtime.
- **Never commit secrets**, model weights, or runtime state (`data/`, `benchmarks/`
  are gitignored on purpose).
- **The user runs the live kiosk session.** Build and prepare, but do not restart or
  replace the running session — say what to restart and let them do it.
- Rust: `cargo clippy` clean for the crates CI covers; `migrations/` are append-only;
  the SDK crosses an unstable Rust ABI — freeze shared types and trait layouts for host-only refactors; default methods alone do not ensure binary compatibility.
- Frontend tests are plain Node scripts with their own `check()` helpers; add one
  under `web/js/tests/` (the CI glob picks it up automatically).
- When a task touches the assistant's *status strings*, note the split: UI-facing
  labels are catalogue keys, while text sent to the model (`describe_tool_step`,
  system prompts) must stay English.
