# Shiny — AI Sphere Desktop

Shiny is a Rust AI assistant with a browser UI and a plugin system. The core is a
**voice-first conversational agent** driven by a local Ollama server — in-browser Vosk
speech recognition, Supertonic text-to-speech, an orb for voice input, typed chat with
resumable conversations, and web search — shown in a **desktop-style web workspace**: a
fixed top HUD above a Hyprland-style desktop where every active plugin runs in its own
window (tiled, in columns, or floating) across multiple workspaces.

Everything beyond that ships as a **self-contained plugin** — a folder with
`plugin.toml`, a Rust `cdylib`, and optional `skills/`, `migrations/` and a
`web/plugin.js` window surface. Plugins are installed at runtime by uploading a `.zip`
or `.tar.gz` archive through the plugin API: no core edits and no restart (the HTTP
router hot-swaps on install/uninstall).

- 14 plugins ship in this repo (11 self-contained window apps, a tool-only demo, a
  chrome-integrated keyboard, and the traveler domain plugin).
- See [`PLUGINS.md`](./PLUGINS.md) for the plugin architecture, trait surface and a
  worked install example.

## What it looks like

The web UI (`web/`) is a desktop-style workspace:

- **Top HUD** — clock + local weather; a plugin icon tray (grouped by category; tap an
  inactive icon to activate the plugin, tap an active one to focus its window); the
  traveler plugin's saved-places menu; workspace tabs; and Settings / Plugins / Chats
  on the right.
- **Plugin-window desktop** — every active plugin with an interface lives in its own
  window (slim title bar with close/deactivate + fullscreen; drag and resize in the
  Windows layout). Layouts: **Master & stack**, **Columns**, or free **Windows**, with
  `Alt`-shortcuts (`Alt+Enter` fullscreen, `Alt+H/L` focus, `Alt+,/.` workspace,
  `Alt+1..9` jump, `Alt+N` / `Alt+Shift+N` add/remove workspace).
- **Bottom chrome** — the AI **orb** (a fluid canvas sphere that takes its palette from
  your accent) above an artifact dock and the compose input.
- With no plugins active Shiny is a bare voice/chat assistant on the desktop shell;
  activating plugins adds their windows and agent tools.

## Features

**Core (always on)**

- Ollama-driven agent loop: voice or typed input, spoken + text replies
- Voice: **Vosk STT** in the browser (per-language models auto-downloaded) and
  **Supertonic 3 TTS** through a local sidecar
- Orb gestures (tap = talk, long-press = wake, double-tap = type) and bubble chat with
  Markdown replies
- Resumable conversations (server-side chat history)
- Core agent actions: `web_search` (the only built-in *domain* tool) plus plugin and
  desktop controls — `plugin_activate` / `plugin_deactivate` / `list_plugins` /
  `show_plugin`, `workspace_create` / `workspace_remove` / `workspace_switch` /
  `workspace_move`, `desktop_focus` / `desktop_fullscreen`
- Hyprland-style desktop window manager (workspaces, master/columns/windows layouts,
  focus, fullscreen), persisted per user
- Multi-user accounts; per-user plugin activation, preferences, workspaces and
  desktop backgrounds
- Plugin manager: runtime install/uninstall/activate of `.zip`/`.tar.gz` plugins with
  hot router swap and an install audit log
- Settings control panel and a Plugins page (install / activate / activity feed)
- Noir + Light themes with a user-selectable accent and gradient; unified UI library
- GNOME-style plugin notifications and destination insight cards

**Traveler plugin — trips, maps, GPS, diary, navigation**
- Trips with start/end and statistics; live GPS position logging (GPSD, mock fallback)
- OpenStreetMap map window; geocoding / reverse geocoding / routing / POI search
- AI-generated Markdown diaries (daily cron at a configurable time)
- Turn-by-turn driving navigator with a minimal HUD

**Other plugins**
- `word` / `calc` / `impress` — documents, spreadsheets and slide decks with real
  OpenDocument import/export
