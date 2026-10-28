# Browser plugin — `web/plugin.js` window surface

`plugins/browser/web/plugin.js` is the Browser window. Core builds the `.tile`
and the plugin mounts a `section.tile.browser-tile` inside it. The chrome is
HTML; the **page** is a web view **item in the shell's own scene**
(`crates/peakd`), driven from here over `window.ipc`. The one thing rendered
locally is the home surface: a sandboxed `srcdoc` iframe holding the
bookmarks grid (favicon + name) and the related-news shelf.

Imports from core:

```js
import { apiFetch } from '../../js/api.js';
import { copyText, readClipboardText } from '../../js/clipboard.js';
import { openContextMenu } from '../../js/contextMenu.js';
import { openWithPlugin, pluginForFile, revealInFiles } from '../../js/files.js';
import { onOpenUrl } from '../../js/linkRouter.js';
import { nativeOccluderHoles, watchNativeOccluders } from '../../js/nativeOccluders.js';
import { openCoreWindow } from '../../js/tiles.js';
import { icon, iconButton, searchBar, themeMode } from '../../ui/index.js';
```

## Exports

| Export | Signature | Role |
|---|---|---|
| `BROWSER_PLUGIN` | `'browser'` | The plugin name constant (used for `data-plugin` and the desktop) |
| `mountBrowserTile()` | `→ HTMLElement` | Build and return the tile; idempotent |
| `unmountBrowserTile()` | `→ void` | Tear down all tabs, observers and cached state |
| `getBrowserTileElement()` | `→ HTMLElement \| null` | The mounted tile |
| `wireBrowserEvents()` | `→ void` | Attach window-level listeners; idempotent (`wired` flag) |
| `browserContextMenu()` | `→ MenuItem[]` | Entries core splices into this window's right-click menu |
| `default` | `{ name, icon, mount, unmount, getElement, wireEvents, contextMenu }` | The core plugin-window contract |

```js
export default {
  name: 'browser',
  icon: 'ui/launcher',
  mount: mountBrowserTile,
  unmount: unmountBrowserTile,
  getElement: getBrowserTileElement,
  wireEvents: wireBrowserEvents,
  contextMenu: browserContextMenu,
};
```

### `mountBrowserTile()`

- `ensureStylesheet()` injects `plugin.css` (core does **not** auto-inject a
  plugin stylesheet).
- Creates `section.tile.browser-tile` with `dataset.plugin = 'browser'`.
- Builds the tab strip, toolbar, viewport, downloads panel, browser menu,
  history panel and bookmarks panel, then appends them.
- Adds capture-phase `pointerdown`/`keydown` document listeners.
- `updateNavButtons()`, `updateShield()`, `startBoundsSync()`, one home tab
  (`createTab({})`), then `loadSettings()`, `loadDownloads()`, `loadBookmarks()`.
- Returns the tile.

### `unmountBrowserTile()`

Stops the bounds sync, removes the frame-message and document listeners, sets
`wired = false`, destroys every tab (which closes its native view), clears all
cached arrays, removes the tile and nulls every element reference.

### `wireBrowserEvents()`

Attaches, once:

| Event | Handler |
|---|---|
| `artifact:saved` (on `window`) | `onArtifactSaved` — an AI-created `browser_page` artifact navigates the active tab |
| `url:open` (via core `onOpenUrl`) | `openCoreUrl` — a link clicked anywhere in the app opens as a new tab |
| `clipboard:paste` (on `window`) | `onClipboardPaste` — a pick in the top-bar Clipboard menu pastes into the focused page (see Clipboard below) |
| `message` (on `window`) | `onFrameMessage` — the home surface's `browser:open-card` / `browser:open-bookmark` / `browser:refresh-news` |
| `theme:change` | `pushSettings` — keeps pages' `prefers-color-scheme` in step with the app theme |

Note `window.__peakdViewEvent` is defined at **module scope**, not inside
`wireBrowserEvents`, because the shell may call it before `wireEvents` runs (a
navigation can finish while the window is still mounting).

### `browserContextMenu()`

Returns menu items: New tab, New incognito tab, Downloads, History, Bookmarks,
Bookmark/Remove bookmark, Copy page address, Paste and go, Focus address bar,
Reload, Home. The bookmark label depends on `isBookmarked(activeTab()?.url)`.

## Clipboard

The Browser is one of the surfaces the central clipboard service
(`web/js/clipboard.js`, see `docs/core/clipboard.md`) integrates:

- The window chrome (address bar, panels) is app DOM, so copies made there are
  recorded by the service like anywhere else.
- Copies made **inside the page** never reach the app DOM (the page is a native
  child view). The Qt shell's clipboard filter reports plain Ctrl/Cmd+C from a
  child view through `window.__shinyClipboardCapture`, so page copies show up in
  the top-bar history.
