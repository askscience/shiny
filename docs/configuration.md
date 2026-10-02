# Configuration

Shiny reads configuration from the process environment. A `.env` file in the
repository root is loaded at startup with [`dotenvy`](../src/main.rs) and takes
precedence only over the process defaults (real environment variables still
win). `RUST_LOG` overrides `LOG_LEVEL`.

All values parsed by the binary live in
[`Config::from_env`](../src/config.rs); the exhaustive variable reference is
[Environment variables](reference/env-vars.md). This page groups them by intent.

---

## Data directories

| Path | Default | Override | Contents |
|---|---|---|---|
| Database | `sqlite://data/traveler.db` | `DATABASE_URL` | Identity, travel, chat, plugin state. |
| Log | `data/shiny.log` | `LOG_FILE` | Tee'd alongside stdout. |
| Plugins | `data/plugins` | `PLUGINS_DIR` | Installed plugin folders + `install.log` + `.install.lock`. |
| Backgrounds | `data/backgrounds` | `BACKGROUNDS_DIR` | Per-user desktop background images. |
| Vosk models | `data/vosk-models` | `VOSK_MODELS_DIR` | Served to the browser at `/api/voice/models/vosk`. |
| Whisper models | `data/whisper-models` | `WHISPER_MODELS_DIR` | faster-whisper CTranslate2 folders. |
| Qwen TTS models | `data/qwen-tts-models` | `QWEN_TTS_MODELS_DIR` | Qwen3-TTS GGUF weights. |
| Web root | `web` | `WEB_DIR` | Static UI, served with an SPA fallback. |
| User homes | `~/.shiny/home/<id>` | `SHINY_HOME_MODE=real` | Files-plugin home; `real` uses the OS home. |
| Paired devices | `~/.local/share/shiny/paired_devices.json` | `SHINY_PAIRED_FILE` | Iroh allowlist. |
| Session token | `$XDG_RUNTIME_DIR/shiny-session-token` | `SHINY_SESSION_TOKEN_FILE` | Loopback-only kiosk login. |

---

## Server

| Variable | Default | Meaning |
|---|---|---|
| `SERVER_HOST` | `0.0.0.0` | HTTP bind address. |
| `SERVER_PORT` | `8080` | HTTP port. Per-user installs use `8080 + uid − 1000`. |
| `LOG_LEVEL` | `info` | Filter used when `RUST_LOG` is unset. |
| `LOG_FILE` | `data/shiny.log` | File the tracer tees to. |

## AI provider

| Variable | Default | Meaning |
|---|---|---|
| `OLLAMA_URL` | `http://127.0.0.1:11434` | Ollama base URL. |
| `OLLAMA_MODEL` | `gemma4:31b-cloud` | Default model. |
| `ADMIN_TOKEN` | — | Read into `ConfigSnapshot` for plugins; **not enforced** by core auth. |

Users can additionally configure an OpenAI-compatible provider per account in
their Assistant settings; see [agent](core/agent.md).

## Voice

| Variable | Default | Meaning |
|---|---|---|
| `SUPERTONIC_URL` | `http://127.0.0.1:7788` | Supertonic TTS sidecar. |
| `SUPERTONIC_VOICE` | `M1` | Default voice preset. |
| `AUTO_START_SUPERTONIC` | `true` | Spawn the Supertonic sidecar with the server. |
| `SUPERTONIC_AUTO_INSTALL` | `true` | Let the launcher provision `.venv-supertonic`. |
| `SUPERTONIC_PYTHON` | — | Explicit interpreter for the sidecar. |
| `WHISPER_URL` | `http://127.0.0.1:7789` | faster-whisper STT sidecar. |
| `WHISPER_MODELS_DIR` | `data/whisper-models` | Model storage. |
| `AUTO_START_WHISPER` | `true` | Spawn the faster-whisper sidecar with the server. |
| `WHISPER_AUTO_INSTALL` | `false` | Let the launcher pip-install faster-whisper. |
| `WHISPER_PYTHON` | — | Explicit interpreter for the sidecar. |
| `QWEN_TTS_URL` | `http://127.0.0.1:7787` | Qwen3-TTS sidecar. |
| `QWEN_TTS_MODELS_DIR` | `data/qwen-tts-models` | GGUF model storage. |
| `AUTO_START_QWEN_TTS` | `false` | Opt-in: first start may build the server and fetch ~600 MB. |