- `pdf` — view and edit PDFs: render pages, extract text, edit text, add
  highlights/notes/links/watermarks, rotate/reorder/delete/merge pages, create
  from HTML+CSS (pdf_oxide)
- `mail` — IMAP inbox + SMTP compose
- `calendar` — events in a month-grid window
- `calculator` — basic and scientific math
- `image` — photographic effects, filters and transforms (Photon)
- `studio` — session-grid music sequencer + synth, rendered to WAV (trem engine)
- `radio` — internet radio via Radio Browser
- `youtube` — search and watch videos
- `keyboard` — virtual multi-language on-screen keyboard (8 layouts)
- `hello` — minimal plugin authoring example (one `hello` tool)

## Quick Start

### Prerequisites

- Rust (edition 2021)
- System SQLite (`libsqlite3`)
- Python 3.9+ with `supertonic[serve]` (TTS sidecar)
- [Ollama](https://ollama.com) — optional; AI features degrade gracefully when absent
- [GPSD](https://gpsd.io) on `localhost:2947` — optional; falls back to mock GPS
- ~500 MB disk for the Supertonic ONNX model + ~50 MB per Vosk STT language

### Run

```bash
git clone <repo>
cd shiny

# Optional configuration lives in a `.env` file in the repo root (see below).
# TTS needs the Supertonic sidecar — start it by hand:
./voice/start_supertonic.sh
# …or let the server spawn it (add to `.env`):
#   AUTO_START_SUPERTONIC=true

cargo run
```

Open `http://localhost:8080` (defaults from `SERVER_HOST:SERVER_PORT`). Microphone
access requires HTTPS unless you connect over `localhost`.

## Voice interaction

| Gesture | Action |
|---|---|
| Tap the sphere | Listen once, reply spoken aloud |
| Long-press the sphere | Wake mode — hold, say “Hey <assistant name>”, keep talking, release to end |
| Double-tap the sphere | Type a message (bubble chat, Markdown replies) |

Speech recognition runs **in the browser** (Vosk, 16 kHz); the model for the chosen
language (~50 MB) is downloaded from the server and cached. Speech synthesis runs on a
local Supertonic sidecar. The language defaults to your browser/system locale and is
changeable in Settings → Voice.

## Plugins

Every plugin is optional and activated **per user** (on the Plugins page or via its
tray icon). Activating one registers its agent tools and skill docs, mounts its REST
routes, runs its migrations, and — where it has a window surface — opens a window.

| Plugin | Category | Agent tools | REST routes (all authed) | Window |
|---|---|---|---|---|
| `hello` | System | `hello` | — | — (demo) |
| `keyboard` | System | — | — | on-screen keyboard (8 layouts) |
| `traveler` | Travel | 22: trips, locations, maps/POI, navigation, diary, planning, artifact cards | *domain REST served by core* (`/api/trips`, `/api/map/*`, …) | map + navigator (mounted by core) |
| `word` | Office | `doc_*` (7) | `/api/documents…` | Word (`.odt`) |
| `calc` | Office | `calc_*` (6) | `/api/spreadsheets…` | Calc (`.ods`) |
| `impress` | Office | `slide_*` (6) | `/api/presentations…` | Impress (`.odp`) |
| `pdf` | Office | `pdf_*` (12) | `/api/pdfs…` | PDF viewer/editor (annotations) |
| `mail` | Office | `mail_status/list/read/send` | `/api/mail/*` | Mail (IMAP + SMTP) |
| `calendar` | Office | `calendar_*` (5) | `/api/calendar/events…` | Calendar |
| `calculator` | Office | `calculator_eval/history/clear_history` | `/api/calculator/*` | Calculator |
| `image` | Media | `image_*` (4) | `/api/images…` | Image editor |
| `radio` | Media | `radio_search/play/stop` | `/api/radio/nowplaying` | Radio |
| `youtube` | Media | `youtube_search/play` | `/api/youtube/search` | YouTube |
| `studio` | Media | `studio_*` + presets/arrangements (12) | `/api/studio/*` | Studio (sequencer) |

Notes: `word` stores real `.odt` bytes; `calc` keeps a JSON cell grid and `impress`
keeps slide data, exchanging real `.ods`/`.odp` files at import/export (codecs live in
the SDK). The traveler plugin adds no routes of its own — the trip/map/diary REST API
and the map window are special-cased in the core binary, so the plugin itself registers
its agent tools and skills.

## Multi-user

Shiny has **no enforced admin role** — every account is a peer (the first registered
user is flagged `is_admin`, but core routing never gates on it). Each user gets:

- **Per-user plugin activation** (`user_plugin_states`) — turning a plugin on or off
  affects only that user.
- **Per-user preferences and state** — appearance, desktop layout, workspaces, windows
  and voice language. “Remember workspace” restores a previous session; otherwise each
  sign-in starts a fresh desktop and a new chat.
- **Per-user files** — backgrounds plus plugin-owned content (documents, spreadsheets,
  presentations, mail accounts, images, calendar events, …) are scoped to the owner.
- **Auth** — accounts are `username` + password; requests authenticate with a Bearer
  token or the `shiny_token` session cookie.

## Configuration

Settings are read from the environment; a `.env` file in the repo root is loaded at
startup (via `dotenvy`). `RUST_LOG` overrides `LOG_LEVEL`.

| Variable | Default | Description |
|---|---|---|
| `SERVER_HOST` | `0.0.0.0` | HTTP bind address |
| `SERVER_PORT` | `8080` | HTTP port |
| `DATABASE_URL` | `sqlite://data/traveler.db` | SQLite database path |
| `OLLAMA_URL` | `http://127.0.0.1:11434` | Ollama API base URL |
| `OLLAMA_MODEL` | `gemma4:31b-cloud` | Default Ollama model |
| `GPSD_HOST` | `127.0.0.1` | GPSD daemon host |
| `GPSD_PORT` | `2947` | GPSD daemon port |
| `DIARY_AUTO_GENERATE` | `true` | Enable the daily diary cron |
| `DIARY_GENERATE_TIME` | `21:00` | Diary generation time (HH:MM) |
| `LOG_LEVEL` | `info` | Log filter (used when `RUST_LOG` is unset) |
| `LOG_FILE` | `data/shiny.log` | Log file (tee’d alongside stdout) |
| `SUPERTONIC_URL` | `http://127.0.0.1:7788` | Supertonic TTS sidecar URL |
| `SUPERTONIC_VOICE` | `M1` | Default TTS voice preset |
| `VOSK_MODELS_DIR` | `data/vosk-models` | Vosk model storage (served to the browser) |
| `AUTO_START_SUPERTONIC` | `false` | Spawn the `supertonic serve` sidecar on startup |
| `WEB_DIR` | `web` | Static web UI directory |
| `PLUGINS_DIR` | `data/plugins` | Installed-plugin directory |
| `BACKGROUNDS_DIR` | `data/backgrounds` | Per-user desktop background files |
| `ADMIN_TOKEN` | — | Optional token (exposed to plugins; not enforced by core) |

## Architecture

```
Browser (web/)                             shiny (core binary)
┌───────────────────────────┐              ┌─────────────────────────────────────┐
│ desktop workspace shell    │              │ agent loop (Ollama) + web_search +   │
│  HUD · windows · orb       │    HTTP      │ plugin/desktop control actions       │
│  compose chat · settings   │◀────────────▶│ voice (Vosk files + Supertonic TTS)  │
│  / plugins pages           │              │ auth · multi-user · preferences      │
└───────────────────────────┘              │ trip/map/diary REST + services        │
                                           │ plugin manager ── hot-swap router     │
                                           └──────────────────┬────────────────────┘
                                                              │ dlopen (cdylib)
                  ┌───────────────────────────────────────────▼─────────────────────┐
                  │ Plugin: plugin.toml + lib<name>.so + skills/ + migrations/ +    │
                  │         web/plugin.js (window) + RouteSpec                      │
                  │   → agent tools · skills · REST routes · windows · migrations   │
                  └─────────────────────────────────────────────────────────────────┘
```

The binary owns the HTTP server, auth, the agent/tool dispatch, voice plumbing, the
desktop shell, the trip/map/diary services, and the plugin manager. Plugins are
discovered in `PLUGINS_DIR` at startup or installed at runtime; each registers agent
tools (with Markdown skill docs), REST routes, database migrations and a front-end
window. The plugin SDK (`crates/shiny-plugin-sdk`) supplies the shared trait surface,
the ODT/ODS/ODP codecs, canonical clients (`OllamaClient`, `SearchService`,
`SupertonicClient`), and the identity plumbing that crosses the `dlopen` boundary.
SQLite is the **system** library — a vendored `libsqlite3-sys` shim is patched in so
every loaded plugin shares a single SQLite instance.

## Project Layout

```
src/                      # core binary
├── main.rs               # bootstrap: config → db → sidecar → plugins → diary cron → serve
├── lib.rs · config.rs · errors.rs
├── auth/mod.rs           # Bearer-or-cookie auth middleware
├── db/mod.rs             # SQLite pool + embedded migrations (001..007)
├── models/               # traveler · trip · location · diary
├── api/                  # router (mod) + auth, travelers, trips, locations, diary, chat,
│                         # search, agent, voice, insights, ollama, artifacts,
│                         # background, preferences
├── services/             # agent_runner / agent_steps / agent_tools · chat_memory ·
│                         # gpsd · osm · diary_gen · navigation · insights/
│                         # (ollama / supertonic / web_search re-export the SDK clients)
└── plugins/              # manager · registry · loader (dlopen) · installer · admin_api

crates/shiny-plugin-sdk/  # plugin API: tools, routes, manifest, migrations, context,
                          # outcome, artifacts, notifications, navigation,
                          # odt/ods/odp codecs, shared services

plugins/<name>/           # one self-contained plugin each (13 total)
voice/                    # Supertonic sidecar launcher, Vosk model downloader, lang map
migrations/               # core schema (001_init .. 007_chat_conversations)
web/                      # browser UI (desktop workspace shell)
```

## Tech Stack

| Component | Crate |
|---|---|
| HTTP | [axum 0.7](https://crates.io/crates/axum) — router hot-swapped via `arc-swap` |
| Database | [sqlx 0.8](https://crates.io/crates/sqlx) + system SQLite (vendored `libsqlite3-sys`) |
| HTTP client | [reqwest 0.12](https://crates.io/crates/reqwest) (rustls) |
| Async runtime | [tokio 1](https://crates.io/crates/tokio) |
| Serialization | [serde](https://crates.io/crates/serde) / [serde_json](https://crates.io/crates/serde_json) |
| Logging | [tracing](https://crates.io/crates/tracing) + `tracing-subscriber` (stdout + file tee) |
| Plugin loading | [libloading](https://crates.io/crates/libloading) (dlopen cdylibs) |
| Archives | [zip](https://crates.io/crates/zip) + [flate2](https://crates.io/crates/flate2) + [tar](https://crates.io/crates/tar) |
| Config | [dotenvy](https://crates.io/crates/dotenvy) (`.env`) |
| Plugin SDK codecs | ODT/ODS/ODP via [roxmltree](https://crates.io/crates/roxmltree) + zip |

## API Overview

All `/api/*` routes except register/login, `GET /api/voice/languages` and the Vosk
model files require a Bearer token or the `shiny_token` cookie. Plugin routes mount
only while their plugin is installed (and are authenticated as well).

**Core**

| Method | Path | Purpose |
|---|---|---|
| POST | `/api/auth/register` · `/api/auth/login` | Create account / sign in (username + password) |
| GET · PUT | `/api/travelers/me` | Profile |
| GET · PUT | `/api/preferences` | Per-user key/value preferences |
| GET · POST · DELETE | `/api/background` | Per-user desktop background |
| GET · POST | `/api/trips` | List / create trips |
| GET | `/api/trips/active` | Active trip |
| GET · PUT | `/api/trips/:id` | Get / update a trip |
| POST | `/api/trips/:id/start` · `/api/trips/:id/end` | Start / end a trip |
| GET | `/api/trips/:id/stats` · `/api/trips/:id/route` | Trip stats / route |
| POST · GET | `/api/locations` | Submit / query GPS points |
| GET | `/api/map/search` · `/api/map/reverse` · `/api/map/route` · `/api/map/poi` | Geocode, route, POIs |
| GET | `/api/navigate/start` | Start turn-by-turn navigation |
| GET | `/api/diary` · `/api/diary/:date` · `/api/diary/search` | List / read / search diaries |
| POST | `/api/diary/generate` | Generate a diary entry (Ollama) |
| POST | `/api/chat` · GET `/api/chat/history` | Legacy one-shot chat |
| GET · POST | `/api/chat/conversations` | List / create conversations |
| GET · DELETE | `/api/chat/conversations/:id` | Read / delete a conversation |
| POST | `/api/search` | Web search (+ AI summary when Ollama is up) |
| POST | `/api/agent` | Agent run (JSON, or SSE when `stream: true`) |
| GET | `/api/ollama/models` | Ollama model list |
| GET | `/api/insights/context` | Destination insight cards |
| GET · POST | `/api/artifacts` · GET · PUT `/api/artifacts/:id` | Saved artifact cards |
| POST | `/api/tts` | Supertonic TTS proxy → `audio/wav` |
| GET | `/api/voice/status` | Vosk + Supertonic readiness |
| POST | `/api/voice/download` | Download a Vosk model |
| GET | `/api/voice/languages` | Supported STT/TTS languages |
| GET | `/api/voice/models/vosk/*` | Serve Vosk model archives (public) |
| GET · POST | `/api/plugins` | List / install plugins |
| GET | `/api/plugins/active` | Plugins active for the session |
| POST | `/api/plugins/uninstall` · `/api/plugins/activate` · `/api/plugins/deactivate` | Manage plugins |
| GET | `/api/plugins/install.log` | Install audit log |

**Plugin routes** — documents (`word`), spreadsheets (`calc`), presentations
(`impress`), `/api/pdfs` (`pdf`), `/api/mail/*` (`mail`), `/api/calendar/events` (`calendar`),
`/api/calculator/*` (`calculator`), `/api/images` (`image`),
`/api/radio/nowplaying` (`radio`), `/api/youtube/search` (`youtube`),
`/api/studio/*` (`studio`).

**Static** — `/settings` and `/plugins` are standalone pages; `/plugins/<name>/*`
serves each plugin’s web assets; everything else falls back to the app (`web/`).

## Voice & Languages

`voice/lang_map.json` drives both sides of the voice stack:

- **TTS** — Supertonic 3 via the sidecar, with voice codes for 32 languages (zh uses
  the English voice).
- **STT** — Vosk small models download on demand: 19 languages ship a native Vosk
  model; the remaining 13 fall back to the English model.

## Graceful Degradation

| Service | If unavailable |
|---|---|
| Ollama | Chat / diary / agent tools error; everything else runs |
| Supertonic | TTS fails; STT still works |
| GPSD | Mock GPS (fixed point + drift) |
| Nominatim / OSRM / Overpass | Map and geo endpoints error |
| DuckDuckGo | Search returns empty results |

## Development

```bash
cargo run                # dev server (RUST_LOG=debug for verbose logs)
cargo build --release    # release build
cargo check              # compile check
```

> [`API_DOCS.md`](./API_DOCS.md) holds older request/response examples; the endpoint
> table above is the current source of truth.
