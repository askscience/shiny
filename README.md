# Shiny — AI Sphere Desktop

Shiny is a Rust AI assistant with a browser UI and a plugin system. The core is a
**voice-first conversational agent** driven by a local Ollama server — streaming
faster-whisper speech recognition (or in-browser Vosk), Supertonic text-to-speech, an orb
for voice input, typed chat with resumable conversations, and web search — shown in a
**desktop-style web workspace**: a fixed top HUD above a Hyprland-style desktop where
every active plugin runs in its own window (tiled, in columns, or floating) across
multiple workspaces.

Everything beyond that ships as a **self-contained plugin** — a folder with
`plugin.toml`, a Rust `cdylib`, and optional `skills/`, `migrations/` and a
`web/plugin.js` window surface. Plugins are installed at runtime by uploading a `.zip`
or `.tar.gz` archive through the plugin API: no core edits and no restart (the HTTP
router hot-swaps on install/uninstall).

- 15 plugins ship in this repo (12 self-contained window apps, a tool-only demo, a
  chrome-integrated keyboard, and the traveler domain plugin).
- See [`PLUGINS.md`](./PLUGINS.md) for the plugin architecture, trait surface and a
  worked install example.

## What it looks like

The web UI (`web/`) is a desktop-style workspace:

- **Top HUD** — clock + local weather; a plugin icon tray (grouped by category; tap an
  inactive icon to activate the plugin, tap an active one to focus its window); the
  traveler plugin's saved-places menu; host sound and network chips; workspace tabs;
  and Settings / Plugins / Chats on the right.
- **Plugin-window desktop** — every active plugin with an interface lives in its own
  window (slim title bar with close/deactivate + fullscreen; drag and resize in the
  Windows layout). Fullscreen hands the screen to the app and hides the top bar, which
  comes back from the top edge with the app name and close / exit-fullscreen in a glass
  bubble on its left. Layouts: **Master & stack**, **Columns**, or free **Windows**, with
  `Alt`-shortcuts (`Alt+Enter` fullscreen, `Alt+H/L` focus, `Alt+,/.` workspace,
  `Alt+1..9` jump, `Alt+N` / `Alt+Shift+N` add/remove workspace).
- **Bottom chrome** — the AI **orb** (a fluid canvas sphere that takes its palette from
  your accent) above an artifact dock and the compose input.
- With no plugins active Shiny is a bare voice/chat assistant on the desktop shell;
  activating plugins adds their windows and agent tools.

## Features

**Core (always on)**

- Ollama-driven agent loop: voice or typed input, spoken + text replies
- Voice: **faster-whisper STT** with streaming partials through a local Python sidecar
  (default; tiny model bundled, small model downloadable) or **Vosk STT** in the browser
  (per-language models auto-downloaded), plus **Supertonic 3 TTS** through a sidecar
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
- Host sound panel (PipeWire): a top-bar chip with output/input volume, mute
  and default-device switching, fed live from the machine's audio server
- Plugin manager: runtime install/uninstall/activate of `.zip`/`.tar.gz` plugins with
  hot router swap and an install audit log
- Settings and Plugins windows on the desktop (install / activate / activity feed)
- Noir + Light themes with a user-selectable accent and gradient; unified UI library
- GNOME-style plugin notifications and destination insight cards
- Optional Touch Bar surface (MacBook Pro T1/T2) over the same actions the HUD
  and orb expose — native on macOS, through `tiny-dfr` on Linux, and completely
  dormant on any other PC

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
- `image` — layered image editor: bottom-to-top layers and folders with opacity,
  11 blend modes, compositing, merge/flatten, plus Photon effects, filters and
  transforms applied per layer
