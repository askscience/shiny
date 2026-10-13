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

- Creates one xterm.js terminal per tab (with the fit/webgl/canvas addons), each
  bound to its own PTY session from `POST /api/terminal/sessions`.
- The window holds a single `GET /api/terminal/events` (SSE) connection for
  every tab; frames carry their session id. Input and resize are POSTs; only
  the active tab's host is visible.
- The tabs live in the window's top bar: the strip itself is the merged window
  bar (`data-window-bar`, the browser's arrangement), so the tabs and the
  new-tab `+` sit next to the window controls. A tab is labelled with the
  shell's OSC title, falling back to `Terminal 1`, `Terminal 2`…; its tooltip
  names the shell and start directory.
- Closing a tab (its X) terminates that shell; a shell that exits closes its
  tab; closing the last tab opens a fresh shell.
- The shells persist across closing/reopening the window and across page
  reloads: the tab list and the active tab live in `sessionStorage`, and on
  remount every session is reattached and its recent output replayed.
- `terminalContextMenu()` contributes **Copy** / **Paste**, **New tab** /
  **Clear scrollback** / **Close tab**.

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
