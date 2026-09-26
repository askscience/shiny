# Kiosk shell: WebKitGTK → Qt 6 + QtWebEngine — migration plan

Status: **implemented** — the Linux kiosk runs `peakd` (Qt 6.8 LTS + Qt WebEngine);
`crates/peakd-mac` is now the macOS-only wry shell. Verified headlessly: app boot, real
Browser plugin, engine/cookie sharing and persistence, 200 % child alignment, live scale
changes, Cloudflare, downloads, benchmark, plain-Chromium UA. Remaining user checks on the
kiosk: mic through PipeWire, `Meta+Q`, context menu/file dialogs by hand, GPU benchmark
(the orb is slated to become 2D).
Goal: replace the Linux kiosk shell (`crates/peakd-mac`: `tao` + `wry` + WebKitGTK) with a
Qt 6.8 LTS + Qt WebEngine shell, while freezing the shell↔app contract so
`plugins/browser` and `web/` need no changes. macOS keeps the `wry`/WKWebView shell.

Related: [`PLAN-linux-users.md`](./PLAN-linux-users.md) (per-user session that starts
`peakd`), [`PLAN-iroh-remote.md`](./PLAN-iroh-remote.md) (`--iroh` mode, exit 42).

## What this migration is *not*

- **No UI rewrite.** No HTML/CSS/JS is ported to Qt widgets or QML. The app (`web/`,
  every `plugins/*/web/`) keeps being served by `shiny` and rendered by the web engine.
  Qt replaces **WebKitGTK as the engine host**, nothing else.
- **No new windows per plugin.** Plugin "windows" are DOM tiles inside the single kiosk
  page, before and after; that does not change. The only extra native surfaces are the
  Browser plugin's child views, and they stay web-content views (`QWebEngineView`) — not
  Qt UI.
- **No server change.** `src/`, the API, the DB, the plugin SDK, `web/` and `plugins/**`
  are untouched.
- **Touched surfaces:** the new `crates/peakd` (Qt) shell, `crates/peakd-mac` (Linux GTK path
  retired, macOS shell kept), the session scripts, and docs.

Today:  `web app (HTML/CSS/JS)` → WebKitGTK ← `peakd` (tao window, wry, bridge, evdev)
After:  `web app (HTML/CSS/JS)` → QtWebEngine ← `peakd` (Qt window, C++ shim, bridge, evdev)

The C++/Qt code is a thin shim (QApplication, QWebEngineView/QWebEnginePage subclass,
QWebChannel IPC, ~500–900 lines). Rust keeps the policy: config, command protocol, queues,
events, scale, gestures, benchmark, Iroh.

## 0. Why Qt 6 + Qt WebEngine (and not "Qt WebKit")

Qt 6 **has no WebKit**. `qtwebkit/qtwebkit` is the community revival of Qt WebKit; its
last release is **5.212.0-alpha4** (March 2020), built against Qt 5.14/5.15, and there is
no Qt 6 port. Its own release notes warn it is based on an old WebKit revision with known
unpatched vulnerabilities. It is also not in Debian 13 (removed after Debian 12), while
`qt6-webengine-dev` is (`6.8.2`, Chromium 122 base, Debian-patched). Most decisively, it
cannot run this repo's own front end:

| Evidence | Consequence under QtWebKit 5.212 |
|---|---|
| `web/js/` has **382** `?.` and **20** `??` sites (`plugins/browser/web/plugin.js:148` is one of the first to load) | SyntaxError at parse time — the app does not boot |
| `web/js/voice.js:717,1005` call `navigator.mediaDevices.getUserMedia` | `QWebPage::Feature` only has `Notifications`/`Geolocation`; no mic/camera capture exists |
| ES modules + dynamic `import()` across `web/js` (`coreWindows.js`, `map.js`, …) | Partial-at-best module support in that engine vintage |
| Modern TLS/HTTP/2, CORS/cookie behaviour, process sandbox | Predates most of it; no sandbox at all |

So "latest Qt + Qt WebKit" is a contradiction. Qt 6's web engine **is** Qt WebEngine
(Chromium). If the hard requirement were *the WebKit engine family*, the only maintained
routes are WebKitGTK (today) or WPE — there is no Qt route. This plan takes "latest Qt
with a maintained engine" as the requirement and targets **Qt 6.8 LTS + QtWebEngine
Widgets**.

### What the migration buys (from the current code, specifically)

