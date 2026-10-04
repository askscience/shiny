# Architecture

Shiny is a **local-first AI assistant** built as a Rust HTTP server with a
browser-based desktop shell and a runtime-loaded plugin system. The core owns
the agent loop, authentication, the database, the voice stack and a small set of
desktop controls; everything domain-specific ships as a plugin.

This document is the map. Each section links to the detailed document for the
area it describes.

---

## 1. Processes

A running Shiny install is a small constellation of processes:

| Process | Language | Role | Source |
|---|---|---|---|
| `shiny` | Rust (axum) | The server: HTTP + SSE API, auth, agent loop, static web UI, plugin host, host panels. | [`src/main.rs`](../src/main.rs) |
| `peakd` / `peakd-mac` | Rust + Qt WebEngine | The kiosk shell: a native window that renders the web UI, owns child browser web views, reads trackpad gestures, installs the macOS Touch Bar. | [`crates/peakd`](../crates/peakd), [`crates/peakd-mac`](../crates/peakd-mac) |
| `supertonic` sidecar | Python (FastAPI) | Text-to-speech (`supertonic[serve]`). | [`voice/supertonic_server.py`](../voice/supertonic_server.py) |
| `faster-whisper` sidecar | Python (FastAPI) | Streaming speech-to-text. | [`voice/whisper_server.py`](../voice/whisper_server.py) |
| Qwen3-TTS sidecar | native (`qwentts.cpp`) | Optional high-quality TTS. | [`voice/start_qwen_tts.sh`](../voice/start_qwen_tts.sh) |
| `shiny-auth` | Rust (root) | Verify-only PAM helper over a Unix socket (Linux-user mode). | [`crates/shiny-auth`](../crates/shiny-auth) |
| `shiny-server-mode` | Rust | Standalone window that shows the Iroh link + controls in the kiosk. | [`crates/shiny-server-mode`](../crates/shiny-server-mode) |
| `shiny-iroh-client` | Rust | Standalone local proxy to reach a remote server over Iroh. | [`crates/shiny-iroh-client`](../crates/shiny-iroh-client) |

The server, the kiosk and PipeWire all run as the **desktop user**, never as
root. The only privileged components are the optional `shiny-auth` helper and
the systemd/PAM plumbing in [`scripts/`](../scripts).

```
Browser (web/)                              shiny (core binary)
┌───────────────────────────┐               ┌───────────────────────────────────────┐
│ desktop workspace shell    │               │ agent loop (Ollama) + web_search +     │
│  HUD · windows · voice bar │     HTTP      │ desktop/plugin control actions         │
│  compose chat · settings   │◀────────────▶ │ voice (Vosk/Whisper + Supertonic TTS)  │
│  plugins · windows         │               │ auth · multi-user · preferences        │
└───────────────────────────┘               │ trip/map/diary REST + host panels      │
                                            │ plugin manager ── hot-swap router       │
                                            └──────────────────┬────────────────────┘
                                                               │ dlopen (cdylib)
                        ┌──────────────────────────────────────▼────────────────────┐
                        │ Plugin: plugin.toml + lib<name>.so + skills/ + migrations/│
                        │         + web/plugin.js + RouteSpec                       │
                        │   → agent tools · skills · REST routes · windows · schema │
                        └───────────────────────────────────────────────────────────┘
```

---

## 2. Workspace layout

```
shiny/
├── Cargo.toml                     # workspace root + the `shiny` binary crate
├── crates/
│   ├── shiny-plugin-sdk/          # the plugin API (traits, types, codecs, clients)
│   ├── shiny-filter/              # in-process ad/tracker filtering engine (browser)
│   ├── shiny-filter-core/         # shared filter-list parsing/classify logic
│   ├── peakd/                     # Linux kiosk shell (Qt 6 + QtWebEngine)
│   ├── peakd-mac/                 # macOS shell (WKWebView + NSTouchBar)
│   ├── shiny-auth/                # root PAM verify helper
│   ├── shiny-iroh-client/         # local proxy for remote access
│   ├── shiny-iroh-proto/          # shared Iroh protocol types
│   └── shiny-server-mode/         # kiosk server-mode window
├── plugins/                       # 18 bundled plugins, each self-contained
├── src/                           # the core binary
├── migrations/                    # core SQL migrations (001..009)
├── web/                           # the browser UI
│   ├── ui/                        # theme-agnostic component library + icon library
│   ├── themes/                    # theme skins
│   ├── js/                        # app shell modules
│   └── vendor/                    # vendored pdf.js + vosk-browser
├── voice/                         # sidecar launchers, servers and model tooling
├── scripts/                       # installers, kiosk scripts, icon curator
├── greeter/                       # LightDM greeter theme
├── data/                          # runtime state (gitignored): db, plugins, logs
└── docs/                          # this documentation
```

See the [crate map](reference/crate-map.md) for every crate.

---

## 3. Core module map

`src/` is the `shiny` library + binary:

