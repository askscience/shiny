# Environment variables

This is the exhaustive reference. The `.env` file in the repository root is
loaded by [`dotenvy`](../../src/main.rs); real environment variables still take
precedence. Core values are parsed in
[`Config::from_env`](../../src/config.rs); the rest are read by the launchers,
the kiosk shell and the auth helper.

**Legend:** ✓ = read by the server core; L = sidecar launcher; K = kiosk shell
(`peakd`/`peakd-mac`); A = `shiny-auth` helper; G = greeter session launcher
(`scripts/shiny-greeter`).

---

## Server & core

| Variable | Default | Read by | Meaning |
|---|---|---|---|
| `SERVER_HOST` | `0.0.0.0` | ✓ | HTTP bind address. |
| `SERVER_PORT` | `8080` | ✓ | HTTP port. Per-user installs use `8080 + uid − 1000`. |
| `DATABASE_URL` | `sqlite://data/traveler.db` | ✓ | SQLite URL/path. |
| `SYSTEM_PLUGINS_DIR` | — | — | Read-only system plugin baseline; a same-named plugin in `PLUGINS_DIR` overrides it. |
| `WEB_DIR` | `web` | ✓ | Static UI directory (SPA fallback). |
| `LOG_LEVEL` | `info` | ✓ | Filter used when `RUST_LOG` is unset. |
| `LOG_FILE` | `data/shiny.log` | ✓ | File the tracer tees to. |
| `RUST_LOG` | — | ✓ | Overrides `LOG_LEVEL`. |
| `PLUGINS_DIR` | `data/plugins` | ✓ | Writable plugin directory (uploads + `install.log`). |
| `SHINY_MIN_PLUGIN_TS` | security floor | loader | Warn when a plugin cdylib predates this Unix timestamp; `0` disables the check. |
| `SHINY_MAIL_KEY_FILE` | `data/mail.key` | mail | AES-256-GCM key file (32 bytes, mode 0600) for encrypted account passwords. |
| `BACKGROUNDS_DIR` | `data/backgrounds` | ✓ | Per-user desktop backgrounds. |
| `ADMIN_TOKEN` | — | ✓ | Exposed to plugins via `ConfigSnapshot`; **not enforced**. |
| `SHINY_POWER_SUPPLY_DIR` | `/sys/class/power_supply` | ✓ | Override the battery sysfs root. |
| `SHINY_PORT` | — | K (older) | Legacy port hint in the kiosk. |

## AI provider

| Variable | Default | Read by | Meaning |
|---|---|---|---|
| `OLLAMA_URL` | `http://127.0.0.1:11434` | ✓ | Ollama base URL (empty = unset). |
| `OLLAMA_MODEL` | `gemma4:31b-cloud` | ✓ | Default model. |

## Voice & sidecars

| Variable | Default | Read by | Meaning |
|---|---|---|---|
| `SUPERTONIC_URL` | `http://127.0.0.1:7788` | ✓ | Supertonic TTS. |
| `SUPERTONIC_VOICE` | `M1` | ✓ | Default voice preset. |
| `AUTO_START_SUPERTONIC` | `true` | ✓ | Spawn the Supertonic sidecar. |
| `SUPERTONIC_AUTO_INSTALL` | `true` | ✓ L | Provision `.venv-supertonic`. |
| `SUPERTONIC_PYTHON` | — | ✓ L | Explicit interpreter. |
| `SUPERTONIC_HOST` / `SUPERTONIC_PORT` | `127.0.0.1` / `7788` | L | Sidecar bind. |
| `SUPERTONIC_LOG_FILE` | `data/supertonic-sidecar.log` | L | Log file. |
| `SUPERTONIC_BOOTSTRAP_PYTHON` | `python3` | L | Interpreter used to build the venv. |
| `WHISPER_URL` | `http://127.0.0.1:7789` | ✓ | faster-whisper STT. |
| `WHISPER_MODELS_DIR` | `data/whisper-models` | ✓ L | Model storage. |
| `AUTO_START_WHISPER` | `true` | ✓ | Spawn the faster-whisper sidecar. |
| `WHISPER_AUTO_INSTALL` | `false` | ✓ L | pip-install faster-whisper. |
| `WHISPER_PYTHON` | — | ✓ L | Explicit interpreter. |
| `WHISPER_HOST` / `WHISPER_PORT` | `127.0.0.1` / `7789` | L | Sidecar bind. |
| `WHISPER_LOG_FILE` | `data/whisper-sidecar.log` | L | Log file. |
| `WHISPER_LOCK_FILE` | `$XDG_RUNTIME_DIR/shiny-whisper-7789.lock` | L | Start lock — one launcher starts the sidecar, the rest wait (falls back to `/tmp`). |
| `WHISPER_BOOTSTRAP_PYTHON` | `python3` | L | Downloader/venv interpreter. |
| `KMP_DUPLICATE_LIB_OK` | `TRUE` (set by launcher) | L | OpenMP duplicate-runtime tolerance. |
| `VOSK_MODELS_DIR` | `data/vosk-models` | ✓ | Vosk model storage (served to the browser). |
| `QWEN_TTS_URL` | `http://127.0.0.1:7787` | ✓ | Qwen3-TTS. |
| `QWEN_TTS_MODELS_DIR` | `data/qwen-tts-models` | ✓ L | GGUF storage. |
| `AUTO_START_QWEN_TTS` | `false` | ✓ | Opt-in: may build the server + fetch ~600 MB. |
| `QWEN_TTS_PORT` | `7787` | L | Sidecar bind. |