| Today | Where it hurts | With QtWebEngine |
|---|---|---|
| WebKitGTK strips Super/Command before the DOM (`metaKey` false) | `install_exit_shortcut` GTK widget hack in `crates/peakd-mac/src/main.rs:137` | Chromium delivers `metaKey` to the DOM; the JS shortcut works everywhere (shim keeps a shortcut as belt-and-braces) |
| GDK turns any X error fatal; `BadWindow` race aborts the kiosk | `install_x_error_guard` (`main.rs:191`) | Qt/xcb has no equivalent fatal handler; guard likely deleted (verify) |
| No trackpad gesture delivery to the page | `crates/peakd-mac/src/gestures.rs` evdev reader | Chromium handles scroll/pinch; three-finger workspace swipes still come from the evdev reader (unchanged) |
| No request interception; filter had to be a proxy | `crates/shiny-filter`, stale docs | `QWebEngineUrlRequestInterceptor` exists if in-process filtering ever returns; filtering stays server-side for now |
| Permission story is a wry default | `main.rs:417`, `browse.rs:375` | `featurePermissionRequested` / `QWebEnginePermission` (6.8) with a real path to persist grants |
| Devtools = one wry flag | `cfg.devtools` | DevTools page or `QTWEBENGINE_REMOTE_DEBUGGING` port |
| Engine age vs anti-bot/Cloudflare | Browser plugin's whole reason to be native | Chromium is the largest desktop browser; Cloudflare sees a common client |

Costs to accept: ~2–4× resident memory, GPU/compositing questions on the T2 hybrid
graphics under matchbox, asynchronous IPC plumbing (§3.4), Chromium sandbox requirements.

## 1. What exists today (findings)

### 1.1 The shell — `crates/peakd-mac` (2,400 lines + `examples/probe.rs`)

| File | Responsibility | Fate in the migration |
|---|---|---|
| `src/main.rs` (678) | CLI/env config, loopback wait, tao window, wry main webview (UA, nav, permissions, init script, IPC), proxy, scale apply + watch, GTK exit shortcut, X error guard, event loop pumping views/touchbar/gestures, exit code 42 on server mode | Linux GTK path retired; macOS shell keeps the rest |
| `src/browse.rs` (457) | `ViewBus` queues, `Command` JSON protocol, `CssRect`→native rect, child `WebView` lifecycle, events to `window.__peakdViewEvent` | Protocol/state stay in Rust; the `WebView` layer becomes a Qt shim |
| `src/bench.rs` (352) | `BenchStep` user events, JS injection, `evaluate_script_with_callback`, window `Moved` observation, JSON report + diagnosis | Ported; report shape frozen |
| `src/display.rs` (208) | Choice file read/watch, DPI→auto scale via GDK monitors, runtime file | Ported; DPI via `QScreen` |
| `src/gestures.rs` (620) | evdev multi-touch → 3-finger swipes → `trackpad:gesture` | Unchanged (toolkit-independent) except the evaluate hook |
| `src/touchbar.rs` (271) | macOS `NSTouchBar` | Untouched (macOS-only) |
| `examples/probe.rs` | wry-based diagnostics | Rewritten as the Qt spike |

### 1.2 The contract the shell must keep (frozen)

- **Page → shell** over `window.ipc.postMessage`:
  - `peakd:view:<json>` commands: `open` `{id,url,rect,visible}`, `navigate`, `setBounds`,
    `setVisible`, `back`, `forward`, `reload`, `close`, `focus`;
  - `peakd:exit`, `peakd:server-mode`.
- **Shell → page** on the main view: `window.__peakdViewEvent({id,type,…})` with types
  `url`, `title`, `load` (`phase: started|finished`), `new-window`.
- **Exit codes**: `0` deliberate quit, `42` switch to server mode, non-zero = crash and the
  session supervisor restarts (`scripts/peakd-kiosk.sh`, `scripts/shiny-session`).
- **Files**: `~/.config/peakd/display.json` (choice), `~/.cache/peakd/display.json`
  (applied value), CLI/env list in `config.rs` (`--url`, `--scale`, `--proxy`, `--devtools`,
  `--benchmark*`, `--iroh`, `--offline`, `--no-touchbar`, …).
- **Benchmark JSON** keys (`fps`, `worstFrameMs`, `windowMoves`, `verdict`, `diagnosis`, …).
- **Injections**: `EXIT_SHORTCUT_JS` on every main-frame load and in child views.
- **`window.ipc` must exist synchronously** when page modules evaluate: the browser plugin
  computes `nativeAvailable` at import (`plugins/browser/web/plugin.js:38`).

### 1.3 The browser plugin