- A pick in the Clipboard menu with this window focused dispatches
  `clipboard:paste`; `onClipboardPaste` forwards it to the shell as a
  `peakd:view:` `paste` command, which sets the system clipboard and triggers
  the engine's Paste action on the active tab.
- "Paste and go" reads the clipboard via `readClipboardText()` and navigates.

## Shield and ad blocking

The toolbar shield toggles the shell's filter (`peakd:filter:`) and persists the
choice (`POST /api/browser/settings`). Two things make the on/off state visible:

- Half the filter is a **document-start scriptlet** (the YouTube ad-skip, see
  `crates/peakd/src/youtube.rs`), and a view only picks up the new script
  collection when a document is created — so `toggleShield()` reloads the active
  tab's page (`peakd:view:` `reload`) right after telling the shell. Flipping the
  shield shows or hides ads on the page in front of the user, not only on the
  next visit.
- The shell pushes `window.__peakdShield` only when something changes, so the
  window polls `peakd:shield:query` every `SHIELD_POLL_MS` (3 s) while it is
  mounted. That keeps the button's "N blocked" count live — the visible proof
  the filter is working on a page that looks the same either way.

The core Settings window's own ad-block switch writes the same preference, but
only this window has the shell bridge: it listens for `settings:change` and
re-pushes (and reloads the open page) when that save lands.

## DOM and CSS

- Root: `<section class="tile browser-tile" data-plugin="browser">`.
- Every style is scoped under `.browser-tile`; class prefix is **`browser-`**:
  `browser-tabstrip`, `browser-tab`, `browser-tab-label`, `browser-tab-text`,
  `browser-tab-close`, `browser-tab-mask`, `browser-newtab`, `browser-bar`,
  `browser-address`, `browser-nav--forward`, `browser-frames`, `browser-frame`,
  `browser-shield` (`.is-on`/`.is-off`), `browser-downloads-panel`,
  `browser-downloads-head`, `browser-downloads-title`, `browser-downloads-clear`,
  `browser-downloads-list`, `browser-downloads-badge`, `browser-download`,
  `browser-download is-completed` (state class), `browser-menu`,
  `browser-history-panel`, `browser-bookmarks-panel`, `browser-status`.
- The toolbar controls are **core components** (`iconButton`, `searchBar`,
  `icon`); `plugin.css` only arranges them. Back/forward share `ui/arrow-left`
  and forward is mirrored in CSS (`browser-nav--forward`).
- The home surface is a sandboxed iframe: `sandbox="allow-scripts"` with
  `referrerpolicy="no-referrer"`, `srcdoc` set to
  `homeDocument(news, bookmarks)`. Its inline click handler is allowlisted by
  an exact CSP hash (`web/js/tests/cspHomeShelfHash.test.mjs`).