See [voice](core/voice.md) and [sidecars](deployment/sidecars.md).

## Travel

| Variable | Default | Meaning |
|---|---|---|
| `GPSD_HOST` | `127.0.0.1` | GPSD host. |
| `GPSD_PORT` | `2947` | GPSD port. |
| `DIARY_AUTO_GENERATE` | `true` | Enable the daily diary cron. |
| `DIARY_GENERATE_TIME` | `21:00` | Local `HH:MM` at which the diary is generated. |

## Linux users & authentication

| Variable | Default | Meaning |
|---|---|---|
| `SHINY_LINUX_USERS` | `false` | Bind Shiny accounts to real Linux accounts (NSS lookup, OS-home Files, PAM login). |
| `SHINY_HOME_MODE` | `virtual` | Files home: `virtual` (`~/.shiny/home/<id>`) or `real` (the OS home). `real` requires `SHINY_LINUX_USERS=true`. |
| `SHINY_AUTH_ENABLED` | `false` | Verify the real Linux password through the `shiny-auth` helper (falls back to Argon2 when absent). |
| `SHINY_AUTH_SOCK` | `/run/shiny/auth.sock` | Unix socket the helper listens on. |
| `SHINY_AUTH_PAM_SERVICE` | `shiny` | PAM service the **helper** authenticates against; set in the helper's unit. |
| `SHINY_SESSION_TOKEN_FILE` | `$XDG_RUNTIME_DIR/shiny-session-token` | Loopback-only session token path. |

See [data & auth](core/data-and-auth.md) and
[multi-user Linux](deployment/multi-user-linux.md).

## Remote access

Remote access shares the server port. Iroh is compiled behind the `iroh` feature
(`cargo build --features iroh`); Tailscale uses the `tailscale` CLI. There are no
dedicated env vars for the toggles — they are per-user preferences and
commands — except `SHINY_PAIRED_FILE` for the allowlist.

## Plugins

| Variable | Default | Meaning |
|---|---|---|
| `PLUGINS_DIR` | `data/plugins` | Where plugins live and the install log is written. |
| `ADMIN_TOKEN` | — | Exposed to plugins via `ConfigSnapshot`; not enforced. |
| `CORE_TRAVELER_BUILTIN` | `true` | Legacy switch: when true the embedded traveler code still answers unclaimed actions. |

Plugin-specific settings (mail accounts, browser search engine, …) are stored
per user in the database, not in `.env`. See the plugin's own `docs/` folder.

---

## Example `.env`

```dotenv
# Server
SERVER_HOST=127.0.0.1
SERVER_PORT=8080
LOG_LEVEL=debug

# AI
OLLAMA_URL=http://127.0.0.1:11434
OLLAMA_MODEL=gemma4:31b-cloud

# Voice: let the server bring the sidecars up
AUTO_START_WHISPER=true
AUTO_START_SUPERTONIC=true

# Multi-user
SHINY_LINUX_USERS=false
SHINY_HOME_MODE=virtual
SHINY_AUTH_ENABLED=false
```

---

## Notes

- Parsing is permissive: booleans accept `true`/`false` and fall back to their
  default on anything else; ports fall back to the default on parse failure.
- An empty `OLLAMA_URL`, `WHISPER_URL` or `QWEN_TTS_URL` is treated as unset.
- Mutating host state (audio/network/brightness/power) is gated to the local
  machine; remote clients get read-only panels. This is enforced in
  [`host_remote_gate`](../src/api/mod.rs), not by config.
- The loops in `voice/start_*.sh` read their own variables (`WHISPER_PORT`,
  `SUPERTONIC_PORT`, `SUPERTONIC_AUTO_INSTALL`, …); the server maps its config
  to those when it spawns them.
