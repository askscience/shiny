# Terminal — window surface

[`web/plugin.js`](../web/plugin.js), class prefix `terminal-*`.

```js
export default {
  name: 'terminal',
  mount: mountTerminalTile,
  unmount: unmountTerminalTile,
  getElement: getTerminalTileElement,
  wireEvents: wireTerminalEvents,
  contextMenu: terminalContextMenu,
};
```

## Behaviour

- Creates an xterm.js terminal (with the fit/webgl/canvas addons) bound to a PTY
  session from `POST /api/terminal/sessions`.
- Subscribes to `GET /api/terminal/stream` (SSE) for output, POSTs input and
  resize.
- The shell persists across closing/reopening the window and across page
  reloads; on remount the recent output is replayed and the same session
  reattached.
- `terminalContextMenu()` contributes **Copy** / **Paste**, **New shell** /
  **Clear scrollback** / **Kill session**.

## Clipboard

The terminal is one of the surfaces the central clipboard service
(`web/js/clipboard.js`, see `docs/core/clipboard.md`) integrates. Through
xterm's `attachCustomKeyEventHandler`:

- **Ctrl/Cmd+C** copies the selection when one exists and stays a SIGINT when
  none does (the standard terminal trade-off). **Ctrl/Cmd+V** pastes;
  Ctrl+Shift+C / Ctrl+Shift+V work too.
- Copying records the text in the top-bar Clipboard history (`copyText`);
  pasting reads through `readClipboardText()` and goes into the PTY via
  `term.paste()`, so bracketed-paste mode is respected.
- A pick in the Clipboard menu pastes directly when this window is focused
  (core dispatches `clipboard:paste` with the focused window's name).

## Development

Vendored xterm lives in `web/vendor/`. The app serves the installed copy at
`data/plugins/terminal/web/`; copy or reinstall after editing.