- The bookmarks grid shows each saved page as a favicon tile with the name
  underneath (the site's own `/favicon.ico`, letter fallback). A grid card
  posts `browser:open-bookmark`, and both the grid and the bookmarks popover
  open the URL in a new tab.

## Custom events and globals

### Listens

| Event | Source | Effect |
|---|---|---|
| `artifact:saved` | core artifact store | `onArtifactSaved` navigates to `art.narrative` (falls back to `payload.url`/`params.url`/`title`) for `artifact_type === 'browser_page'` |
| `url:open` | core link router (`web/js/linkRouter.js`) | `openCoreUrl` — a link clicked anywhere in the app opens as a new tab |
| `message` | sandboxed home surface | `browser:open-card` → `news/click` POST + navigate; `browser:open-bookmark` → open in a new tab; `browser:refresh-news` → reload the shelf |
| `theme:change` | core theme | `pushSettings()` |
| `settings:change` | core Settings window (browser section) | `onSettingsChanged` — re-fetches `/api/browser/settings`, re-pushes to the shell, and reloads the active page when `adblock` changed |
| `resize`, `desktop:changed`, `overlay:open`, `fullscreen:change`, `menu:change` | core | `requestSync()` for native-view bounds/mask |

### Global callbacks the shell calls

| Global | Payload | Purpose |
|---|---|---|
| `window.__peakdViewEvent(event)` | `{ id, type, ... }` | Per-tab `title`, `url`, `load` (`phase: started/finished`), `new-window`, `context-menu` (`menu` = `{x,y,linkUrl,editable,selection}`), `page-js` (`token`, `result` — the answer to `runJs`); and non-tab `download` events (`applyDownload`) |
| `window.__peakdShield(state)` | `{ enabled, blocked, rules }` | Updates the shield button title / `browserSettings` |
| `window.__peakdDownloads(list)` | `Download[]` | Replaces the live downloads list (answer to `peakd:downloads:list`) |

### Dispatches / posts

- The sandboxed shelf `parent.postMessage(...)`s `browser:open-card` and
  `browser:refresh-news`; the window handles them in `onFrameMessage` and
  matches `event.source` against each tab's `frameEl.contentWindow` so the
  **tab that sent it** updates, never the active one.

## IPC bridge (`window.ipc.postMessage`)

`nativeAvailable` is true only when `window.ipc.postMessage` exists. Helpers:
`nativeSend` (prefixed `peakd:view:`), `shellSend` (prefixed non-view JSON),
`shellRaw` (raw string).

| Prefix | Payload | Purpose |
|---|---|---|
| `peakd:view:` | `{ op, id, … }` | Native child-webview commands |
| `peakd:settings:` | `{ adblock, downloadsDir, colorScheme }` | Shell settings |
| `peakd:filter:` | `{ enabled }` | Live shield toggle |
| `peakd:download:` | `{ id, action }` | `pause`/`resume`/`cancel`/`remove`/`clear` |
| `peakd:downloads:list` | raw (no JSON) | Ask the shell to push its live list via `window.__peakdDownloads` |
| `peakd:shield:query` | raw (no JSON) | Ask the shell to push the shield state via `window.__peakdShield` (polled while mounted, see Shield and ad blocking) |

`peakd:view:` ops (parsed by `crates/peakd/src/browse.rs` `Command::parse`):
`open` (`url`, `rect`, `visible`, `incognito`), `navigate`, `setBounds`,
`setVisible`, `setMask` (`holes`), `back`, `forward`, `reload`, `close`,
`focus`, `paste` (`text`), `action` (`verb` — one engine action on the page's
focused element: `Cut` / `Copy` / `Paste` / `SelectAll`), and `runJs` (`token`,
`script` — evaluate a script in the page and answer with a `page-js` view event
carrying the same token and the result).

## API calls (`apiFetch`)

| Call | Trigger |
|---|---|
| `GET /api/browser/settings` | `loadSettings` |
| `POST /api/browser/settings` | shield toggle, incognito, engine change |
| `POST /api/browser/navigate` | `navigateTo` |
| `POST /api/browser/session/close` | `closeTab` (best-effort) |
| `GET /api/browser/news?limit=12[&refresh=1]` | `fetchNews` / `showHome` |
| `POST /api/browser/news/click` | home card click (`onHomeCard`) |
| `GET /api/browser/downloads?limit=50` | `loadDownloads` |
| `POST /api/browser/downloads/event` | `applyDownload` (shell event) |
| `POST /api/browser/downloads/remove` | `downloadAction(id, 'remove')` |
| `POST /api/browser/downloads/clear` | `downloadAction('', 'clear')` |
| `GET/POST/DELETE /api/browser/history*` | history panel open/clear |
| `GET /api/browser/bookmarks`, `POST …/add`, `POST …/remove` | bookmarks panel and the star toggle |

## Tab model

Each tab is `{ id, sessionId, title, url, home, incognito, native, history,
historyIndex, loaded, watchdog, frameEl }`.

- `MAX_TABS = 8`; creating beyond the cap closes the oldest inactive tab.
- `NATIVE_LOAD_TIMEOUT_MS = 45000`: a long, non-error watchdog that only drops
  the "Loading…" hint — it never reloads, because a forced reload would
  interrupt a Cloudflare challenge.
- `recordLocation` maintains each tab's local back/forward list; `go(delta)`
  sends `back`/`forward` to the shell (the shell owns the real history).
- Incognito tabs set `is-incognito`, render a mask icon and ask the shell for
  the off-the-record profile (`incognito: true` on `open`); their navigations
  are not recorded.
- Closing the last tab creates a fresh home tab.

## Page bounds, visibility and holes

The page is an item in the shell's own scene, composited by the same renderer
as the app's DOM, so it follows its window frame for frame — a drag never has
to drop it (that was the old white flash). The window therefore:

- computes `viewportRect()` in CSS pixels plus `dpr` and sends `setBounds`
  (the shell moves the item; there is no separate window to move);
- hides it (`setVisible: false`) only for the states the app draws over
  everything: a `.hidden` tile (a workspace switch), `overview-active`,
  `launcher-active`;
- computes `occluderHoles()` and sends `setMask`: the shell cuts the page there
  and lets the presses through to the app, so an overlay stays usable while the
  page keeps rendering everywhere else. Core chrome comes from
  `nativeOccluderHoles()` in `web/js/nativeOccluders.js` — the chat panel,
  top/workspace bar, virtual keyboard, navigation HUD, travel panel, tile
  overlay, and the shared menus/modals/sheets/floating
  windows/toasts/notifications. Persistent non-modal chrome such as the voice
  bar stays out on purpose: a hole disables page input, so only surfaces the
  user is actively working in may take the page's pointer. This window adds its
  own popovers and any higher tile in the "Windows" layout;
- re-sends the mask on the way back from a hide, because a stale hole would
  otherwise clip the page once it moves.

Hiding a page never suspends one the engine still needs: the shell freezes a
hidden page only where `recommendedState` (and `recentlyAudible`) say it may, so
a background tab that is playing audio in the "keep alive" sense — a YouTube tab
you switched away from, a window left in another workspace — keeps playing
exactly like a background tab in a normal browser. Idle hidden pages are still
frozen (see `kPageQml` in `crates/peakd/shim/peakd.cpp`).

`requestSync()` coalesces bursts into one sync per animation frame;
`startBoundsSync()` adds a 1 s fallback interval, a `ResizeObserver` on the
frame wrapper, `MutationObserver`s on the tile grid and body class, the core
occluder watcher (which catches a panel's delayed `.hidden` class), and the
window events listed above. `stopBoundsSync()` tears them all down.

## Right-click inside a page

A right-click in a page opens the app's own menu over the page — never a
platform menu. The shell forwards the request as a `context-menu` view event
(`{x, y, linkUrl, editable, selection}`; the position is local to the page item,
so the plugin adds the frame origin and divides by `dpr`), and the plugin opens
core's `openContextMenu()` at the cursor. The `.ctx-menu` mask hole keeps it
visible and clickable (see above). Entries map back to `peakd:view:` commands:
`back` / `forward` / `reload`, Copy Link Address through the clipboard service
(so it lands in the top-bar history), and the edit verbs — Cut / Copy / Paste /
SelectAll — via the generic `action` op. Only the active tab's page opens a
menu.

## Gotchas

- **The page is not an iframe.** `frameEl` is parked on `about:blank` and hidden
  whenever a tab is native; all real rendering is the page item in the shell's
  scene. A test must
  assert the IPC *commands*, not the iframe URL (see
  `web/plugin.smoke.mjs`).
- **The home shelf cannot reach the app.** The sandbox omits
  `allow-same-origin`; the only channel is `postMessage`, and `onFrameMessage`
  guards by `event.source`, not by iframe element.
- **`artifact:saved` is the AI navigation path.** The URL lives in `narrative`;
  `payload.url`, `params.url` and `title` are legacy fallbacks. Editing this
  path can silently break AI-driven opens.
- **Links from the app arrive before the tile exists.** This window is the
  default link handler (`open-urls`): core imports and wires the module, then
  mounts the tile. A `url:open` handled while `tileEl` is null is parked in
  `pendingCoreOpen` and drained at the end of `mountBrowserTile` — `createTab`
  needs the frame wrapper, so it cannot run earlier.
- **The engine list is duplicated** in `SEARCH_ENGINES` for menu labels only;
  the server (`plugins/browser/src/settings.rs`) is the source of truth for the
  actual search URL.
- **No `window.ipc` means no pages.** Without the shell bridge,
  `nativeAvailable` is false and `loadNative`/`showHome` degrade (the smoke test
  stubs `window.ipc` to exercise the contract).
- **The downloads panel merges two sources**: the persisted
  `/api/browser/downloads` rows and live `window.__peakdViewEvent` events, by
  `id`. Clearing the panel must delete finished rows server-side too.
- **CSS files are not auto-injected**; `ensureStylesheet()` is mandatory in
  `mount`.

## Smoke test

`plugins/browser/web/plugin.smoke.mjs` runs the real module against a DOM shim
and asserts: mount/exports, the toolbar controls, settings push, one sandboxed
home tab with news, navigation POST + `peakd:view: open`, `setBounds`/`setMask`,
the shield (state + `peakd:filter:` + persistence), incognito tabs, downloads
events + badge + panel mask hole, a core occluder (the chat-panel case) punching
and clearing its hole, drag hide/restore, tab close, AI `artifact:saved`
navigation, and the clipboard contract (context-menu entries,
`clipboard:paste` → `peakd:view: paste`, picks for other windows ignored).

```bash
node --experimental-vm-modules plugins/browser/web/plugin.smoke.mjs
```

## Cross-references

- [`README.md`](README.md) · [`architecture.md`](architecture.md) · [`tools.md`](tools.md) · [`routes.md`](routes.md)
- [`../../../web/js/nativeOccluders.js`](../../../web/js/nativeOccluders.js) — the core surfaces that punch through a page.
- [`../../../crates/peakd/src/browse.rs`](../../../crates/peakd/src/browse.rs) — the IPC contract.
- [`../../../docs/core/desktop.md`](../../../docs/core/desktop.md) — tile/window host.
