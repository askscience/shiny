# Environment variables

This is the exhaustive reference. The `.env` file in the repository root is
loaded by [`dotenvy`](../../src/main.rs); real environment variables still take
precedence. Core values are parsed in
[`Config::from_env`](../../src/config.rs); the rest are read by the launchers,
the kiosk shell and the auth helper.

**Legend:** ✓ = read by the server core; L = sidecar launcher; K = kiosk shell
(`liteaura`/`liteaura-mac`); A = `shiny-auth` helper; G = greeter session launcher
(`scripts/shiny-greeter`).

---

## Server & core

| Variable | Default | Read by | Meaning |
|---|---|---|---|
| `SERVER_HOST` | `127.0.0.1` | ✓ | HTTP bind address. Loopback by default; set `0.0.0.0` to serve the LAN. |
| `SHINY_ALLOWED_HOSTS` | *(empty)* | `src/api/remote.rs` | Extra host names (comma-separated) accepted in `Host`, for a custom reverse-proxy domain. IP addresses, `localhost`, single-label and `.local` names and `*.ts.net` are always accepted. |
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
| `MAP_TILES_CACHE_DIR` | `~/.cache/shiny/map-tiles` |  | Disk cache for the map's OpenStreetMap tiles. |
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
| `Z_IMAGE_MODELS_DIR` | `data/z-image-models` | ✓ L | Z-Image Turbo quants + shared encoder + VAE. |
| `QWEN_IMAGE_MODELS_DIR` | `data/qwen-image-models` | ✓ L | Qwen-Image 2.1 Turbo quants + shared Qwen3-VL encoder, VAE, TAE, vision tower. |
| `Z_IMAGE_SD_CLI` | auto-discovered | ✓ | Explicit `sd-cli` (stable-diffusion.cpp) binary. |
| `Z_IMAGE_CACHEDIT` | `0.30` | ✓ | Cache-DiT threshold; `0`/`off` disables the cache. |
| `YUE2_MODELS_DIR` | `data/yue2-models` | ✓ L | YuE2 song weights + Oobleck VAE + v9 decoder companion + tokenizer. |
| `YUE2_ENGINE` | auto-discovered | ✓ | Explicit `ace-server` (HOT-Step-CPP) binary for song generation. |

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

## Kiosk shell (`liteaura` / `liteaura-mac`)

| Variable | Default | Read by | Meaning |
|---|---|---|---|
| `LITEAURA_APP_ORIGIN` | `http://127.0.0.1:8080` | K | URL of the local server. |
| `LITEAURA_ADFILTER_DIR` | `$XDG_DATA_HOME/shiny/adfilter` | K | Shared compiled ad-filter cache. |
| `LITEAURA_DATA_DIR` | `~/.local/share/shiny/liteaura` (sessions) | K | Webview data dir (per-user profile). |
| `LITEAURA_DISPLAY_FILE` | — | K | Interface-scale config file. |
| `LITEAURA_DISPLAY_RUNTIME_FILE` | — | K | Runtime display state. |
| `LITEAURA_UI_SCALE` | — | K | Explicit interface scale. |
| `LITEAURA_TOUCHPAD` | — | K | Touchpad device for gestures. |
| `LITEAURA_IROH` | — | K | Iroh link to connect to. |
| `LITEAURA_PROXY` | — | K | Upstream proxy override. |
| `LITEAURA_TOUCHBAR` (`=0`) | — | K (mac) | Disable the Touch Bar. |
| `LITEAURA_BIN` | — | K shell script | Path to the shell binary. |
| `PULSE_SERVER` | — | K | Pulse server override (usually unset). |
| `PROBE_DEBUG` | — | K | Verbose probing. |
| `HOME` / `XDG_DATA_HOME` | — | K | Standard dirs. |

## Plugins, updates and the installer

Read directly by the code named, not through `Config`.

| Variable | Default | Read by | Meaning |
|---|---|---|---|
| `PLUGIN_WATCH_SECS` | `10` | `src/application/plugin_watch.rs` | How often the server looks for plugins changed on disk. |
| `SHINY_RUSTUP_DIST_URL` | `https://static.rust-lang.org/rustup/dist` | `src/services/plugin_dev/rust_setup.rs` | Where `rustup-init` and its checksum are downloaded from when the plugin builder installs the Rust compiler. |
| `SHINY_PLUGIN_SDK_DIR` | — | `src/services/plugin_dev/rust.rs` | The plugin SDK bundle Rust plugins are built against: a directory with `crates/shiny-plugin-sdk`, `crates/shiny-plugin-protocol`, `vendor/libsqlite3-sys` and `Cargo.lock`. Looked for next in `/usr/share/shiny/plugin-sdk`, then in the directory the server runs in. See [the plugin builder](../plugins/builder.md). |
| `SHINY_LOOKS_DIR` | `/var/lib/shiny/looks` | `src/services/greeter_look.rs` | Where each account publishes its look for the login screen. |
| `LITEAURA_POINTER_FILE` | `~/.config/liteaura/pointer.json` | `src/services/pointer.rs` | Mouse and touchpad settings file. |
| `LITEAURA_AI_TIER` | — | `src/services/ai_switch.rs` | Set by the session: `none` / `off` stop the client fetching speech models on its own. |
| `SHINY_BOOT_TOKEN` | — | K | The kiosk auto-login token, handed to the shell through the environment (never argv). |
| `SEARXNG_URL` | — | Browser plugin | A SearXNG instance for the Browser's searches. |
| `SHINY_GOKAPI_DIR` | `~/.shiny/gokapi` | Files plugin | State directory of the file-sharing server. |
| `SHINY_GOKAPI_BUNDLE_DIR` | `/usr/libexec/shiny/gokapi` | Files plugin | Where the bundled file-sharing binaries are installed. |
| `LITEAURA_CAPTURE_RENDER` | — | Capture plugin | Path to `liteaura-capture-render` (default: next to the server, then `PATH`). |
| `OLLAMA_BIN` | — | Updates plugin | Path to the `ollama` binary for a non-standard install. |
| `OLLAMA_INSTALL_URL` | Ollama's install script | Updates plugin | Script URL used to update Ollama. |
| `OLLAMA_UPDATE_CMD` | — | Updates plugin | Replaces the whole Ollama update command. **Run as root through `sh -c`**: set it only in an environment you control. |
| `SHINY_LIVE_INSTALLER` (`=1`) | — | Installer plugin | Marks a live session; the installer is inert without it. |
| `SHINY_INSTALLER_SOCKET` | `/run/shiny/installer.sock` | Installer plugin | Socket of the root installer helper. |
| `LITEAURA_ENGINE_SQUASHFS` | `/run/live/medium/live/filesystem.squashfs` | installer helper | System image the installer copies. |
| `LITEAURA_ENGINE_MEDIUM` | `/run/live/medium/live` | installer helper | The live medium's directory. |
| `LITEAURA_ENGINE_SOURCE` | — | installer helper | An already-mounted source tree to copy instead of the squashfs. |

`ADFILTER_DIR`, `SHINY_CRAFT_CLI` and the `*_TEST_*` / `*_DEBUG` variables are
read only by examples and tests.

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
