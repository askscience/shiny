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
- Look: the strip and tabs are the shared `tabStrip` / `tab` from `web/ui`
  (the Browser's too): flat mono uppercase labels, the active one a grey
  pill, its close the catalog `ui/close` icon (shown on hover / for the active
  tab / always on touch). The terminal styles only reshape the active tab
  (`.terminal-tabstrip[data-terminal-style] .ui-tab.is-active`). The status line under the terminal is a mono uppercase
  meta line — `SHELL · ~/CWD · STATE` (e.g. `ZSH · ~/SHINY · CONNECTED`),
  built from the session's `ready` frame and decoded (`decodeText`) when it
  changes.
- Theme: xterm reads its face from `--font-mono` and its colours from the
  theme tokens `--terminal-bg`, `--terminal-fg`, `--terminal-cursor`,
  `--terminal-selection` and the optional `--terminal-ansi-<name>` (e.g.
  `--terminal-ansi-bright-blue`). xterm draws on a transparent background
  (the theme colour at alpha 0, so inverse video keeps its colour) over the
  host, which paints `--terminal-bg`. Dusk defines a light paper palette with an
  ANSI set tuned for it (every colour ≥ 4.5:1 on the paper); Ink keeps the
  near-black sheet with xterm's ANSI defaults; other themes fall back to a dark
  surface. The palette is rebuilt (not merged) on `theme:change` — after the
  token stylesheet has loaded — and on `appearance:change`.
- Closing a tab (its X) terminates that shell; a shell that exits closes its
  tab; closing the last tab opens a fresh shell.
- The shells persist across closing/reopening the window and across page
  reloads: the tab list and the active tab live in `sessionStorage`, and on
  remount every session is reattached and its recent output replayed.
- `terminalContextMenu()` contributes **Copy** / **Paste**, **New tab** /
  **Clear scrollback** / **Close tab**.

## Styles

*Settings → Appearance → Terminal* picks one of four styles (preference
`terminal.style`, `getTerminalStyle`/`setTerminalStyle` in
`web/js/preferences.js`, announced as `terminal:style`). A style is shape and
motion only — every colour is a theme token or the accent, so each style
follows Dusk, Ink or any other theme. The Terminal plugin must be installed
for the setting to show.

| id | Settings label | Cursor | Surface |
|---|---|---|---|
| `smear` (default) | Minimal | block whose corners chase the target at different speeds (the Ghostty/Neovide smear); the glyph under it is redrawn in `--terminal-bg` | generous margins, tabs underlined, no status bar — a pill shows on hover and while resizing |
| `glass` | Glass | accent beam with a glow that sheds sparks and drags a tail | surface at 58% so the window's glass shows; status chips (state, shell, cwd, last command, size) |
| `paper` | Paper | ink beam that leaves a fading accent highlighter stroke | a left gutter with a mark per command (hollow = prompt, accent = running, green/red = exit), duration on the right, a band behind the running command; the meta status line |
| `blocks` | Blocks | accent block on an underdamped spring that squashes/stretches with speed, ribbon tail | every command a card (red-tinted on failure, accent rule while running, accent ring on the live prompt), duration and exit on the card |

How it is drawn (`web/fx.js`, one controller per tab):

- xterm's own cursor is a transparent, non-blinking 1px bar; fx.js reads the
  position from the buffer and draws on a canvas **over** the xterm element.
  It follows `?25l/h` (hidden cursor) and an app asking for a bar (DECSCUSR
  5/6). The loop runs only while something moves; at rest the canvas blinks in
  CSS. Reduced motion or the power saver: the cursor snaps, no trail.
- Command marks and cards are drawn on a canvas **under** the xterm element,
  from the shell integration's OSC 133 marks (see
  [architecture § Shell integration](architecture.md#shell-integration)).
  Nothing is drawn in the alternate screen (vim, htop). A prompt that ran
  nothing (empty Enter, Ctrl+C) leaves no mark. Without the integration (a
  shell other than bash/zsh) the styles still work, just without marks.
- The text inset is padding on `.xterm` (the fit addon subtracts it), set per
  style in `web/css/tiles.css` under `[data-terminal-style]`; the line height
  is per style too (`LINE_HEIGHT` in `plugin.js`).

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
