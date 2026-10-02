# Terminal — architecture

```
plugins/terminal/
├── plugin.toml
├── skills/terminal.md
├── src/
│   ├── lib.rs        module wiring
│   ├── plugin.rs     manifest, routes, entry (no tools)
│   ├── routes.rs     /api/terminal/* handlers (SSE stream)
│   └── pty.rs        PTY session management
└── web/
    ├── plugin.js     the Terminal window
    ├── icon.svg
    └── vendor/       xterm.js + addon-fit/-webgl/-canvas + xterm.css
```

## PTY sessions

`pty.rs` owns one pseudo-terminal per session: it spawns the login shell, pumps
the master fd into a broadcast channel (consumed by the SSE `stream` route), and
accepts input/resize/close from the other routes. Sessions are keyed by a
session id the window receives from `POST /api/terminal/sessions`.

The window keeps its shell **alive while closed**: reopening the window (or
reloading the page) replays the recent output and reattaches to the same
session. "New shell" starts a fresh session; "Kill session" terminates the
current one.

## Streaming

`GET /api/terminal/stream` is an SSE stream of terminal output; input and resize
are POSTs. This avoids WebSocket handling in the plugin while keeping the
interactive feel.

## Runtime

Handlers are registered with `bridged_route`, so their async work runs on the
plugin-owned runtime. See [runtime & ABI](../../../docs/plugins/runtime-abi.md).

## Vendored client

xterm.js and its addons are vendored under `web/vendor/` (not loaded from a CDN)
so the terminal works offline and under the kiosk.

## Related

[README](README.md) · [routes](routes.md) · [window](window.md).
