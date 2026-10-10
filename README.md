# Shiny — AI Desktop

**Shiny is a local-first AI assistant you talk to, wrapped in a desktop.** The
core is a voice-first conversational agent (Ollama) inside a browser-based
desktop shell — a fixed HUD, tiled/floating plugin windows across workspaces, a
voice bar, resumable chat and web search. Everything beyond that ships as a
**self-contained plugin**: a folder with `plugin.toml`, a Rust `cdylib`, and
optional migrations, skills and a `web/plugin.js` window.

Plugins are installed at runtime (drop a `.zip`/`.tar.gz` on the API) with hot
router swap: **no core edits, no restart**. With no plugin active, Shiny is just
the voice/chat assistant.

```
Browser (web/)                              shiny (core binary)
┌───────────────────────────┐               ┌───────────────────────────────────────┐
│ desktop shell · HUD        │     HTTP/SSE  │ agent loop (Ollama) · plugin dispatch  │
│ windows · voice bar        │◀────────────▶ │ voice (Whisper/Vosk + Supertonic TTS)  │
│ chat · settings · plugins  │               │ auth · SQLite · host panels · plugins  │
└───────────────────────────┘               └───────────────┬───────────────────────┘
                                                            │ dlopen (cdylib)
                                              Plugin: tools · routes · schema · window
```

---

## Highlights

**Core assistant**
- Ollama-driven agent loop — voice or typed input, spoken + text replies.
- **faster-whisper** streaming STT (local sidecar; Vosk in-browser fallback) and
  **Supertonic 3** TTS (optional **Qwen3-TTS**).
- Resumable conversations, Markdown replies, artifact dock, GNOME-style
  notifications, web search.
- Agent actions for plugins and the desktop (`plugin_*`, `workspace_*`,
  `desktop_*`, `show_plugin`).

**Desktop shell (`web/`)**
- Tiling / columns / floating windows across workspaces, with `Alt`-shortcuts.
- Top HUD: clock + weather, plugin tray grouped by category, host chips
  (sound, network, Bluetooth, battery), workspace tabs, power menu, Settings /
  Plugins / Chats.
- Voice bar gestures (tap / long-press wake / double-tap to type), barge-in, a
  selectable icon library (Infinity coloured set by default, auto light/dark)
  and five bundled themes.

**Host integration (Linux)**
- PipeWire sound panel, NetworkManager Wi-Fi, BlueZ Bluetooth, sysfs battery,
  logind power actions, display scale, screen/keyboard backlight.
- **Optional** and self-hiding when the daemon is absent.

**Kiosk & multi-user**
- Native shells: `peakd` (Qt WebEngine) and `peakd-mac` (WKWebView + Touch Bar).
- Real Linux-user mode (NSS + PAM), per-user servers and the **Shiny greeter** —
  the app's own login screen on the seat, no display manager (LightDM optional).
- Remote access over **Iroh** (P2P) or **Tailscale Funnel** (public HTTPS URL),
  with host controls kept local.

---

## Quick start

See **[Getting started](docs/getting-started.md)** for the full guide.

```bash
git clone <repo>
cd shiny

# Optional: create a .env (see docs/configuration.md).

# TTS sidecar (or set AUTO_START_SUPERTONIC=true):
./voice/start_supertonic.sh
# STT defaults to faster-whisper; the server starts its sidecar automatically.
./voice/start_whisper.sh --install

cargo run
```

Open <http://localhost:8080> (defaults from `SERVER_HOST`/`SERVER_PORT`).
Microphone access needs HTTPS unless you connect over `localhost`.

