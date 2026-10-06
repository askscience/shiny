# Crate map

The workspace is defined in [`Cargo.toml`](../../Cargo.toml). It contains the
core binary, the plugin SDK and its codecs, the native shells, the auth helper
and the remote-access crates.

## Published/binary crates

| Crate | Path | Type | Role |
|---|---|---|---|
| `shiny` | [`src/`](../../src) | bin + lib | The server: HTTP/SSE API, agent loop, auth, voice, host panels, plugin host, static web UI. |
| `shiny-plugin-sdk` | [`crates/shiny-plugin-sdk`](../../crates/shiny-plugin-sdk) | lib | The public plugin API: `Plugin`/`Tool` traits, manifest, routes, crons, outcomes, notifications, artifacts, navigation, migrations, `PluginCtx`, canonical clients, and the ODT/ODS/ODP codecs. Both the binary and every plugin link it. |
| `shiny-filter` | [`crates/shiny-filter`](../../crates/shiny-filter) | lib | The Browser's in-process filtering engine: compiles EasyList/EasyPrivacy, classifies and rewrites requests, exposes metrics. |
| `shiny-filter-core` | [`crates/shiny-filter-core`](../../crates/shiny-filter-core) | lib | Shared filter-list parsing/classification logic used by `shiny-filter`. |
| `peakd` | [`crates/peakd`](../../crates/peakd) | bin | Linux kiosk shell (Qt 6 + QtWebEngine): window, display scaling, child browser web views, downloads, trackpad gestures, request interceptor, host shims. |
| `peakd-mac` | [`crates/peakd-mac`](../../crates/peakd-mac) | bin | macOS shell (WKWebView + `NSTouchBar`). Packaged as `Peakd.app` by [`scripts/bundle-app.sh`](../../scripts/bundle-app.sh). |
| `shiny-auth` | [`crates/shiny-auth`](../../crates/shiny-auth) | bin | Root-owned, socket-activated, **verify-only** PAM helper (Linux-user mode). |
| `shiny-iroh-client` | [`crates/shiny-iroh-client`](../../crates/shiny-iroh-client) | bin + lib | Local proxy that dials a remote Iroh endpoint and serves the app on a loopback port. |
| `shiny-iroh-proto` | [`crates/shiny-iroh-proto`](../../crates/shiny-iroh-proto) | lib | Shared protocol types for Iroh remote access (built behind the `iroh` feature). |
| `shiny-server-mode` | [`crates/shiny-server-mode`](../../crates/shiny-server-mode) | bin | The kiosk window shown while the server is in Iroh server mode (link + controls). |

## Plugin crates

Each bundled plugin is its own `cdylib`/`rlib` crate listed in the workspace
`members`. They all depend only on `shiny-plugin-sdk` plus the axum/tokio/serde
ecosystem. See the plugin's `docs/` folder and
[plugin architecture](../plugins/architecture.md).

| Crate (dir) | Library artifact | Category |
|---|---|---|
| `plugins/browser` | `libshiny_browser_plugin` | Web |
| `plugins/files` | `libshiny_files_plugin` | System |
| `plugins/hello` | `libshiny_hello_plugin` | System |
| `plugins/keyboard` | `libshiny_keyboard_plugin` | System |
| `plugins/terminal` | `libshiny_terminal_plugin` | System |
| `plugins/traveler` | `libshiny_traveler_plugin` | Travel |
| `plugins/word` | `libshiny_word_plugin` | Office |
| `plugins/calc` | `libshiny_calc_plugin` | Office |
| `plugins/impress` | `libshiny_impress_plugin` | Office |
| `plugins/pdf` | `libshiny_pdf_plugin` | Office |
| `plugins/mail` | `libshiny_mail_plugin` | Office |
| `plugins/calendar` | `libshiny_calendar_plugin` | Office |
| `plugins/calculator` | `libshiny_calculator_plugin` | Office |
| `plugins/image` | `libshiny_image_plugin` | Media |
| `plugins/studio` | `libshiny_studio_plugin` | Media |
| `plugins/filmcraft` | `libshiny_filmcraft_plugin` | Media |
| `plugins/radio` | `libshiny_radio_plugin` | Media |
| `plugins/youtube` | `libshiny_youtube_plugin` | Media |

## Inherited dependency highlights

| Concern | Crate |
|---|---|
| HTTP server | `axum` 0.7 (router hot-swapped via `arc-swap`) |
| Async runtime | `tokio` 1 |
| Database | `sqlx` 0.8 + system SQLite (`libsqlite3-sys`, vendored shim) |
| HTTP client | `reqwest` 0.12 (rustls); `wreq` 6 (BoringSSL, Chrome TLS/JA3) for the Browser |
| Serialization | `serde` / `serde_json` / `toml` |
| Logging | `tracing` + `tracing-subscriber` (stdout + file tee) |
| Plugin loading | `libloading` (dlopen) |
| Archives | `zip`, `flate2`, `tar` |
| Config | `dotenvy` |
| Office codecs | `roxmltree` + `zip` (ODT/ODS/ODP) |
| Studio DSP | `fundsp`, `rustysynth` |
| Host (Linux) | `nmrs` (NetworkManager), `zbus` (BlueZ/logind), `if-addrs`, `libc` |
| Remote (optional) | `iroh` 1.2 + `shiny-iroh-proto` behind the `iroh` feature |
| QR codes | `qrcode` |

## Build profiles

[`Cargo.toml`](../../Cargo.toml) sets a few deliberate profile decisions:

- **`[profile.release]`** — thin LTO + one codegen unit so the Studio DSP
  engine's per-sample calls inline (~2× faster renders).
- **`[profile.dev.package.fundsp]`** — `opt-level = 2` even in dev builds,
  because fundsp's per-sample DSP is unusably slow at opt-level 0.
- **`[patch.crates-io]`** — `libsqlite3-sys` is patched to the vendored shim in
  [`vendor/libsqlite3-sys`](../../vendor) so every dlopen'd plugin links the one
  system SQLite instead of bundling its own (multiple bundled copies segfault).