| Path | Responsibility |
|---|---|
| [`main.rs`](../src/main.rs) | Bootstrap: config → log tee → sidecars → DB migrations → services → plugins → diary cron → serve. Also owns the `ArcSwap` router handle. |
| [`config.rs`](../src/config.rs) | `Config::from_env`, the `ConfigSnapshot` handed to plugins. |
| [`lib.rs`](../src/lib.rs) | Re-exports `config`, `errors`, `db`, `models`, `auth`, `services`, `api`, `plugins`. |
| [`db/mod.rs`](../src/db/mod.rs) | Connection options, pool, and the embedded core migration runner. |
| [`auth/mod.rs`](../src/auth/mod.rs) | Bearer-or-cookie auth middleware and the loopback-only session token. |
| [`models/`](../src/models) | `Traveler`, `Trip`, `Location`, `DiaryEntry` row types. |
| [`api/mod.rs`](../src/api/mod.rs) | `AppState`, router construction, plugin route mounting, the remote-host gate. |
| [`api/*.rs`](../src/api) | One module per endpoint area (auth, chat, agent, voice, trips, host panels, …). |
| [`services/`](../src/services) | Long-lived services and business logic (agent, voice, travel, host, ai). |
| [`plugins/`](../src/plugins) | Loader (dlopen), registry, manager, installer, admin API. |

The server keeps a **single `AppState`** (cheaply cloneable) behind the axum
router: the `SqlitePool`, `Config`, every host service, the `PluginManager`, the
voice clients, the Iroh/Tailscale services and the session token live there.

### Live router swap

`main.rs` wraps the axum router in a `RouterHandle` built on
`arc_swap::ArcSwap<Router>`. The handle implements
`tower::Service<IncomingStream>` and injects `ConnectInfo` per connection so
handlers can distinguish loopback from remote. Installing or uninstalling a
plugin rebuilds the router and stores it; in-flight requests keep the old
snapshot, the next request sees the new one. No restart. See
[plugin architecture](plugins/architecture.md).

---

## 4. Request lifecycle

1. The client calls an `/api/*` route (`fetch` or `EventSource`).
2. `tower_http` CORS + a global `Cache-Control: no-store` layer run.
3. Public routes (register/login, languages, Vosk model files) bypass auth.
   Everything else runs [`auth_middleware`](../src/auth/mod.rs), which accepts a
   `Bearer` token or the `shiny_token` cookie.
4. Host-capability routes additionally pass `host_remote_gate`: a **non-GET**
   request carrying an `x-forwarded-for` / remote marker is rejected, so a
   remote client can read the panels but never mutate the machine.
5. The handler runs against `AppState` (usually `State<AppState>` +
   `Extension<AuthUser>`), reads/writes SQLite via `sqlx`, and returns
   `Json<ApiResponse<T>>`.
6. Unknown paths fall through to `ServeDir` over `web/`, so the SPA shell is
   served for any client route.

A plugin's `RouteSpec` routes are mounted the same way, wrapped in auth unless
the spec declares `public`. Path parameters are copied into a header
(`PATH_PARAMS_HEADER`) by [`inject_path_params`](../src/api/mod.rs) so a plugin's
own axum copy can read them across the dlopen boundary.

---

## 5. Agent turn lifecycle

The heart of the app. Full detail in [agent](core/agent.md).

1. The frontend posts to `POST /api/agent` (`stream: true` for SSE).
2. [`agent_runner`](../src/services/agent_runner.rs) builds the **system
   prompt**: `web/skills/core-assistant.md` + active plugins' persona, skills
   markdown and context lines + the user's profile/location/trip/diary context.
3. The conversation history is loaded (or a conversation resumed) and sent to
   Ollama (or an OpenAI-compatible provider when the user configured one).
4. The model may emit JSON action blocks; [`parse_actions`](../crates/shiny-plugin-sdk/src/tools.rs)
   extracts them and [`ToolRegistry::invoke`](../src/plugins/registry.rs)
   dispatches to the owning tool. A plugin's tool is refused if the plugin is
   deactivated for that user.
5. Every result is appended to the step log; the loop continues (bounded) until
   the model returns a plain reply.
6. The reply streams to the client as SSE tokens, is persisted to chat memory,
   and may carry artifacts/notifications/navigation payloads.
7. `POST /api/agent/stop` cancels the in-flight turn via the `TurnRegistry`,
   which is how barge-in and the stop button work.

Built-in core actions are deliberately minimal: `web_search` plus plugin and
desktop controls (`plugin_activate`/`plugin_deactivate`/`list_plugins`/
`show_plugin`, workspace and desktop verbs). Everything else — documents,
images, mail, radio, travel — is a plugin tool.

---

## 6. Plugin system

Full detail in [plugin architecture](plugins/architecture.md) and
[authoring](plugins/authoring.md).

- A plugin is a folder with `plugin.toml`, a Rust `cdylib`, optional
  `migrations/`, `skills/` and `web/`.
