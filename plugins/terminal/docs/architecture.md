# Terminal — architecture

```
plugins/terminal/
├── plugin.toml
├── skills/terminal.md
├── src/
│   ├── lib.rs        module wiring
│   ├── plugin.rs     manifest, persona, routes + tool registration
│   ├── routes.rs     /api/terminal/* handlers (SSE stream)
│   ├── pty.rs        PTY session management
│   └── tool.rs       terminal_exec — run a command in a session
└── web/
    ├── plugin.js     the Terminal window
    ├── icon.svg
    └── vendor/       xterm.js + addon-fit/-webgl/-canvas + xterm.css
```

## Agent tool

`terminal_exec` (aliases `terminal`, `terminal_run`, `shell`, `run_command`)
lets the AI run a command in the terminal. It writes the command into a
**persistent shell the tool owns** (`pty::agent_for`, created at 80×24 on first
use) — never the window's session, so a command can't be typed into whatever
the user (or a CLI they run there) is doing. It then collects the session's
output until the shell has been quiet for 400ms or 10s have passed
(`timed_out: true`). The capture starts after the echo of the typed command, so
the shell's own echo and any banner are dropped; ANSI sequences are stripped
and the result is capped at 16k chars, head and tail. The tool wraps the
session registry directly — it does not go through the REST routes.

## PTY sessions

`pty.rs` owns one pseudo-terminal per session: it spawns the login shell, pumps
the master fd into a broadcast channel (consumed by the SSE `events` route, and
by `stream` for single-session clients), and accepts input/resize/close from the
other routes. Sessions are keyed by a session id the window receives from
`POST /api/terminal/sessions`.

The window keeps its shells **alive while closed**: the tab list and the active
tab live in `sessionStorage`, and reopening the window (or reloading the page)
replays each tab's recent output and reattaches to the same session. The strip
is the window's top bar — one shell per tab. Closing a tab terminates that
shell, a shell that exits (`exit`, a crash, a server restart) closes its own
tab, and closing the last tab opens a fresh shell.

## Streaming

`GET /api/terminal/events` is one SSE response carrying every session the
window owns, each frame tagged with its session id; input and resize are POSTs.
One connection for all tabs is not an optimisation: a browser allows only a few
connections per origin (Chromium: six) and the app already holds long-lived
streams of its own, so a stream per tab starved the second tab — its frames
never arrived and its keystrokes queued behind the limit. A tab created after
the stream connected is announced from the session registry and picked up live;
a session the window restored that no longer exists answers `gone`, so its tab
is dropped. `GET /api/terminal/stream?session=…` remains as the single-session
primitive. This avoids WebSocket handling in the plugin while keeping the
interactive feel.

## Runtime

Handlers are registered with `bridged_route`, so their async work runs on the
plugin-owned runtime. See [runtime & ABI](../../../docs/plugins/runtime-abi.md).

## Vendored client

xterm.js and its addons are vendored under `web/vendor/` (not loaded from a CDN)
so the terminal works offline and under the kiosk.

## Related

[README](README.md) · [routes](routes.md) · [window](window.md).

PTY output subscribers and new-session watchers use bounded queues (32 pending
messages plus the channel's sender slot). A stalled consumer is disconnected
rather than retaining unlimited output. A dropped session feed closes the
multiplexed HTTP body so the existing window reconnects, receives `ready`, and
replays the latest bounded scrollback. Session IDs, frame payloads, PTY lifetime
and the 256 KiB scrollback contract are unchanged. The shell itself keeps running.

The window feature-detects the core `apiRequest` helper. New core pages share
session recovery with other requests; a hot install into a page with the older
core API cached retains the legacy bearer-authenticated fetch path until reload.
