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
| Agent tools | none (human-facing only) |

## What it adds

- A Terminal window with a persistent shell session.
- `/api/terminal/*` routes for session create/stream/input/resize/close
  ([routes.md](routes.md)).
- Vendored xterm.js + addons (`addon-fit`, `addon-webgl`, `addon-canvas`) under
  `web/vendor/`.

The plugin contributes **no agent tools and no persona**; the terminal is a
human-facing surface only. The AI can open it with the core `show_plugin` tool
but cannot type into it.

## Security

The shell has the server user's privileges. Shiny refuses the Terminal from
**remote** clients unless *Allow Terminal from remote clients* is enabled in
Settings. See [remote access](../../../docs/deployment/remote-access.md).

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