- `studio` — session-grid music sequencer + synth, rendered to WAV (self-contained DSP on `fundsp`)
- `radio` — internet radio via Radio Browser
- `youtube` — search and watch videos
- `browser` — the **Browser**: a web window. In the kiosk each tab is a real
  child web view (`crates/peakd`'s `browse` module on Linux, `crates/peakd-mac`'s
  on macOS) at the page's true origin, so anti-bot challenges (Cloudflare) pass.
  Ad blocking runs **in-process** (the server compiles EasyList/EasyPrivacy;
  the shell blocks requests via a QtWebEngine request interceptor) with a
  **shield toggle** on the right of the toolbar; a **downloads manager** handles
  every download from every site (progress, pause/resume/cancel/retry/open,
  saved into Files → Downloads); and **incognito** tabs use an off-the-record
  profile. Google sign-in works via a per-host Firefox User-Agent override.
  Plus
  `browser_open` / `browser_search` / `browser_read` tools. Its home surface is a
  **related-news shelf chosen from what you search for**: a decayed interest profile
  over your browsing history picks the topics, the same search engine the address bar
  uses fetches them, and cards are ranked by relevance, freshness and source
- `keyboard` — virtual multi-language on-screen keyboard (8 layouts)
- `hello` — minimal plugin authoring example (one `hello` tool)

## Quick Start

### Prerequisites

