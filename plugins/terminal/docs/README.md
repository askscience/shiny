# Terminal plugin

A real Linux terminal inside the app: a PTY-backed login shell rendered with
[xterm.js](https://xtermjs.org). It is **not a sandbox** — commands run as the
same user the Shiny server runs as.

| | |
|---|---|
| Plugin name | `terminal` |
| Category | `System` |
| Version / API level | `0.1.0` / `1` |
| Crate | `shiny-terminal-plugin` (`libshiny_terminal_plugin.so`) |
| Database | none |
| Web surface | `plugins/terminal/web/plugin.js` (prefix `terminal-*`) |
| Agent tools | `terminal_exec` — run a shell command in the session |

## What it adds

- A Terminal window with browser-style tabs — one persistent shell session per
  tab, all streamed over a single SSE connection.
- `/api/terminal/*` routes for session create/stream/input/resize/close
  ([routes.md](routes.md)).
- Vendored xterm.js + addons (`addon-fit`, `addon-webgl`, `addon-canvas`) under
  `web/vendor/`.

The plugin's agent tool is `terminal_exec`: the AI types a command into a
persistent shell of its own (never one of the window's shells, so the user's
work is never disturbed), waits for the shell to go quiet and gets the output
back; `timed_out: true` means the command is still running and was left alone.
The AI can also open the window with the core `show_plugin` tool, and the user
types there directly.

## Command approval

In the text chat, every `terminal_exec` command is a question: the SSE turn
emits an `approval` event, the chat shows the exact command with **Deny**,
**Allow** and **Allow for this chat**, and the run parks until
`POST /api/agent/approve` answers. "Allow for this chat" persists on the
conversation (`chat_conversations.terminal_auto_approve`), so only that thread
runs commands without asking. See
[agent § Terminal command approval](../../../docs/core/agent.md#46-terminal-command-approval).

A hard-coded guard (`src/services/terminal_guard.rs`) runs before the prompt:
commands that would destroy the system — removing `/` or a system directory,
raw writes to a disk device, `mkfs`/partitioning, power-off/reboot, fork bombs —
are refused and never offered, even in an auto-approved chat. Deleting ordinary
files and folders is allowed; that is what the per-command approval is for.

## Output capture and recovery

Command capture keeps a bounded prefix and rolling tail before applying the
existing 16,000-character output limit. Verbose output retains its start and latest
diagnostics with a truncation marker. If its output feed falls behind and closes,
the returned text explicitly reports interruption; the command itself continues
in its existing PTY. The tool's result fields remain unchanged.

## Security

The shell has the server user's privileges. Shiny refuses the Terminal from
**remote** clients unless *Allow Terminal from remote clients* is enabled in
Settings. See [remote access](../../../docs/deployment/remote-access.md).
The `terminal_exec` tool is gated by the chat's allow/deny prompt and the
hard-coded safety guard (above); the remote-clients preference still covers only
the REST routes (the tool has no access to the request's remote flag yet) — treat
that as roadmap.

## Source layout

```
plugins/terminal/
├── plugin.toml
├── skills/terminal.md
├── src/{lib,plugin,routes,pty}.rs
└── web/{plugin.js,icon.svg,vendor/*}
```

## Related

[architecture](architecture.md) · [routes](routes.md) · [window](window.md) ·
[plugin system](../../../docs/plugins/README.md).