`plugins/browser/web/plugin.js` (971 lines) owns presentation only: tab strip, toolbar,
home shelf, bounds polling (`ResizeObserver` + 200 ms timer), and the native commands
above. It does not know WebKit. Its server side (`src/routes.rs`, `sessions.rs`,
`news.rs`, `history.rs`, `fetch.rs`, `preview.rs`) is toolkit-independent. **It is the
acceptance test for the contract:** if the Qt shell exposes `window.ipc` and
`window.__peakdViewEvent` with the same shapes, this plugin is untouched.

### 1.4 Session and scripts

`scripts/peakd-kiosk.sh` (xinit path) and `scripts/shiny-session` (LightDM path) export
`XDG_SESSION_TYPE=x11`, pin `GDK_BACKEND=x11`, wait for the app, run matchbox, supervise
`peakd`, and treat `42` as a window switch. `scripts/install-linux-session.sh` installs the
session with a hard-coded `PEAKD_BIN`. `scripts/install-touchpad-gestures.sh` installs the
udev `uaccess` rule the evdev reader needs. All of these are migration touch points.

## 2. Decisions locked

| Question | Decision |
|---|---|
| Toolkit / engine | **Qt 6.8 LTS Widgets + QtWebEngine** (Debian 13 `qt6-webengine-dev` 6.8.2, Chromium 122 base + Debian patches) |
| Scope | **Linux kiosk first.** macOS keeps `crates/peakd-mac` (wry/WKWebView + Touch Bar) |
| Shell language | **Rust policy + thin C++ Qt shim** (see §3.3); no pure-C++ rewrite, no Python shell |
| Compatibility surface | **Frozen contract** (§1.2); `plugins/browser` and `web/` unchanged |
| Rollout | Linux runs `peakd` only; `crates/peakd-mac` is the macOS shell (the Linux GTK path is retired) |
| Filtering | Stays server-side; no proxy revival (`--proxy` remains a debug flag) |
| UI scale | Run Qt/Chromium at scale factor 1 and keep the existing **page-zoom** policy and choice files (least behavioural change; revisit with the benchmark) |

## 3. Target architecture

### 3.1 Process and threading model

```
 peakd (one Qt GUI process, Rust main)
 ├── Qt main thread: QApplication, QMainWindow, main QWebEngineView,
 │    child QWebEngineViews, QWebChannel, QTimer pump (100 ms)
 ├── QtWebEngineProcess (Chromium renderers/GPU/network, sandboxed)
 ├── Rust "peakd-gestures" thread (evdev, unchanged)
 └── Rust "peakd-display-watch" thread (mtime poll, unchanged)
```

Rust threads only push into mutex queues (as today). The Qt main thread drains them on a
100 ms `QTimer` (the existing event loop already polls at 100 ms,
`main.rs:544`) plus a wake function for immediate dispatch. All Qt objects are touched only
on the Qt main thread — the same invariant the code already respects for `WebView`.

### 3.2 Crate layout (as built)

- **`crates/peakd` (new, Linux)**: `config.rs` (CLI/env; no proxy/offline/touchbar
  leftovers), `display.rs` (choice/auto/runtime; `--scale` wins at startup, file changes
  after), `gestures.rs` (evdev three-finger swipes, ported with its tests), `browse.rs`
  (`Command`, `CssRect`, `ViewBus`, `Views` state machine behind a `ViewHost` trait),
  `host.rs` (the Qt host), `bench.rs` (schedule + report), `shim.rs` +
  `shim/peakd.{h,cpp}` + `shim/ipc_bridge.h`, `build.rs` (pkg-config Qt 6, `cc`, one
  `moc` run). Self-contained: `cargo test -p peakd` and `cargo build -p peakd`.
- **`crates/peakd-mac` (existing)**: the macOS wry/WKWebView shell (Touch Bar). Its Linux
  GTK path was retired here; on Linux it compiles to a stub that points at `peakd`.
- A shared `peakd-core` crate was planned but is unnecessary now that each platform has
  exactly one shell; the two config/display copies serve different engines.

### 3.3 The Rust ↔ Qt boundary

Qt lives behind a shim that only exposes opaque handles and plain data:

**C++ → Rust callbacks** (all on the Qt main thread):
`ipc(body)`, `view(id, kind, payload)` (`load`/`title`/`url`/`new-window`/`crashed`, plus
`window`/`move` for the benchmark), `pump()` (the 100 ms tick), `js_result(id, value)`.

**Rust → C++ calls**: `peakd_qt_run(url, data_dir, probe, callbacks…) -> exit_code`
(creates `QApplication`, the window, the named profile, runs `exec()`),
`inject_script(name, source)`, `set_window(title, w, h)`, `quit()`,
`main_load(url)`, `main_zoom(factor)`, `main_run_js(script, callback_id)`,
`screen_dpi()`, `screen_size()`,
`view_create/navigate/bounds/visible/back/forward/reload/focus/close(id, …)`.

