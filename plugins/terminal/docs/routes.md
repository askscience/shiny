# Terminal — REST routes

All routes are `auth`, registered in [`src/plugin.rs`](../src/plugin.rs).

| Method | Path | Purpose |
|---|---|---|
| `POST` | `/api/terminal/sessions` | Create (or reattach to) a PTY session; returns its id. |
| `GET` | `/api/terminal/stream` | SSE stream of terminal output for a session. |
| `POST` | `/api/terminal/input` | Send keystrokes to the session. |
| `POST` | `/api/terminal/resize` | Resize the PTY (cols/rows) when the window resizes. |
| `POST` | `/api/terminal/close` | Terminate the session. |

Live sessions are capped at **8 per user** and **32 process-wide**; the shell
runs with a minimal environment (no server secrets).

## Remote access

The routes are refused for remote (Iroh/Tailscale) clients unless *Allow
Terminal from remote clients* is on. See
[remote access](../../../docs/deployment/remote-access.md).
