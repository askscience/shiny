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
- `terminalContextMenu()` contributes **New shell** / **Kill session** and
  copy/paste entries.

## Development

Vendored xterm lives in `web/vendor/`. The app serves the installed copy at
`data/plugins/terminal/web/`; copy or reinstall after editing.