## Travel

| Variable | Default | Read by | Meaning |
|---|---|---|---|
| `GPSD_HOST` | `127.0.0.1` | ✓ | GPSD host. |
| `GPSD_PORT` | `2947` | ✓ | GPSD port. |
| `DIARY_AUTO_GENERATE` | `true` | ✓ | Enable the daily diary cron. |
| `DIARY_GENERATE_TIME` | `21:00` | ✓ | Local `HH:MM`. |

## Linux users & auth

| Variable | Default | Read by | Meaning |
|---|---|---|---|
| `SHINY_LINUX_USERS` | `false` | ✓ | Bind accounts to real Linux accounts. |
| `SHINY_HOME_MODE` | `virtual` | ✓ | `virtual` or `real` (`real` requires Linux users). |
| `SHINY_AUTH_ENABLED` | `false` | ✓ | Verify the real Linux password via the helper. |
| `SHINY_AUTH_SOCK` | `/run/shiny/auth.sock` | ✓ A | Helper socket. |
| `SHINY_LOGIN_SELF_ONLY` | = `SHINY_LINUX_USERS` | ✓ | User sessions: only the server's own OS account may log in (and be listed). |
| `SHINY_GREETER` | `false` | ✓ | Greeter mode: login verifies + starts that account's session instead of issuing a cookie. |
| `SHINY_GREETER_PORT` | `8079` | G | Greeter server port (loopback). |
| `SHINY_AUTH_PAM_SERVICE` | `shiny` | A | PAM service (`/etc/pam.d/<name>`). |
| `SHINY_AUTH_ALLOW_UID` | — | A | uids allowed to connect (one, or a comma-separated list; root always). |
| `SHINY_AUTH_KIOSK_UNIT` | `shiny-kiosk@%s.service` | A | Unit the `login-session` op starts (`%s` = verified account). |
| `SHINY_AUTH_HANDOVER_FILE` | `/run/shiny/handover` | A | Handover marker the greeter's stop script checks. |
| `SHINY_AUTH_SYSTEMCTL` | `/usr/bin/systemctl` | A | systemctl path. |
| `SHINY_AUTH_DRY_RUN` | — | A | Report the unit instead of starting it. |
| `SHINY_AUTH_MAX_ATTEMPTS` | — | A | Per-user attempt cap. |
| `SHINY_AUTH_GLOBAL_MAX_ATTEMPTS` | — | A | Global attempt cap. |
| `SHINY_AUTH_WINDOW_SECS` | — | A | Rate-limit window (seconds). |
| `SHINY_AUTH_TIMEOUT_SECS` | — | A | Verify timeout. |

## Remote access

| Variable | Default | Read by | Meaning |
|---|---|---|---|
| `SHINY_PAIRED_FILE` | `~/.local/share/shiny/paired_devices.json` | ✓ | Iroh device allowlist. |
| `SHINY_SESSION_TOKEN_FILE` | `$XDG_RUNTIME_DIR/shiny-session-token` | ✓ | Loopback session token. |
| `XDG_RUNTIME_DIR` | `/run/user/<uid>` | ✓ K | Runtime dir (token + remote state). |

Iroh itself is a build feature (`--features iroh`); Tailscale uses the
`tailscale` CLI. See [remote access](../deployment/remote-access.md).

## Kiosk shell (`peakd` / `peakd-mac`)

| Variable | Default | Read by | Meaning |
|---|---|---|---|
| `PEAKD_APP_ORIGIN` | `http://127.0.0.1:8080` | K | URL of the local server. |
| `PEAKD_ADFILTER_DIR` | `$XDG_DATA_HOME/shiny/adfilter` | K | Shared compiled ad-filter cache. |
| `PEAKD_DATA_DIR` | `~/.local/share/shiny/peakd` (sessions) | K | Webview data dir (per-user profile). |
| `PEAKD_DISPLAY_FILE` | — | K | Interface-scale config file. |
| `PEAKD_DISPLAY_RUNTIME_FILE` | — | K | Runtime display state. |
| `PEAKD_UI_SCALE` | — | K | Explicit interface scale. |
| `PEAKD_TOUCHPAD` | — | K | Touchpad device for gestures. |
| `PEAKD_IROH` | — | K | Iroh link to connect to. |
| `PEAKD_PROXY` | — | K | Upstream proxy override. |
| `PEAKD_TOUCHBAR` (`=0`) | — | K (mac) | Disable the Touch Bar. |
| `PEAKD_BIN` | — | K shell script | Path to the shell binary. |
| `PULSE_SERVER` | — | K | Pulse server override (usually unset). |
| `PROBE_DEBUG` | — | K | Verbose probing. |
| `HOME` / `XDG_DATA_HOME` | — | K | Standard dirs. |

## Auth helper build/systemd

`LISTEN_FDS` / `LISTEN_PID` are set by systemd socket activation for
`shiny-auth`. See [`crates/shiny-auth`](../../crates/shiny-auth).

---

## Notes

- Empty `OLLAMA_URL`, `WHISPER_URL`, `QWEN_TTS_URL` are treated as unset.
- Booleans accept `true`/`false`, falling back to the default on anything else;
  ports fall back to the default on parse failure.
- The server maps its config to each launcher's `*_URL`/port env when spawning
  it, so `.env` is the single place to change ports.
- Mutating host state is gated to loopback independent of these variables; see
  [host overview](../host/README.md).