Binding: plain **C ABI + `cc` + one `moc` run** in `build.rs` (the `IpcBridge`
`Q_INVOKABLE` is the only meta-object needed; everything else is virtual overrides and
lambda connections, which need no moc). `cxx`/`cxx-qt` were not needed at this size. No
Qt type crosses into Rust beyond opaque handles.

### 3.4 IPC — the one place that must be got exactly right

`QWebChannel` is asynchronous (handshake over `qt.webChannelTransport`). Injecting it with
a `QWebEngineScript` at `DocumentCreation` is not enough on its own: plugin modules read
`window.ipc` during import. So the injected script defines `window.ipc` **immediately**
with a queue:

1. DocumentCreation script injects `qwebchannel.js` + a shim that sets
   `window.ipc.postMessage = (s) => queue.push(s)`; when the channel opens,
   `postMessage` is re-pointed at `channel.objects.ipc.postMessage` and the queue is
   flushed in order.
2. C++ `IpcBridge : QObject` has `Q_INVOKABLE void postMessage(QString)` → `ipc(body)`
   callback into Rust. Rust keeps all routing logic (`ViewBus::handle_ipc`, exit,
   server-mode) exactly as today.
3. The same shim is injected into child views (they only ever send `peakd:exit`).

Phase 0 must assert `window.ipc` is a function on the first line of app JS, and that a
`peakd:view:open` sent during boot arrives.

### 3.5 Profiles, user agent, storage

- **One explicit, persistent profile** named `peakd`, shared by the main view and every
  Browser-plugin child view: same cookie jar, cache and UA. Verified end-to-end — two
  Browser tabs share cookies, and a `max-age` cookie survives a shell restart
  (`data/peakd/qtwebengine/storage/Cookies`). A separate app/browsing profile was
  considered and dropped: one shared jar matches the WebKitGTK shell and is what "the
  plugin Browser shares the engine" means. Storage under
  `/…/data/peakd/qtwebengine/{storage,cache}`.
- **Plain Chromium UA for the whole profile** — the `QtWebEngine/…` token is stripped while
  keeping `Chrome/<engine version>` truthful. The app and the open web present the same
  honest engine string; the old `Peakd/… Safari` UA is gone, since spoofing Safari on
  Chromium's TLS/JS fingerprint is exactly what anti-bot systems flag. Verified in the app
  and in a Browser tab (`Chrome/122.0.6261.171`), and Cloudflare still passes under it.

### 3.6 Coordinates (child views over the main view)

Qt gives this for free compared to wry: a child `QWebEngineView` is a plain `QWidget`
child of the main view, so `setGeometry()` takes **Qt logical pixels**.

- Qt/Chromium run at device scale factor 1 (decision §2), so
  `logical_rect = css_rect × zoomFactor` — **no `devicePixelRatio` multiplication** (the
  current `CssRect::to_rect`, `browse.rs:81`, is deleted along with `wry::Rect`).
- The 200 % kiosk zoom is `main_view.zoomFactor()`, applied once.
- Child views are parented to the main view; `hide()/show()` replaces
  `set_visible`, `setFocus()` replaces `focus()`.
- `browse.rs` keeps its clamped/rounded parsing and its tests.

## 4. API mapping (wry/tao/WebKitGTK → Qt 6 + QtWebEngine)