**Prerequisites:** Rust, system SQLite, Python 3.9+ (`supertonic[serve]` +
`faster-whisper`), and optionally Ollama, GPSD, PipeWire, BlueZ, ffmpeg, CMake
+ `nasm` (the Browser's TLS client).

---

## Plugins

Every plugin is optional and activated **per user**. Activating one registers its
agent tools and skills, mounts its routes, runs its migrations and opens its
window. In the multi-user install plugins are also **installed per user**
(`PLUGINS_DIR` under the account's data dir) over a read-only system baseline
(`SYSTEM_PLUGINS_DIR`); a user plugin of the same name overrides the baseline.

| Plugin | Category | Adds |
|---|---|---|
| `browser` | Web | Ad-filtered web window, child web views, downloads, incognito, news shelf. |
| `files` | System | File browser over the user's home + the shared save/open helpers. |
| `terminal` | System | A real PTY login shell rendered with xterm.js, and a `terminal_exec` agent tool that runs commands in it. |
| `keyboard` | System | Virtual multi-language on-screen keyboard (8 layouts). |
| `hello` | System | Minimal plugin-authoring example (one tool). |
| `updates` | System | System + Ollama updates for every major distro, with a top-bar chip. |
| `traveler` | Travel | 22 trip/GPS/map/navigation/diary/planning/artifact tools. |
| `word` | Office | Word processor storing real `.odt` documents. |
| `calc` | Office | Spreadsheets with live formulas and `.ods` import/export. |
| `impress` | Office | Slide decks with transitions, exported as `.odp`. |
| `pdf` | Office | PDF viewer/editor: render, edit text, annotate, watermark, merge. |
| `mail` | Office | IMAP inbox + SMTP compose with a local cache. |
| `calendar` | Office | Events in a month grid, with start-soon notifications. |
| `calculator` | Office | Basic/scientific math sharing one evaluator with the window. |
| `image` | Media | Layered raster editor (blend modes, filters, painting, selection). |
| `studio` | Media | DAW-grade sequencer + synth; renders to WAV on a self-contained DSP engine. |
| `filmcraft` | Media | FilmCraft video editor in a window (cut, colour, sound, export), plus a headless engine for batch renders. |
| `radio` | Media | Internet radio via Radio Browser. |
| `youtube` | Media | Search and watch videos, with a built-in recommender. |

Each plugin has its own documentation in [`plugins/<name>/docs/`](plugins) (for
example [`plugins/files/docs/README.md`](plugins/files/docs/README.md)).

---

## Documentation

The full documentation lives in **[`docs/`](docs/README.md)**:

| Area | Start here |
|---|---|
| Orientation | [Getting started](docs/getting-started.md) · [Architecture](docs/architecture.md) · [Configuration](docs/configuration.md) |
| Core | [Agent](docs/core/agent.md) · [Chat](docs/core/chat.md) · [Voice](docs/core/voice.md) · [Desktop](docs/core/desktop.md) · [Web UI](docs/core/web-ui.md) · [Themes & icons](docs/core/themes-icons.md) · [Data & auth](docs/core/data-and-auth.md) · [Travel](docs/core/travel.md) |
| Host | [Overview](docs/host/README.md) · [Audio](docs/host/audio.md) · [Bluetooth](docs/host/bluetooth.md) · [Battery & power](docs/host/battery-power.md) · [Network](docs/host/network.md) · [Display & input](docs/host/display-input.md) |
| API | [Index](docs/api/README.md) · [Auth](docs/api/auth.md) · [Chat & agent](docs/api/chat-agent.md) · [Voice](docs/api/voice.md) · [Host](docs/api/host.md) · [Travel](docs/api/travel.md) |
| Deployment | [Overview](docs/deployment/README.md) · [Sidecars](docs/deployment/sidecars.md) · [Multi-user Linux](docs/deployment/multi-user-linux.md) · [Remote access](docs/deployment/remote-access.md) · [Kiosk shell](docs/deployment/kiosk-shell.md) · [T2 Mac](docs/deployment/t2-mac.md) |
| Plugins | [Plugin system](docs/plugins/README.md) · [Authoring](docs/plugins/authoring.md) · [API reference](docs/plugins/reference.md) · [Runtime & ABI](docs/plugins/runtime-abi.md) |
| Reference | [Crate map](docs/reference/crate-map.md) · [Env vars](docs/reference/env-vars.md) · [Glossary](docs/reference/glossary.md) |

---

## Project layout

```
src/                      # core binary: agent, auth, host panels, plugin host
crates/
  shiny-plugin-sdk/       # the plugin API + ODT/ODS/ODP codecs + shared clients
  shiny-filter/ + -core/  # the Browser's in-process ad/tracker filtering engine
  peakd/ · peakd-mac/     # Linux / macOS native shells
  shiny-auth/             # root PAM verify helper
  shiny-iroh-client/ · shiny-iroh-proto/ · shiny-server-mode/
plugins/<name>/           # 19 self-contained plugins, each with docs/
migrations/               # core SQL migrations (001..009)
web/                      # browser UI: ui/ (library), themes/, js/, vendor/
voice/                    # speech sidecars and model tooling
scripts/                  # installers, kiosk scripts, icon curator
greeter/                  # LightDM theme (optional seat)
docs/                     # this documentation
```

See the [crate map](docs/reference/crate-map.md) for every crate.

---

## Tech stack

| Concern | Choice |
|---|---|
| HTTP | [axum](https://crates.io/crates/axum) 0.7, router hot-swapped via `arc-swap` |
| Database | [sqlx](https://crates.io/crates/sqlx) + **system** SQLite |
| HTTP client | `reqwest` (rustls); `wreq` (BoringSSL, Chrome TLS) for the Browser |
| Async | [tokio](https://crates.io/crates/tokio) |
| Plugin loading | `libloading` (dlopen cdylibs) |
| Studio DSP | `fundsp` + `rustysynth` |
| Host (Linux) | `nmrs`, `zbus`, `if-addrs`, `libc` |
| Remote | `iroh` (optional feature) + Tailscale CLI |

---

## Development

```bash
cargo run                          # dev server
cargo build --release --workspace  # server + all plugins
cargo build --features iroh        # include remote access
cargo check                        # compile check
```

Plugin development: [docs/plugins/authoring.md](docs/plugins/authoring.md).
Editing a plugin window? The app serves the **installed** copy at
`data/plugins/<name>/web/`, not `plugins/<name>/web/`.

---

## License

See [LICENSE](LICENSE). Bundled KDE icon artwork is GPL-3.0; see
[`docs/core/themes-icons.md`](docs/core/themes-icons.md).
