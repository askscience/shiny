# Getting started

This guide takes you from a clone to a running assistant. For the conceptual
picture, read [Architecture](architecture.md) first.

---

## 1. Prerequisites

**Required**

- **Rust** (edition 2021) — build the server and workspace.
- **System SQLite** (`libsqlite3`) — the server links the system library, not a
  bundled copy.
- **Python 3.9+** — for the speech sidecars. Optional in the sense that the app
  still runs, but voice needs it:
  - `supertonic[serve]` for text-to-speech;
  - `faster-whisper` for streaming speech-to-text.
- **Ollama** — optional, but chat/diary/agent features degrade without it.

**Optional**

- **GPSD** on `localhost:2947` (travel); falls back to mock GPS.
- **PipeWire** + `pipewire-pulse` + WirePlumber + `pactl`
  (`pulseaudio-utils`) — the top-bar sound panel.
- **BlueZ** (`bluez`) — the Bluetooth panel.
- **ffmpeg** / `ffprobe` on `PATH` (or `FFMPEG_BIN`/`FFPROBE_BIN`) — video
  thumbnails in Files.
- **CMake** and **nasm** — required to build the Browser's upstream client
  (`wreq` links BoringSSL). `brew install cmake nasm` on macOS,
  `apt-get install build-essential cmake nasm` on Debian/Ubuntu.
- **tailscale** — only for the public-URL remote option.

Disk: ~500 MB for the Supertonic ONNX model, ~75 MB for the bundled
faster-whisper `tiny` model (+ ~480 MB for `small`), ~50 MB per Vosk language.

---

## 2. Build and run

```bash
git clone <repo>
cd shiny

# Optional: create a .env (see docs/configuration.md for a starter file).

# Text-to-speech: start the Supertonic sidecar by hand, or set
# AUTO_START_SUPERTONIC=true in .env and let the server do it.
./voice/start_supertonic.sh

# Speech-to-text defaults to faster-whisper; the server starts its sidecar
# automatically (AUTO_START_WHISPER=true). Install the package once if no
# interpreter has it:
./voice/start_whisper.sh --install

cargo run
```

Open <http://localhost:8080>. Microphone access requires HTTPS unless you
connect over `localhost`.

> The first `faster-whisper` start downloads the `tiny` model (~72 MB) because
> `data/` is gitignored. The `small` model is downloaded on demand from
> Settings → Voice.

### Release build

```bash
cargo build --release          # server + all workspace members
cargo build --features iroh    # include peer-to-peer remote access
```

The [kiosk shell](deployment/kiosk-shell.md) is a separate binary
(`cargo build -p peakd` on Linux, `-p peakd-mac` on macOS).

---

## 3. First run

1. The server creates `data/traveler.db`, applies migrations `001`–`009`, starts
   the host panels and loads any plugins in `data/plugins`.
2. On the sign-in screen, **register** the first account (username + password).
   The first account is flagged `is_admin`, but Shiny has no enforced admin
   role — every account is a peer.
3. You land on the desktop shell: a top HUD, an empty desktop, and the voice
   bar at the bottom. With no plugins active, this is a bare voice/chat
   assistant.
4. Open **Plugins** (right side of the HUD) and activate what you want. Each
   activation opens the plugin's window (if it has one) and registers its agent
   tools.

### Talk to it

| Gesture | Action |
|---|---|
| Tap the voice bar | Type a message (Markdown replies) |
| Long-press the voice bar | Wake mode — hold, say "Hey &lt;assistant name&gt;", keep talking, release to end |
| Double-tap the voice bar | Listen once, reply spoken aloud |
| Double-tap while it is answering | Stop it and start listening again |

Everything in the UI is also reachable by voice/typed instruction through the
agent. See [agent](core/agent.md) and [voice](core/voice.md).

---

## 4. Install a plugin

Plugins are installed at runtime — no restart.

```bash
# In the UI: Settings/Plugins → install, or via the API:
curl -X POST http://localhost:8080/api/plugins/install \
  -H "Authorization: Bearer $TOKEN" \
  -F "file=@my-plugin.zip"
```

An archive is a folder with `plugin.toml` + a `.so`/`.dylib`/`.dll` (+ optional
`migrations/`, `skills/`, `web/`). See [authoring a plugin](plugins/authoring.md).

---

## 5. Where to go next

- Configure it: [Configuration](configuration.md) and
  [Environment variables](reference/env-vars.md).
- Understand the internals: [Architecture](architecture.md).
- Talk to the agent: [Agent](core/agent.md).
- Work on voice: [Voice](core/voice.md) and [Sidecars](deployment/sidecars.md).
- Write a plugin: [Plugins](plugins/README.md).
- Deploy a kiosk / multi-user machine: [Deployment](deployment/README.md).

---

## Troubleshooting the first run

| Symptom | Check |
|---|---|
| `error: linker not found` / SQLite errors | Install the system `libsqlite3` dev package. |
| AI errors | Is Ollama running at `OLLAMA_URL`? `curl $OLLAMA_URL/api/tags`. |
| No microphone | Serve over `localhost`/HTTPS; browser mic permissions. |
| TTS silent | Is the Supertonic sidecar up on `SUPERTONIC_URL`? |
| STT does nothing | faster-whisper sidecar up? Otherwise Vosk downloads a model. |
| Sound/Bluetooth chip missing | Expected when PipeWire/BlueZ are absent — see [host](host/README.md). |
| Plugin window blank | Check the install log at `data/plugins/install.log` and the browser console. |

## Development verification

Run checks without starting or restarting the desktop:

```bash
scripts/test-web.sh
scripts/check-format.sh
CARGO_BUILD_JOBS=1 scripts/test-rust.sh
cargo clippy --locked --workspace --exclude peakd --exclude peakd-mac --all-targets
```

The web check syntax-checks first-party JavaScript and runs every Node test and
the Browser and Image plugin surface smoke checks,
stopping at the first failure. There is no frontend bundler or npm installation.
The formatting gate covers the refactored modules; older formatting is kept
separate from functional changes.

The Rust runner explicitly builds native plugin fixtures before testing. Tests
use unique temporary databases and staged plugin source assets; missing libraries
fail with instructions rather than silently skipping. The default fixture library
directory is `target/debug`; set `SHINY_TEST_LIB_DIR` explicitly for another build
profile or `CARGO_TARGET_DIR`. Tests never auto-discover installed plugins or run
plugin startup hooks. Keep build concurrency low while working inside Terminal.

`tests/fixtures/plugin-contracts.json` pins native tools and routes. To recapture
an intentionally changed contract, run only the explicit capture test against the
chosen known build, then review the resulting diff:

```bash
cargo test -p shiny --test plugin_contracts capture_plugin_contracts -- --ignored
```

CI builds the Qt kiosk shell separately on Debian 13. Hardware and session
acceptance checks are listed in [Kiosk shell](deployment/kiosk-shell.md).

The Flow UI checks cover interrupted reveals, reduced motion, renderer stalls,
dialog reopening and nested focus isolation. The Settings lifecycle check mounts
all 12 sections and verifies cleanup across locale changes. Keep existing
translation keys when reorganizing controls; add catalog entries for new text.