| Today | Qt 6 replacement |
|---|---|
| `tao::window::WindowBuilder` / event loop | `QApplication` + `QMainWindow` + `QTimer` pump (shim) |
| `wry::WebViewBuilder` main view | `QWebEngineView` + a `QWebEnginePage` subclass |
| `build_gtk(default_vbox)` | plain widget parenting (`setCentralWidget`) |
| `build_as_child` + `set_bounds` | `new QWebEngineView(parent)` + `setGeometry` |
| `window.ipc.postMessage` | `QWebChannel` + synchronous queuing shim (§3.4) |
| `with_initialization_script` | `QWebEngineScript` (`DocumentCreation`, `MainWorld`) in the page's script collection |
| `evaluate_script` | `QWebEnginePage::runJavaScript(script)` |
| `evaluate_script_with_callback` | `runJavaScript(script, cb)` (bench `/ `js_result`) |
| `with_navigation_handler` | `QWebEnginePage::acceptNavigationRequest` override |
| `with_new_window_req_handler` | `QWebEnginePage::createWindow` override (`JavascriptCanOpenWindows` on) |
| `with_permission_handler` (Mic/Camera) | `featurePermissionRequested` → `setFeaturePermission` (or `QWebEnginePermission` in 6.8) |
| `with_document_title_changed_handler` | `QWebEngineView::titleChanged` |
| `with_on_page_load_handler(Started/Finished)` | `loadStarted()` / `loadFinished(bool)` |
| `with_user_agent` | `QWebEngineProfile::setHttpUserAgent` |
| `with_visible` | `QWidget::setVisible` |
| `go_back/go_forward/reload` | `QWebEnginePage::triggerAction(Back/Forward/Reload)` (or history()) |
| `webview.zoom(f)` | `QWebEngineView::setZoomFactor(f)` |
| `with_devtools` | devtools page (`setDevToolsPage`) or `QTWEBENGINE_REMOTE_DEBUGGING=127.0.0.1:9222` |
| `ProxyConfig` / proxy env | `QNetworkProxy::setApplicationProxy` + a `QNetworkProxyFactory` that returns `DirectConnection` for loopback (replaces the `no_proxy` trick, `main.rs:641`) |
| GDK monitor DPI (`display.rs:118`) | `QScreen::physicalDotsPerInch()` / `logicalDotsPerInch()` |
| GTK key handler for Cmd/Alt+Q (`main.rs:137`) | `EXIT_SHORTCUT_JS` alone in Chromium + `QShortcut(Qt::ApplicationShortcut)` as backup |
| `XSetErrorHandler` BadWindow guard | delete; re-add only if Phase 0 shows an xcb abort |
| `tao::WindowEvent::Moved` (bench) | `QWidget::moveEvent` on the window → `window_moved` |

New behaviour that must be implemented because QtWebEngine does not do it implicitly:

- **Context menu** for child views: `createStandardContextMenu()` on `contextMenuEvent`
  (today WebKitGTK shows its own link/image menu).
- **File pickers**: `QWebEnginePage::chooseFiles` → `QFileDialog` (Files/Image plugins use
  `web/js/files.js` `pickFiles`; `webkitdirectory` maps to `UploadFolder`).
- **Downloads**: `QWebEngineProfile::downloadRequested` → save to `~/Downloads` and log
  (the app itself saves through `/api/files/upload`, so this is only for arbitrary sites).
- **Fullscreen**: `QWebEngineSettings::FullScreenSupportEnabled` + accept
  `fullScreenRequested`, or `web/js/fullscreen.js` breaks.
- **Autoplay**: `PlaybackRequiresUserGesture = false` (TTS, Radio, YouTube).
- **Renderer crashes**: `renderProcessTerminated` → reload the view / report the event.

## 5. Phases

### Phase 0 — Engine spike (go/no-go)

A `crates/peakd/examples/spike.rs` (or the probe example ported) that opens the real app
under matchbox, plus one child view over the Browser plugin's viewport.

Pass criteria, all on the T2 kiosk panel:
1. App boots: no JS parse errors (modern syntax), modules load, orb/tiles render.
2. `window.ipc` exists at first app script; `peakd:view:open` round-trips.
3. Child view overlays the plugin viewport within ~1 px at 200 % zoom, and follows tile
   drags (bounds poll).
4. `getUserMedia` audio works through PipeWire (`web/js/voice.js`) — **if this fails,
   stop.**
5. Audio playback works (TTS sidecar) with autoplay set false.
6. Cloudflare challenge passes in a child view; YouTube plays.
7. GPU compositing is healthy: `--benchmark` fps/memory within an agreed multiple of the
   GTK shell; fallback flags (`--disable-gpu`, `--use-gl=angle`,
   `--disable-gpu-compositing`) documented.
8. `Meta+Q` reaches the DOM (exit shortcut JS) and the shim shortcut.

Outcome: a written spike report appended to this plan (like the Iroh spike findings), or a
decision to stay on WebKitGTK.

### Phase 0 results — headless spike (2026-09-25)

Toolchain on Debian 13: Qt **6.8.2** / QtWebEngine **Chromium 122** linked from a Cargo
build (`crates/peakd`) via `pkg-config` + `cc` + one `moc` run (no `cxx-qt` needed at
this stage). `QApplication` + `QWebEngineView` + `QWebChannel` all work.

Verified headlessly (offscreen and under `xvfb-run`, `--disable-gpu`,
`QTWEBENGINE_DISABLE_SANDBOX=1`):

| Check | Result |
|---|---|
| Modern JS (optional chaining, `??`, dynamic `import()`) | **pass** — the app's syntax parses |
| Loopback secure context + `getUserMedia` API | **pass** — `secureContext:true`, `getUserMedia:true` on `http://127.0.0.1` (capture itself still to verify on the kiosk) |
| UA | `QtWebEngine/6.8.2 Chrome/122.0.6261.171 Safari/537.36` |
| `window.ipc` at document creation (queuing `QWebChannel` bootstrap) | **pass** — main *and* child views send before the handshake completes and all messages arrive in order |
| Contract round-trip `peakd:view:open` → Rust → child `QWebEngineView` → load events | **pass** (test page `spike:hello` / `spike:child-ready` / `t1 load finished`, 3/3) |
| Real app boots against a headless `shiny` (token login) | **pass** — `#app` visible, desktop + orb render; screenshot proof |
| WebGL / three.js orb | **fails only because the headless run has no GPU** — the app falls back to the 2D orb; must be re-verified on the kiosk |

Still to verify in the actual kiosk X session (Phase 0 exit criteria):
GPU/WebGL + benchmark parity, mic capture through PipeWire, Cloudflare in a child view,
child-view pixel alignment at 200 %, `Meta+Q` in the DOM.

Spike code: `crates/peakd/{build.rs,src/main.rs,shim/peakd.{h,cpp},shim/ipc_bridge.h}`.
Run: `QT_QPA_PLATFORM=xcb ./target/debug/peakd http://127.0.0.1:8080/`;
probe: `PEAKD_QT_PROBE=1 PEAKD_QT_SCREENSHOT=/tmp/app.png QT_QPA_PLATFORM=offscreen …`.

### Phases 1–3 results — shell core (2026-09-25)

`crates/peakd` is a shell now, not a spike:

| Area | State |
|---|---|
| Config/CLI/env (`--url`, `--scale`, `--devtools` via Chromium remote debugging, `--iroh` behind the `iroh` feature, display files) | done — own `config.rs`, proxy/offline/touchbar leftovers removed |
| Interface scale | done — page zoom applied on the pump, choice file polled each tick, runtime file written |
| `window.ipc` contract (`peakd:view:*`, `peakd:exit`, `peakd:server-mode` → 42) | done |
| Child views | done — create/navigate/setBounds/setVisible/back/forward/reload/focus/close, `new-window`, url/title/load events, main-view reload on renderer crash |
| Permissions | mic/camera granted, matching the wry shell |
| Gestures | evdev reader ported; `trackpad:gesture` dispatched on the pump (verified opening the T2 trackpad) |
| Benchmark | done — same `js/bench.js` instrumentation and identical report keys; schedule runs on the pump, window moves arrive as `window`/`move` view events |
| Context menu, `chooseFiles`, downloads | done — standard context menu (`QWebEngineView::createStandardContextMenu`), file/multi/folder/save pickers from the `accept` MIME types, downloads to `$HOME/Downloads` (verified headlessly; the pickers themselves need the kiosk) |
| Session scripts (`QT_QPA_PLATFORM=xcb`) | done — `peakd-kiosk.sh` and `shiny-session` run `peakd`; `install-linux-session.sh` substitutes the binary and builds it when missing |

Verified headlessly:
- `cargo test -p peakd` — 22/22 (protocol, rect mapping, host state machine, scale, gestures, benchmark diagnosis).
- `peakd:exit` → process status 0; `peakd:server-mode` → status 42.
- Real app under `xvfb-run`: boots authenticated, applies and records the scale in
  `~/.cache/peakd/display.json`, and runs with no console errors beyond the headless
  WebGL fallback (three.js → 2D orb).
- `--benchmark --benchmark-seconds 3 --benchmark-settle 10 --benchmark-output …`:
  settle → inject → measure → collect → pretty JSON report with the same keys as the
  GTK shell (`fps`, `windowMoves`, `verdict`, `diagnosis`, …); 60 fps idle under Xvfb's
  software rendering.
- Download: a `download` link saves the file under `$HOME/Downloads` and logs
  completion, then `peakd:exit` leaves with status 0.
- Real Browser plugin (driven with the `PEAKD_QT_EVAL` debug hook): activating the
  window, navigating a tab and reporting its title through `__peakdViewEvent` all work;
  two tabs share the cookie jar (tab 2 reads a cookie set by tab 1), and a `max-age`
  cookie survives a shell restart — the named `peakd` profile is shared and persistent.
- Child-view alignment at 200 % (`--scale 2`): the Browser element measures
  618×116 CSS px at `devicePixelRatio` 2, the child view receives 1236×232 logical px,
  and the child page reports exactly `vp=1236x232@1` — `css × dpr`, no rounding drift.
  (The `--scale` flag was parsed but not applied before this check; fixed.)
- Live scale change: writing `~/.config/peakd/display.json` while the shell runs
  re-zooms within a pump tick (`interface scale 100%` → `150%`) and updates the runtime
  file — no restart.
- Cloudflare: `https://nowsecure.nl/` resolves to the real site in a Browser tab
  (`title nowsecure.nl`, address bar intact); the only console error is the site's own
  JS, none from the shell.
- UA: plain `Chrome/122.0.6261.171` (no `QtWebEngine` token) in the app and in a Browser
  tab; Cloudflare passes under it.

Still kiosk-only: GPU/WebGL (the orb is expected to move to a 2D renderer anyway),
mic capture through PipeWire, `Meta+Q` in the DOM, context menu / file dialogs by hand,
and the Qt-vs-GTK benchmark on the real panel.

### Phase 1 — Crate skeleton (done)

`crates/peakd` with `build.rs` (pkg-config `Qt6Core Qt6Gui Qt6Widgets Qt6Network
Qt6WebEngineWidgets Qt6WebChannel`), shim, QApplication window, main view, named profile,
loopback wait, zoom from `display.json` + `--scale`, own `config.rs` (CLI/env parity,
`--iroh` behind the existing feature).

### Phase 2 — Main view + contract

Queuing `window.ipc` shim (§3.4); `EXIT_SHORTCUT_JS` injection; `peakd:exit` and
`peakd:server-mode` (exit 42); permission handler; title/url/load wiring; display choice
watcher; renderer-crash reload; `--devtools`.

### Phase 3 — Child views and the Browser plugin

Port `Views` to the `ViewHost` trait; Qt host: open/navigate/bounds/visible/back/forward/
reload/focus/close, `createWindow` → `new-window` event, context menu, `chooseFiles`,
per-view events, browsing profile + UA, hidden tabs. Acceptance: the browser plugin's smoke
suite (`plugins/browser/web/plugin.smoke.mjs`) plus a manual pass of tabs, Cloudflare,
address bar, new-tab links, home shelf.

### Phase 4 — Parity

Gestures pump, benchmark (identical JSON), `window_moved`, X-error guard review, touchbar
off on Linux (verify `__shinyTouchBar` is not set, as today), server-mode switch end to end.

### Phase 5 — Session integration and switch

`scripts/peakd-kiosk.sh` / `scripts/shiny-session` / `install-linux-session.sh` run
`peakd` on `QT_QPA_PLATFORM=xcb`, substitute `@PEAKD_BIN@`, build the shell if it is
missing, and keep the exit contract (`0`/`42`/crash). Document the
`QTWEBENGINE_DISABLE_SANDBOX=1` fallback if user namespaces are unavailable. Docs:
README kiosk/session sections, PLUGINS.md browser entry, `crates/shiny-filter` doc comment
(it described the proxy era), `web/js/gestures.js` comment.

### Phase 6 — Cleanup (done)

The Linux GTK path is retired: `crates/peakd-mac` is the macOS shell (its GTK/X11 deps,
gesture copy, exit shortcut and X-error guard are gone), `crates/peakd` owns the Linux
kiosk, `--proxy`/`PEAKD_PROXY` is deleted from the Qt shell rather than transplanted, and
the docs above are updated. `web/js/gestures.js`'s comment still says "WebKitGTK" but the
behaviour it describes (evdev swipes) is unchanged.

## 6. File map

| Action | Path |
|---|---|
| new | `crates/peakd/{Cargo.toml,build.rs,src/{main,config,display,gestures,browse,host,bench,shim}.rs,shim/peakd.{h,cpp},shim/ipc_bridge.h,js/{bench.js,bench-drag-tile.js}}` |
| change | `Cargo.toml` (workspace member), `crates/peakd-mac/*` (Linux GTK path retired, macOS shell only) |
| change | `scripts/{peakd-kiosk.sh,shiny-session,install-linux-session.sh}` (Qt-only, `QT_QPA_PLATFORM=xcb`) |
| change | `README.md`, `PLUGINS.md`, `crates/shiny-filter/src/lib.rs` docs |
| unchanged | `plugins/browser/**`, `web/**`, `crates/peakd-mac/src/touchbar.rs`, `scripts/bundle-app.sh` |

## 7. App compatibility checklist (WebKit assumptions to re-test)

| Area | Code | QtWebEngine note |
|---|---|---|
| Voice | `web/js/voice.js` `getUserMedia` | Supported; grant `MediaAudioCapture`. PipeWire from Chromium's audio service — Phase 0 gate |
| File pickers | `web/js/files.js` `pickFiles`, Files/Image plugins | `chooseFiles` override; Chromium fires `cancel` (the code already handles it) |
| Fullscreen | `web/js/fullscreen.js` | Enable `FullScreenSupportEnabled`, accept the request; the kiosk window is maximised anyway, re-verify exit |
| Media | YouTube embed, Files player, Studio/Radio | Autoplay setting; codecs from Debian ffmpeg (VP9/Opus fine; check H.264/AAC) |
| Canvas DPR | `orbCanvas2d.js`, `pdf`, `studio` use `devicePixelRatio` | With scale factor 1 and page zoom, DPR follows zoom exactly as under WebKitGTK — verify at 200 % |
| CSS | `-webkit-backdrop-filter`, `-webkit-line-clamp`, `-webkit-mask-*`, `-webkit-box` | All supported/aliased by Chromium |
| UA | none of `web/js` reads `navigator.userAgent` | Safe to move to the real Chromium UA |
| Storage | app prefs are server-side; localStorage/IndexedDB by plugins | Persist the profile paths (§3.5) |
| Virtual keyboard | `plugins/keyboard` is DOM-only | Existing limitation with native child views is unchanged |
| `window.open` | Browser plugin tabs | `JavascriptCanOpenWindows` + `createWindow` → `new-window` event |

## 8. Risks & mitigations

| Risk | Impact | Mitigation |
|---|---|---|
| GPU/compositing on T2 hybrid graphics under matchbox | blank/slow kiosk | `QTWEBENGINE_CHROMIUM_FLAGS` fallbacks; compare `--benchmark`. Not a functional blocker: the orb is slated to move from WebGL to a 2D renderer, and the 2D fallback already works (`web/js/orbCanvas2d.js`), leaving GPU as a performance question for the open web |
| Memory/CPU vs WebKitGTK | regression on the kiosk | benchmark; keep `MAX_TABS = 8`; per-profile limits |
| IPC handshake race | Browser plugin silently falls back | queuing shim + Phase 0 assertion (§3.4) |
| Sandbox needs user namespaces | kiosk won't start | check `kernel.unprivileged_userns_clone`; document `QTWEBENGINE_DISABLE_SANDBOX=1` (security trade-off) |
| QtWebEngine version churn on distro upgrades | regressions | pin Debian 13's 6.8.x; benchmark per upgrade |
| Two shells across platforms (Linux Qt, macOS wry) | drift | each platform has one shell; the shared contract (§1.2) is frozen and tested from the Qt side |
| Chromium UA changes anti-bot outcomes | some sites behave differently | plain Chromium UA verified with Cloudflare (`nowsecure.nl`) |
| Licensing | — | repo is GPL-3.0; Qt 6/QtWebEngine is LGPLv3 dynamically linked — compatible |

## 9. Verification

- `cargo test -p peakd` (22 unit tests: protocol, rects, host state machine, scale,
  gestures, benchmark diagnosis).
- `cargo build -p peakd && QT_QPA_PLATFORM=xcb ./target/debug/peakd --url …` under
  `xinit` + matchbox, as `scripts/peakd-kiosk.sh` does today.
- Contract: `window.ipc` present at boot; open a tab, navigate, setBounds, back/forward,
  close; events arrive (title/url/load/new-window) — verified with the real Browser plugin.
- Browser plugin: tabs, Cloudflare, new-tab links, home shelf, link hover.
- App: voice input, TTS, Files pick/upload, fullscreen, YouTube, Radio, Studio playback.
- Session: quit → tty1/DM returns; server mode → Qt window closes, server-mode window
  opens, Stop returns to Qt.
- `--benchmark --benchmark-output …` produces the same keys; compare with GTK numbers.
- Crash recovery: kill the renderer (`renderProcessTerminated`) and the whole shell;
  `Restart=on-failure` contract holds.

## 10. Open questions

1. ~~`cxx-qt` vs plain `cxx` + one `moc` run~~ **Resolved: plain C ABI + one `moc` run**
   (only the `IpcBridge` meta-object is needed).
2. ~~One profile or two (app vs browsing)?~~ **Resolved: one named profile, shared.**
3. `--devtools` in a kiosk: currently Chromium remote debugging on `127.0.0.1:9222`;
   possibly add a hidden devtools view later.
4. ~~UA: Peakd token or stock?~~ **Resolved: plain Chromium UA** (QtWebEngine token
   stripped), one shared profile for app and open web.
5. Eventually unify macOS on Qt (would need the `objc2` Touch Bar bridged onto the Qt
   `NSWindow`) or keep two shells permanently? Default: keep two.