- Rust (edition 2021)
- System SQLite (`libsqlite3`)
- Python 3.9+ with `supertonic[serve]` (TTS sidecar) and `faster-whisper` (STT sidecar)
- [Ollama](https://ollama.com) — optional; AI features degrade gracefully when absent
- [GPSD](https://gpsd.io) on `localhost:2947` — optional; falls back to mock GPS
- [PipeWire](https://pipewire.org) with `pipewire-pulse` and WirePlumber, plus
  `pactl` (`pulseaudio-utils`) on `PATH` — optional; powers the top-bar sound
  panel. Without it the chip hides itself and the rest of the app is unchanged.
  On a T2 Mac, `apple-t2-audio-config` supplies the ALSA UCM profiles for the
  internal speakers and microphone.
- [ffmpeg](https://ffmpeg.org) (`ffmpeg` + `ffprobe` on `PATH`, or `FFMPEG_BIN`/`FFPROBE_BIN`)
  — optional; the Files window uses it to render video thumbnails/posters and read
  duration. Without it, videos show a generic icon.
- ~500 MB disk for the Supertonic ONNX model + ~75 MB for the bundled faster-whisper tiny
  model (+ ~480 MB if you opt into the small model) + ~50 MB per Vosk STT language
- **CMake and `nasm`** — the browser's upstream client (`wreq`) links BoringSSL, which
  builds through CMake and needs `nasm` on x86_64 (`brew install cmake nasm` on macOS,
  `apt-get install build-essential cmake nasm` on Debian/Ubuntu). This is what lets
  proxied pages carry a real Chrome TLS fingerprint instead of being blocked as a bot.

### Run

```bash
git clone <repo>
cd shiny

# Optional configuration lives in a `.env` file in the repo root (see below).
# TTS needs the Supertonic sidecar — start it by hand:
./voice/start_supertonic.sh
# …or let the server spawn it (add to `.env`):
#   AUTO_START_SUPERTONIC=true

# STT defaults to faster-whisper; the server starts its sidecar automatically
# (AUTO_START_WHISPER=true). Install the package once if no interpreter has it:
./voice/start_whisper.sh --install

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
| Tap while the assistant is answering | Stop it and start listening again |

**Long-press wake mode listens until the wake word.** It is not on a timer: the mic
stays armed until the phrase arrives (or a tap cancels it). That stays cheap because
nothing leaves the machine while the room is quiet — the browser runs a local
voice-activity gate and only uploads speech (plus a short run-up and tail) to
faster-whisper, so an idle wake listener costs one RMS loop per audio frame.

**Stopping the assistant.** An answer can be stopped while it is being generated, while
it is typing itself out, or while it is being spoken:

- **Text mode** — a round stop button at the bottom-right of the conversation (Escape
  does the same). It appears only while there is an answer to stop.
- **Voice, both modes** — talk over the assistant. The microphone stays open while it
  thinks and speaks; sustained speech above a barge-in threshold cuts the reply off and
  hands the mic straight to the new request, so “wait, I meant…” is not talked over.
  Tapping the orb during an answer does the same thing.

Either way the turn is abandoned server-side (an in-flight model call is dropped rather
than left to finish), the partial answer is kept, and the conversation records an
**invisible** note — a `system` row the agent reads but the chat never renders — saying
the user stopped that reply. The next request therefore knows the answer was cut short
instead of being confused by a truncated thread.

Two speech-recognition engines are selectable in **Settings → Voice**:

- **Faster Whisper** (default) — a `faster-whisper` (CTranslate2) sidecar on the machine
  running the server. Audio streams to it while you speak and words come back as
  partial transcripts, so the end-of-utterance result is both live and much more accurate
  than Vosk. The **tiny** model is bundled with the app; **small** (~480 MB) can be
  downloaded from the same panel.
- **Vosk** — the original in-browser WASM recogniser. No server-side speech service, at
  the cost of accuracy. Its language model (~50 MB) is downloaded from the server and
  cached in the browser.

Whisper needs to be told the language: the picker is always a concrete language (the
browser/system locale on first run), which skips language detection and makes recognition
faster. The same setting drives spoken replies and the assistant's reply language.
Speech synthesis runs on a local Supertonic sidecar.

## Plugins

Every plugin is optional and activated **per user** (in the Plugins window or via its
tray icon). Activating one registers its agent tools and skill docs, mounts its REST
routes, runs its migrations, and — where it has a window surface — opens a window.

| Plugin | Category | Agent tools | REST routes (all authed) | Window |
|---|---|---|---|---|
| `hello` | System | `hello` | — | — (demo) |
| `files` | System | `file_*` (10) | `/api/files/*` | Files (browser: grid/list, thumbnails, preview with Space, double-click opens in the owning app, upload, trash) |
| `keyboard` | System | — | — | on-screen keyboard (8 layouts) |
| `traveler` | Travel | 22: trips, locations, maps/POI, navigation, diary, planning, artifact cards | *domain REST served by core* (`/api/trips`, `/api/map/*`, …) | map + navigator (mounted by core) |
| `word` | Office | `doc_*` (7) | `/api/documents…` | Word (`.odt`) |
| `calc` | Office | `calc_*` (6) | `/api/spreadsheets…` | Calc (`.ods`) |
| `impress` | Office | `slide_*` (6) | `/api/presentations…` | Impress (`.odp`) |
| `pdf` | Office | `pdf_*` (12) | `/api/pdfs…` | PDF viewer/editor (annotations) |
| `mail` | Office | `mail_status/list/read/send` | `/api/mail/*` | Mail (IMAP + SMTP) |
| `calendar` | Office | `calendar_*` (5) | `/api/calendar/events…` | Calendar |
| `calculator` | Office | `calculator_eval/history/clear_history` | `/api/calculator/*` | Calculator |
| `image` | Media | `image_*` (10): docs, layers, merge/flatten, edits | `/api/images…` | Image editor (layers) |
| `radio` | Media | `radio_search/play/stop` | `/api/radio/nowplaying` | Radio |
| `youtube` | Media | `youtube_search/play` | `/api/youtube/search` | YouTube |
| `studio` | Media | `studio_*` + presets/arrangements (12) | `/api/studio/*` | Studio (sequencer) |

Notes: `word` stores real `.odt` bytes; `calc` keeps a JSON cell grid and `impress`
keeps slide data, exchanging real `.ods`/`.odp` files at import/export (codecs live in
the SDK). With the `files` plugin installed, every app's save/export action writes into
the user's home folders (`Documents`, `Pictures`, `Music`, …) via `/api/files/upload`
instead of triggering a browser download — desktop-style; without Files, exports fall
back to a normal download. The traveler plugin adds no routes of its own — the
trip/map/diary REST API and the map window are special-cased in the core binary, so the
plugin itself registers its agent tools and skills.

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

### Linux users

With `SHINY_LINUX_USERS=true` a Shiny account can be **bound to a real Linux
account**: core resolves the login name through NSS (`getpwnam_r`) at login and
caches the `unix_user` / `unix_uid` / `unix_home` on the row. Two things change:

- **Files on the real home** — `SHINY_HOME_MODE=real` makes the Files plugin (and
  every app export) operate on the account's actual `$HOME` instead of
  `~/.shiny/home/<id>`. The classic folders are created only when missing, and
  the sandbox still refuses any path that escapes the home.
- **Real password login** — `SHINY_AUTH_ENABLED=true` verifies the Linux password
  through the privileged `shiny-auth` helper (PAM). The helper is a root-owned,
  socket-activated, **verify-only** process (`scripts/install-linux-auth.sh`):
  it exposes `verify`/`ping`, checks the caller with `SO_PEERCRED`, rate-limits
  attempts, and never stores or logs a password. A PAM **denial is final**; only
  an *unreachable* helper falls back to the local Argon2 hash, so
  `!pam`-provisioned accounts can never be logged into while the helper is down.
  Self-registration is disabled in this mode: accounts are provisioned from the
  Linux account on first successful login.

`GET /api/auth/unix-users` (loopback-only) lists the human accounts the login
picker can offer. Everything is off by default, so a stock install behaves
exactly as before.

### Desktop session and login manager

For a real multi-user desktop, three idempotent installers wire it up:

```bash
sudo scripts/install-linux-auth.sh      # PAM helper + Linux-user mode
sudo scripts/install-linux-session.sh   # per-user server + the `shiny` session
sudo scripts/install-greeter.sh         # LightDM + the Noir greeter
```

- Each login runs its **own** `shiny` server as that user — a systemd **user**
  unit (`/etc/systemd/user/shiny.service`) on port `8080 + uid - 1000`, with its
  state in `~/.local/share/shiny/` (per-user SQLite DB, log, backgrounds). This
  is what lets the Files plugin reach *each* user's real home.
- `/usr/local/bin/shiny-session` waits for that user's server, then runs matchbox
  + `peakd` (Qt 6 + QtWebEngine; `cargo build -p peakd`). Quitting the
  shell (`Alt`+`Q`) ends the session and returns to the greeter.
- LightDM starts `/usr/share/xsessions/shiny.desktop`; the greeter theme lives in
  `greeter/` (installed to `/usr/share/themes/Shiny`). **Autologin is off** — the
  greeter is shown on every boot (`autologin-user` is left commented in
  `/etc/lightdm/lightdm.conf.d/50-shiny.conf`).
- `install-linux-session.sh` disables the single-user `shiny.service` /
  `peakd.service`; both scripts have `--uninstall` to restore the old setup.

## Remote access (Iroh)

Shiny can serve the app itself — **not** screen sharing — to your other devices
over [Iroh](https://www.iroh.computer), a peer-to-peer QUIC transport with NAT
traversal: no port forwarding, no public IP, end-to-end encrypted. Build the
server and shell with `--features iroh`.

- **Turn it on**: Settings → **Remote** → *Server*. The link changes on every
  start/stop. In the kiosk, turning it on hands the screen to the **server-mode
  window** (link + controls); the kiosk returns when you press *Stop server*.
- **Connect**: `peakd --iroh <ticket>` (built with `--features iroh`), or the
  standalone `shiny-iroh-client --ticket <ticket> --listen 127.0.0.1:8080` and
  open `http://127.0.0.1:8080`. Then log in with your password — remote clients
  use the normal web login; the local kiosk does not.
- **Pairing**: *Pair a new device* opens a 120 s window in which the next
  connecting device is added to the allowlist; after that, unpaired keys are
  rejected before any HTTP. *Forget devices* returns to ticket-only access;
  *Rotate key* invalidates the old ticket.
- **Host controls stay local**: a remote client cannot change the machine's
  audio, network, brightness or backlight, and the Terminal is refused unless
  *Allow Terminal from remote clients* is on. Your microphone, speakers and
  location are the client's own.
- The loopback session token (`$XDG_RUNTIME_DIR/shiny-session-token`, mode
  `0600`) logs the local kiosk in without a password and is **never** accepted
  over Iroh or the LAN.

The app runs on port `8080 + uid − 1000` per user; the proxy dials that user's
endpoint. Iroh needs outbound UDP and TCP 443 to its relays and `dns.iroh.link`
(see Iroh's network guide); self-hosted relays are supported.

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
| `WHISPER_URL` | `http://127.0.0.1:7789` | faster-whisper STT sidecar URL |
| `WHISPER_MODELS_DIR` | `data/whisper-models` | faster-whisper (CTranslate2) model storage |
| `AUTO_START_WHISPER` | `true` | Spawn the faster-whisper sidecar on startup |
| `WHISPER_AUTO_INSTALL` | `false` | Let the launcher pip-install faster-whisper into `.venv-whisper` |
| `WHISPER_PYTHON` | — | Interpreter that has `faster-whisper` installed |
| `AUTO_START_SUPERTONIC` | `false` | Spawn the `supertonic serve` sidecar on startup |
| `WEB_DIR` | `web` | Static web UI directory |
| `PLUGINS_DIR` | `data/plugins` | Installed-plugin directory |
| `BACKGROUNDS_DIR` | `data/backgrounds` | Per-user desktop background files |
| `ADMIN_TOKEN` | — | Optional token (exposed to plugins; not enforced by core) |
| `SHINY_LINUX_USERS` | `false` | Bind Shiny accounts to real Linux accounts (NSS lookup, OS-home Files, PAM login). Off keeps today's virtual-home behaviour. |
| `SHINY_HOME_MODE` | `virtual` | Files-plugin home: `virtual` (`~/.shiny/home/<id>`) or `real` (the account's OS home). `real` needs `SHINY_LINUX_USERS=true`. |
| `SHINY_AUTH_ENABLED` | `false` | Verify the real Linux password through the `shiny-auth` helper (falls back to the Argon2 hash when the helper is absent). |
| `SHINY_AUTH_SOCK` | `/run/shiny/auth.sock` | Unix socket the `shiny-auth` helper listens on. |
| `SHINY_AUTH_PAM_SERVICE` | `shiny` | PAM service the **helper** authenticates against (`/etc/pam.d/<name>`); set in the `shiny-auth.service` unit, not the server. |
| `SHINY_SESSION_TOKEN_FILE` | `$XDG_RUNTIME_DIR/shiny-session-token` | Loopback-only session token for the local kiosk / server-mode window. |
| `SHINY_PAIRED_FILE` | `~/.local/share/shiny/paired_devices.json` | Paired-device allowlist for Iroh remote access. |

## Architecture

```
Browser (web/)                             shiny (core binary)
┌───────────────────────────┐              ┌─────────────────────────────────────┐
│ desktop workspace shell    │              │ agent loop (Ollama) + web_search +   │
│  HUD · windows · orb       │    HTTP      │ plugin/desktop control actions       │
│  compose chat · settings   │◀────────────▶│ voice (Vosk/Whisper + Supertonic TTS) │
│  plugins · windows         │              │ auth · multi-user · preferences      │
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
voice/                    # Supertonic + faster-whisper sidecar launchers, model
                          # downloaders, lang map
migrations/               # core schema (001_init .. 007_chat_conversations)
web/                      # browser UI (desktop workspace shell)
```

## Tech Stack

| Component | Crate |
|---|---|
| HTTP | [axum 0.7](https://crates.io/crates/axum) — router hot-swapped via `arc-swap` |
| Database | [sqlx 0.8](https://crates.io/crates/sqlx) + system SQLite (vendored `libsqlite3-sys`) |
| HTTP client | [reqwest 0.12](https://crates.io/crates/reqwest) (rustls) for core services; [wreq 6](https://crates.io/crates/wreq) (BoringSSL, Chrome TLS/JA3 emulation) for the browser's upstream fetch |
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
| POST | `/api/agent/stop` | Stop the turn named by `turn_id` (barge-in / stop button) |
| GET | `/api/ollama/models` | Ollama model list |
| GET | `/api/insights/context` | Destination insight cards |
| GET · POST | `/api/artifacts` · GET · PUT `/api/artifacts/:id` | Saved artifact cards |
| POST | `/api/tts` | Supertonic TTS proxy → `audio/wav` |
| GET | `/api/audio/status` | Host audio: output/input devices, volume, mute (PipeWire) |
| GET | `/api/audio/events` | Audio change stream (SSE) |
| POST | `/api/audio/volume` · `/api/audio/mute` · `/api/audio/default` | Set volume, mute, default device (loopback only) |
| GET | `/api/touchbar` | Host Touch Bar capability (T2 MacBook) |
| GET · POST | `/api/keyboard/backlight` | Host keyboard backlight (write is loopback-only) |
| GET · POST | `/api/screen/brightness` | Host screen brightness (write is loopback-only) |
| GET | `/api/voice/status` | Vosk + faster-whisper + Supertonic readiness, model inventory |
| POST | `/api/voice/download` | Download a Vosk model |
| POST | `/api/voice/whisper/download` | Background-download a faster-whisper model (`tiny`/`small`) |
| POST | `/api/voice/stt/chunk` | Stream one PCM chunk to faster-whisper (raw 16 kHz mono PCM16LE body) |
| POST | `/api/voice/stt/close` | Drop a faster-whisper streaming session |
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
`/api/studio/*` (`studio`), `/api/files/*` (`files`).

**Static** — the app shell is served at `/`; `/plugins/<name>/*` serves each
plugin’s web assets; everything else falls back to the app (`web/`). Settings and
Plugins are built-in windows, not separate pages.

## Voice & Languages

`voice/lang_map.json` drives both sides of the voice stack:

- **TTS** — Supertonic 3 via the sidecar, with voice codes for 32 languages (zh uses
  the English voice).
- **STT (faster-whisper)** — the model's language is the ISO-639-1 code itself, passed
  explicitly so Whisper skips language detection. All 32 languages are supported by the
  model; tiny is best for English and small is a worthwhile upgrade for the rest.
- **STT (Vosk)** — Vosk small models download on demand: 19 languages ship a native Vosk
  model; the remaining 13 fall back to the English model.

### faster-whisper sidecar

`voice/whisper_server.py` (FastAPI + `faster-whisper`) holds one model in memory and
implements the *LocalAgreement-2* streaming policy: the core server posts microphone
chunks to `/api/voice/stt/chunk`, the sidecar re-decodes only the uncommitted tail and
returns a growing partial transcript, and a final flush re-decodes the whole utterance
for accuracy. `voice/start_whisper.sh` finds an interpreter with `faster-whisper`
(app venv → `python3` → common system/conda installs), optionally building
`.venv-whisper` with `--install`.

The **tiny** model is the default. It is not committed to git (all of `data/` is
ignored), so `voice/start_whisper.sh` downloads it once (~72 MB) the first time the
sidecar starts. The **small** model is downloaded on demand from Settings → Voice. Both
go through `voice/download_whisper.py`, which is stdlib-only so it works under any
`python3`.

## Host audio (PipeWire)

The top-bar sound chip controls the machine itself: output and input volume,
mute, and the default devices. Core reads PipeWire through its PulseAudio
compatibility socket — `pactl --format=json` snapshots plus `pactl subscribe`
for changes — so the host needs `pipewire-pulse` and `pactl`
(`pulseaudio-utils`). Without them the chip hides itself.

The snapshot is cached and broadcast, exactly like the network panel:
`/api/audio/status` reads the cache, `/api/audio/events` relays changes as SSE,
and the mutations (`/api/audio/volume`, `/api/audio/mute`, `/api/audio/default`)
are accepted only from the local machine. Every `pactl` call is spawned with a
runtime-dir fallback (`/run/user/<uid>`), so the service finds the user's
socket without session environment.

### Running unprivileged

The server, the kiosk and PipeWire all run as the **desktop user**, never as
root: the kiosk is a browser rendering the open web, and PipeWire refuses to
run as root by design (Debian's units ship `ConditionUser=!root`). On this
machine the desktop user is `eev`:

- `loginctl enable-linger eev` keeps `user@1000.service` — and therefore
  PipeWire — running from boot, so `/run/user/1000` exists for the server and
  the kiosk. The daemons are enabled in that user's session:
  `systemctl --user enable --now pipewire.socket pipewire-pulse.socket wireplumber.service`.
- `shiny.service` runs as `User=eev` with `XDG_RUNTIME_DIR=/run/user/1000` and
  orders itself after `user@1000.service`.
- `peakd.service` runs as `User=eev` with `PAMName=login`: the PAM session
  registers seat0 as active, which is what lets an unprivileged Xorg take the
  DRM/input devices (no setuid Xorg wrapper) and hands the shell's web engine
  the user's PipeWire socket. `peakd-kiosk.sh` therefore needs no
  `PULSE_SERVER`.
- Wi-Fi control (the network chip) needs one polkit grant for a service
  without an active seat session:
  `/etc/polkit-1/rules.d/49-shiny-network.rules` allows that user exactly the
  NetworkManager actions the panel performs. The panel is loopback-only, so
  remote callers cannot reach them.

On a T2 Mac the internal speakers and mic come from the T2 kernel's
`t2bce_audio` driver plus `apple-t2-audio-config` (ALSA UCM profiles).
PipeWire + WirePlumber expose them as the `HiFi` profile — 6-channel speakers
and a 3-channel mic.

### Power and lid behaviour

The session disables X blanking (`xset s off`, `xset -dpms` in
`peakd-kiosk.sh`), so an idle kiosk never goes dark on its own. The remaining
power handling separates what is a hardware bug from what is wanted.

The T2 Mac's ACPI buttons report **phantom presses**, and `systemd-logind`
acted on all of them:

- **Power Button** (`PWRB`, `/dev/input/event1`) → `Power key pressed short.`,
  observed 07:31, 10:43, 10:50, 11:25, 11:27, 12:05, 12:19 on 2026-09-20 —
  each one **powered the machine off** (the default `HandlePowerKey=poweroff`),
  which looked like "the kiosk dropped to a terminal and I had to restart".
- **Sleep Button** (`SLPB`, `/dev/input/event2`) → `Suspend key pressed short.`,
  observed 11:37, 11:44, 11:57 — each one suspended the machine.

So `/etc/systemd/logind.conf.d/49-shiny-kiosk.conf` ignores the *short*
phantom press but keeps a deliberate *long* hold working:

```
HandlePowerKey=ignore            HandlePowerKeyLongPress=poweroff
HandleSuspendKey=ignore          HandleSuspendKeyLongPress=suspend
HandleHibernateKey=ignore        HandleHibernateKeyLongPress=hibernate
IdleAction=ignore
HandleLidSwitch=suspend          HandleLidSwitchExternalPower=suspend
HandleLidSwitchDocked=ignore
```

Idle never suspends; closing the lid does (except when docked). This lives
outside the repo — re-install it after a re-provision. The sleep targets must
stay **unmasked** for lid-close suspend to work.

### Power/kiosk event log

Because these phantom events are invisible in the moment, everything power- and
kiosk-related is recorded to `/var/log/shiny-power.log`:

- `shiny-powerlog.service` follows the journal and appends button presses
  (logged even when the action is `ignore`), suspends/resumes, and the kiosk
  starting/dying.
- the `systemd-sleep` hook `/etc/systemd/system-sleep/50-shiny-log` snapshots
  the kiosk processes, sessions and display state around every suspend/resume,
  so it is obvious whether the kiosk came back.

Read it with:

```bash
shiny-power-report        # the log plus recent power journal
shiny-power-report -f     # follow it live
```

Finally, `peakd.service.d/10-wait-drm.conf` runs
`/usr/local/bin/peakd-wait-drm.sh` first: without it Xorg won the race against
the i915/amdgpu probe on the first start of every boot and died with
`Cannot run in framebuffer mode`, flashing the console until `Restart=` caught
it.

## Touch Bar (MacBook Pro T1/T2)

Shiny can drive the Touch Bar, and — this is the important part — it does so
without assuming the machine has one. The bar is an **optional input surface**
over actions the HUD and orb already expose. On a normal PC nothing is
registered, nothing is installed, and the app behaves exactly as before.

There are two transports, but one action vocabulary
(`web/js/touchbarShared.js`):

- **macOS** — `peakd` puts a native `NSTouchBar` on the kiosk window
  (`crates/peakd-mac/src/touchbar.rs`); each button evaluates a `touchbar:action`
  event in the page. A Mac without Touch Bar hardware simply never shows the
  bar. Turn it off with `PEAKD_TOUCHBAR=0` or `peakd-mac --no-touchbar`.
- **Linux T2** — the kernel (`hid-appletb-*` / `apple-ib-tb`) plus the
  `tiny-dfr` daemon own the bar. `sudo scripts/touchbar/install-touchbar.sh`
  installs a Shiny icon row (mic, stop, mute, volume, screen brightness,
  workspace arrows, keyboard backlight, on-screen keyboard, gear) and copies
  the four custom SVGs into `/etc/tiny-dfr`. It also installs a udev rule that
  lets the desktop user (via the `video` group) write the `kbd_backlight` LED
  and the panel `backlight`, so the server can dim or brighten the keyboard and
  the screen — both are plain sysfs, no GTK or desktop daemon, which is why
  they work in the matchbox kiosk.
  `tiny-dfr` can only emit key codes, so each button sends a
  **Ctrl+Alt+Shift+1…9** combo, which `web/js/touchbar.js` maps back to actions.
  (A combo rather than F13–F24: the X keymap binds those codes to
  `XF86*` keysyms on a `us`/`es` layout, so the page would never see `F13`;
  digits are mapped everywhere and the page matches the physical
  `event.code`.) The script is a clean no-op
  unless it sees the T2 hardware, backs the existing config up once, and
  `--uninstall` restores it. The media layer is left to the distro, so Fn still
  reaches brightness and the media keys. The daemon needs the desktop user in
  the `video` group to reach the display devices.

Because `tiny-dfr` owns the bar globally, the row shows in every app, not only
the kiosk — it has no per-app layers.

The page learns a bar exists from two places: the kiosk shell's init flag, or
the server's `GET /api/touchbar` probe (`available`). The server runs on the
same machine and does the same sysfs check, so **Automatic** works in a plain
browser too, not just under `peakd`.

The buttons map to: tap-to-talk (the orb's exact gesture, barge-in included),
stop the answer, host output mute / volume, screen brightness, previous/next
workspace, keyboard backlight, toggle the virtual keyboard, and open Settings.

Per-user enablement lives in **Settings → Touch Bar**: *Automatic* (only when
the host reports a bar), *Always on* (force it, to try the buttons on a normal
keyboard) or *Off*. The default is Automatic, so an ordinary PC never reacts to
the combo. `web/js/tests/touchbar.test.mjs` pins the web vocabulary, the
`tiny-dfr` TOML and the native macOS button list together, so the three cannot
drift apart.

## Touchpad gestures (three fingers)

The kiosk recognises three-finger trackpad swipes and maps them onto the
desktop:

| Swipe | Action |
|---|---|
| left | next workspace |
| right | previous workspace |
| up | window overview — every open window, live |
| down | plugin launcher |

The web engines do not deliver trackpad gestures to the page (X11 has no
protocol for them, and no gesture daemon is installed on the image), so the
Linux shell reads the internal touchpad's evdev stream itself
(`crates/peakd/src/gestures.rs`) and dispatches a `trackpad:gesture` event
that
`web/js/gestures.js` maps onto the desktop. The reader only *watches* the
device — read-only, never `EVIOCGRAB` — so the pointer and two-finger scrolling
are untouched. macOS has native multi-touch gestures and does not use this path.

The reader needs permission to open `/dev/input/event*`, and the kiosk runs as
an unprivileged user, so install the udev rule once:

```bash
sudo scripts/install-touchpad-gestures.sh
```

It tags the touchpad with logind's `uaccess` (an ACL for the active seat
session) and reloads udev. Without the rule the shell logs that gestures are off
and everything else runs unchanged. `PEAKD_TOUCHPAD=/dev/input/eventN`
overrides device discovery on unusual hardware.

## Graceful Degradation

| Service | If unavailable |
|---|---|
| Ollama | Chat / diary / agent tools error; everything else runs |
| Supertonic | TTS fails; STT still works |
| faster-whisper | Voice falls back to the in-browser Vosk engine |
| PipeWire / `pactl` | The sound chip hides itself; the rest of the app is unchanged |
| GPSD | Mock GPS (fixed point + drift) |
| Touch Bar | The feature stays dormant: the native bar is not installed off macOS and `auto` mode never activates. On Linux, `install-touchbar.sh` exits without touching anything. |
| Touchpad gestures | The evdev reader finds no readable device and stays off; the pointer, scrolling and the rest of the app are unchanged. |
| Iroh remote access | Not compiled in or not enabled → no endpoint is bound; Settings shows it Off and the built-in login/desktop are unchanged. An unreachable relay still leaves local addresses. |
| `shiny-auth` helper | Login falls back to the local Argon2 hash; `remote`/Linux-user features still work. |
| ffmpeg | Video thumbnails/posters fall back to a generic icon (playback is unaffected) |
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