- On boot, [`PluginManager::discover_and_install`](../src/plugins/manager.rs)
  walks the read-only `SYSTEM_PLUGINS_DIR` baseline first and then the writable
  `PLUGINS_DIR`, so a user plugin of the same name overrides the baseline. At
  runtime, `POST /api/plugins/install` accepts a `.zip`/`.tar.gz` and writes it
  to `PLUGINS_DIR`.
- The [loader](../src/plugins/loader.rs) validates the manifest (name and
  relative paths), `dlopen`s the library, calls `shiny_plugin_entry`, runs new
  migrations against the core `plugin_schema_versions` table, calls
  `Plugin::register`, then `on_load`.
- Contributions (tools, skills markdown, persona, context lines, routes) are
  held by the manager and become live immediately; the router is rebuilt.
- Activation is **per user** (`user_plugin_states`). Installation is per user
  in the multi-user install (each server has its own `PLUGINS_DIR`); the
  baseline is shared and cannot be uninstalled. Deactivated plugins contribute
  nothing to the prompt and their tools refuse dispatch.

Because plugins are native code in-process, the SDK enforces two rules: wrap
every tool with `bridged(...)` so it runs on a plugin-owned runtime, and access
the database/HTTP only through `ctx.pool()`/`ctx.db()`/`ctx.ollama()`/etc.
See [runtime & ABI](plugins/runtime-abi.md).

---

## 7. Data

SQLite is the **system** library (a vendored `libsqlite3-sys` shim is patched in
so every loaded plugin shares one SQLite). Default database:
`sqlite://data/traveler.db`.

The core owns migrations `001`–`009`: identity (`travelers`), travel
(`trips`, `locations`, `diary_entries`), chat (`chat_messages`,
`chat_conversations`), artifacts, `user_plugin_states` and `user_preferences`,
plus the Linux `unix_*` identity columns. Plugins own their own tables and run
their own migrations. See [data & auth](core/data-and-auth.md) and
[plugin migrations](plugins/migrations.md).

---

## 8. Voice pipeline

See [voice](core/voice.md) and [sidecars](deployment/sidecars.md).

- **STT (default)** — the browser captures audio, gates it locally, and streams
  PCM chunks to `POST /api/voice/stt/chunk`; the server proxies to the
  faster-whisper sidecar, which uses a LocalAgreement-2 policy to return growing
  partials and a final flush.
- **STT (fallback)** — Vosk runs in the browser (WASM, model downloaded from the
  server), trading accuracy for not needing a server-side speech service.
- **TTS** — `POST /api/tts` proxies to Supertonic (default) or Qwen3-TTS. The
  low-power AI mode swaps the engines without touching the user's saved choice.

---

## 9. Host integration pattern

Every host panel (network, audio, Bluetooth, battery, power, display) follows
the same shape: a background service probes the host, caches one snapshot,
broadcasts changes over an SSE route, and exposes mutations that are
**loopback-only**. When the daemon/hardware is absent the panel reports
`available: false` and the chip hides itself. See [host overview](host/README.md)
and the [graceful degradation table](#10-graceful-degradation).

---

## 10. Graceful degradation

| Service | If unavailable |
|---|---|
| Ollama | Chat/diary/agent tools error; the rest runs |
| Supertonic | TTS fails; STT still works |
| faster-whisper | Voice falls back to in-browser Vosk |
| PipeWire / `pactl` | Sound chip hides; app unchanged |
| NetworkManager | Network chip hides; app unchanged |
| BlueZ / `bluetoothd` | Bluetooth chip hides; app unchanged |
| No battery (desktop) | Battery chip hides; app unchanged |
| GPSD | Mock GPS (fixed point + drift) |
| Touch Bar | Feature dormant; `auto` mode never activates off hardware |
| Touchpad gestures | Reader finds no device and stays off |
| Iroh remote access | Not compiled/enabled → no endpoint; local login unchanged |
| `shiny-auth` helper | Login falls back to the local Argon2 hash |
| ffmpeg | Video thumbnails fall back to a generic icon |
| Nominatim / OSRM / Overpass | Map/geo endpoints error |
| DuckDuckGo | Search returns empty results |

---

## 11. Remote access

Two independent paths, both opt-in, documented in
[remote access](deployment/remote-access.md):

- **Iroh** — a peer-to-peer QUIC transport. The server binds an endpoint, shows
  a link with a QR code, and (optionally) an ed25519 device allowlist. A local
  session token authenticates the kiosk without a password and is **never**
  accepted over Iroh/LAN.
- **Tailscale Funnel** — the same app at a public `*.ts.net` HTTPS URL any
  browser can open. `x-forwarded-for` marks the request remote, which triggers
  the same host-mutation gate Iroh uses.

---

## Further reading

- [Configuration](configuration.md) · [Environment variables](reference/env-vars.md)
- [Agent](core/agent.md) · [Voice](core/voice.md) · [Desktop](core/desktop.md)
- [Plugin system](plugins/README.md)
- [API reference](api/README.md)
