# Terminal — REST routes

All routes are `auth`, registered in [`src/plugin.rs`](../src/plugin.rs).

| Method | Path | Purpose |
|---|---|---|
| `POST` | `/api/terminal/sessions` | Create (or reattach to) a PTY session; returns its id. |
| `GET` | `/api/terminal/events` | SSE stream of **every** session the window owns, each frame tagged with its session id — one connection for all tabs. |
| `GET` | `/api/terminal/stream` | SSE stream of terminal output for one session (the single-session primitive). |
| `POST` | `/api/terminal/input` | Send keystrokes to the session. |
| `POST` | `/api/terminal/resize` | Resize the PTY (cols/rows) when the window resizes. |
| `POST` | `/api/terminal/close` | Terminate the session. |

Live sessions are capped at **8 per user** and **32 process-wide**; the shell
runs with a minimal environment (no server secrets).

The window uses `events`, not `stream`: a browser allows only a handful of
connections per origin (Chromium: six) and the app already holds long-lived
streams of its own (the top-bar chips), so one stream per tab starved the
second tab — its frames never arrived and its keystrokes queued behind the
limit. `events` takes an `expect=` list of session ids; any the server no
longer has answers `gone` so the window can drop that tab.

## Remote access

The routes are refused for remote (Iroh/Tailscale) clients unless *Allow
Terminal from remote clients* is on. See
[remote access](../../../docs/deployment/remote-access.md).
