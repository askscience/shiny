# Kiosk shell (`peakd` / `peakd-mac`)

The web UI is normally rendered by a native shell rather than a browser tab —
this is what gives Shiny real child browser web views, trackpad gestures, the
macOS Touch Bar and a kiosk that survives crashes.

Source: [`crates/peakd/`](../../crates/peakd),
[`crates/peakd-mac/`](../../crates/peakd-mac),
[`crates/shiny-server-mode/`](../../crates/shiny-server-mode),
[`scripts/peakd-kiosk.sh`](../../scripts/peakd-kiosk.sh),
[`scripts/shiny-session`](../../scripts/shiny-session).

---

## `peakd` (Linux)

A Qt 6 + QtWebEngine window (`QT_QPA_PLATFORM=xcb`). It renders the app and owns
the **Browser** plugin's tabs as real child web views at the page's true origin.

| Module | Role |
|---|---|
| `main.rs` | Window, args, event loop. |
| `config.rs` | Command-line/env config (`PEAKD_*`). |
| `display.rs` | Applies the interface scale the server persists (`/api/display`). |
| `browse.rs` | Child web views for Browser tabs; request interceptor hookup. |
| `downloads.rs` | Download manager plumbing. |
| `filter.rs` | Loads the compiled ad-filter cache the server shares and asks the engine per request. |
| `gestures.rs` | Reads the touchpad evdev stream and dispatches `trackpad:gesture`. |
| `host.rs` | Host shims (page/app integration). |
| `shim.rs` | Compatibility shims. |
| `bench.rs` | Benchmarks. |

### Startup

[`scripts/peakd-kiosk.sh`](../../scripts/peakd-kiosk.sh) is the X11 session:
pins Qt to X11, sets `PEAKD_APP_ORIGIN` and `PEAKD_ADFILTER_DIR`, disables X
blanking (`xset s off`, `-dpms`), starts matchbox (no titlebar), then runs
`peakd` under a supervisor. The supervisor distinguishes a deliberate quit
(status 0) from a crash (non-zero → restart) because `xinit` does not propagate
the client's status. Status `42` is the **server-mode switch** (see
[remote access](remote-access.md)).

The shell resolves `peakd` via `PEAKD_BIN`, then `/usr/local/bin/peakd` /
`/usr/bin/peakd`, then `PATH`.

### Environment

| Variable | Meaning |
|---|---|
| `PEAKD_APP_ORIGIN` | URL of the local server (default `http://127.0.0.1:8080`). |
| `PEAKD_ADFILTER_DIR` | Shared compiled ad-filter cache (per user). |
| `PEAKD_TOUCHPAD` | Override the touchpad device (`/dev/input/eventN`). |
| `PEAKD_BIN` | Path to the shell binary. |
| `QT_QPA_PLATFORM` | Pinned to `xcb`. |

### Browser child web views

The Browser plugin's tabs are **not** iframes. Each tab is a native child web
view at the page's true origin (`browse.rs`), so anti-bot challenges
(Cloudflare) pass. Ad blocking runs **in-process**: the server compiles
EasyList/EasyPrivacy into a cache the shell restores, and a
`QWebEngineUrlRequestInterceptor` asks the engine per request. There is no
filter proxy in the page path.

---

## `peakd-mac` (macOS)

A WKWebView shell packaged as `Peakd.app` by
[`scripts/bundle-app.sh`](../../scripts/bundle-app.sh). The `.app` bundle is
required for macOS to grant microphone/camera access (`getUserMedia` needs
`NSMicrophoneUsageDescription` and a real bundle TCC can key a grant to).

| Module | Role |
|---|---|
| `main.rs` | App + window. |
| `browse.rs` | Child web views (WBWebView). |
| `touchbar.rs` | Native `NSTouchBar`; buttons evaluate `touchbar:action`. |
| `display.rs` | Display scale. |
| `config.rs` | CLI/env config. |

Flags/env: `PEAKD_TOUCHBAR=0` or `--no-touchbar` disables the bar. macOS has
native multi-touch gestures, so it does not use the evdev reader.

---

## Server mode

[`shiny-server-mode`](../../crates/shiny-server-mode) is the kiosk window shown
while Iroh server mode is on: it displays the link + controls and exits 0 on
Stop. [`scripts/shiny-session`](../../scripts/shiny-session) supervises the
swap.

---

## Per-user session

[`scripts/shiny-session`](../../scripts/shiny-session) is the LightDM session
script (installed by
[`scripts/install-linux-session.sh`](../../scripts/install-linux-session.sh)):

1. Writes `SERVER_PORT` (`8080 + uid − 1000`) to `~/.config/shiny/env`.
2. Starts the per-user `shiny.service` and the speech sidecars.
3. Waits (up to ~30 s) for `/api/voice/languages` to answer.
4. Auto-logs-in the kiosk with the loopback session token (or opens the plain
   URL if the token file is missing).
5. Starts matchbox and loops between `peakd` and the server-mode window.

Quitting the shell (`Alt`+`Q`) ends the session and returns to the greeter.

---

## Related

- [Remote access](remote-access.md)
- [Display & input](../host/display-input.md) (scale, Touch Bar, gestures)
- [Multi-user Linux](multi-user-linux.md)
- [T2 Mac](t2-mac.md)
