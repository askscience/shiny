# Clipboard

One clipboard service for the whole app: a top-bar icon with a history of
copied items, native Ctrl/Cmd+C/X/V kept everywhere, and explicit copy/paste
support in the surfaces that cannot use the DOM's own clipboard
(the Terminal's PTY, the Browser's native page view).

## Files

| File | Role |
|---|---|
| `web/js/clipboardShared.js` | Pure rules: entry shape, dedupe, cap, preview, relative time. No DOM — loaded by the Node test. |
| `web/js/clipboard.js` | The service: OS clipboard access, history, DOM capture, shell hook. |
| `web/js/clipboardMenu.js` | The top-bar popover (list, per-row remove, Clear all) and the `Ctrl/Cmd+Shift+V` shortcut. |
| `web/js/preferences.js` | `getClipboardHistory()` / `setClipboardHistory()` — per-user storage under `clipboard.history`. |
| `web/ui/icons/ui/clipboard.svg` | The toolbar/menu icon (`ui/clipboard`). |
| `web/js/tests/clipboardShared.test.mjs` | Node checks for the pure rules. |

## How a copy is recorded

- **App windows / inputs**: the browser does the copy natively (no key is
  hijacked). `clipboard.js` listens for the DOM `copy` / `cut` events in the
  capture phase and records the text, with the source taken from the enclosing
  `[data-plugin]` tile (`app` for chrome like the HUD).
- **Programmatic copies**: every "Copy …" action calls `copyText(text, { source })`,
  which writes the OS clipboard (async Clipboard API, with an `execCommand`
  fallback for kiosk webviews that refuse the permission) and then records the
  text. The Files "Copy Path" and the Settings link/URL buttons go through it.
- **Browser pages**: the page is a native child view, so its Ctrl/Cmd+C never
  reaches the app DOM. The Qt shell's clipboard filter (`crates/peakd/shim/peakd.cpp`)
  runs the engine's Copy action for a focused child view and reports the
  selection back through the new `peakd_clipboard_cb`; Rust forwards it with
  `window.__shinyClipboardCapture(text)` (`web/js/clipboard.js` defines it).
- **Pastes are not history entries** — they only refresh the in-memory mirror
  that `readClipboardText()` falls back to when the async read is refused.

Entries are `{ text, source, at }`, newest first, duplicates collapse instead of
piling up, the list is capped at 50, and texts over 32 KB are copied but not
stored (rules in `clipboardShared.js`).

## Shortcuts

| Context | Behaviour |
|---|---|
| Any app text input/selection | Native Ctrl/Cmd+C/X/V, kept as-is; copies are recorded |
| Anywhere in the app | `Ctrl/Cmd+Shift+V` opens/closes the Clipboard menu |
| Terminal | `Ctrl/Cmd+C` copies the selection when one exists, else stays SIGINT; `Ctrl/Cmd+V` pastes. `Ctrl+Shift+C` / `Ctrl+Shift+V` also work |
| Browser page | Native Chromium Ctrl/Cmd+C/V; plain Ctrl/Cmd+C is captured into the history by the shell; a menu pick pastes into the focused page |
| macOS | Cmd variants work in the app and terminal; Browser pages stay native (the shell hook is Linux-only for now) |

The menu is also the paste route for surfaces the keyboard can reach but the
DOM cannot: picking an entry dispatches `clipboard:paste`
(`{ text, focused }`, focused = the desktop's focused window) and the Terminal
and Browser surfaces consume it — the Browser forwards it to the shell as a
`peakd:view:` `paste` command, which sets the system clipboard and runs the
engine's Paste action on the active tab.

## Plugin surfaces

A surface that needs the clipboard imports the core service directly — the same
relative path convention as `../../js/api.js`:

```js
import { copyText, readClipboardText } from '../../js/clipboard.js';

await copyText(selection, { source: 'terminal' }); // write + record
const text = await readClipboardText();             // OS clipboard, mirror fallback
```

Events on `window`: `clipboard:changed` (the history changed) and
`clipboard:paste` (a menu pick with a paste-capable window focused).

## Testing

- `node web/js/tests/clipboardShared.test.mjs` — the pure rules.
- `node --experimental-vm-modules plugins/browser/web/plugin.smoke.mjs` — the
  browser window's clipboard contract (menu entries, `peakd:view: paste`).
- `cargo test -p peakd` — the view protocol, including parsing `paste`.

The Qt shell part needs a real shell build: `cargo build -p peakd` (Linux) and
a kiosk relaunch. Plugin web files are served from `data/plugins/<name>/web/`,
so copy or reinstall after editing the plugin sources.

## Out of scope

- macOS Browser-page capture (`crates/peakd-mac` needs its own injected
  capture script + IPC allow-list; the app/terminal side is platform-neutral).
- Image/file clipboard entries, and syncing the history to remote clients.
- Capturing OS-wide clipboard changes (`QClipboard::dataChanged`): history
  only records what is copied inside Shiny windows and Browser pages.
