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
| Tap the sphere | Listen once, reply spoken aloud |
| Long-press the sphere | Wake mode — hold, say "Hey &lt;assistant name&gt;", keep talking, release to end |
| Double-tap the sphere | Type a message (Markdown replies) |
| Tap while it is answering | Stop it and start listening again |

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
