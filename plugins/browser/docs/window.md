# Browser plugin — `web/plugin.js` window surface

`plugins/browser/web/plugin.js` is the Browser window. Core builds the `.tile`
and the plugin mounts a `section.tile.browser-tile` inside it. The chrome is
HTML; the **page** is a native child webview owned by the shell
(`crates/peakd`), driven from here over `window.ipc`. The one thing rendered
locally is the home surface: a sandboxed `srcdoc` iframe holding the
related-news shelf.

Imports from core:

```js
import { apiFetch } from '../../js/api.js';
import { openWithPlugin, pluginForFile, revealInFiles } from '../../js/files.js';
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
| `message` (on `window`) | `onFrameMessage` — the sandboxed home shelf's `browser:open-card` / `browser:refresh-news` |
| `theme:change` | `pushSettings` — keeps pages' `prefers-color-scheme` in step with the app theme |

Note `window.__peakdViewEvent` is defined at **module scope**, not inside
`wireBrowserEvents`, because the shell may call it before `wireEvents` runs (a
navigation can finish while the window is still mounting).

### `browserContextMenu()`

Returns menu items: New tab, New incognito tab, Downloads, History, Bookmarks,
Bookmark/Remove bookmark, Focus address bar, Reload, Home. The bookmark label
depends on `isBookmarked(activeTab()?.url)`.

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
- The home shelf is a sandboxed iframe: `sandbox="allow-scripts"` with
  `referrerpolicy="no-referrer"`, `srcdoc` set to `homeDocument(news)`.

## Custom events and globals

### Listens

| Event | Source | Effect |
|---|---|---|
| `artifact:saved` | core artifact store | `onArtifactSaved` navigates to `art.narrative` (falls back to `payload.url`/`params.url`/`title`) for `artifact_type === 'browser_page'` |
| `message` | sandboxed home shelf | `browser:open-card` → `news/click` POST + navigate; `browser:refresh-news` → reload the shelf |
| `theme:change` | core theme | `pushSettings()` |
| `resize`, `desktop:changed`, `overlay:open`, `fullscreen:change`, `menu:change` | core | `requestSync()` for native-view bounds/mask |

### Global callbacks the shell calls

| Global | Payload | Purpose |
|---|---|---|
| `window.__peakdViewEvent(event)` | `{ id, type, ... }` | Per-tab `title`, `url`, `load` (`phase: started/finished`), `new-window`; and non-tab `download` events (`applyDownload`) |
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

`peakd:view:` ops (parsed by `crates/peakd/src/browse.rs` `Command::parse`):
`open` (`url`, `rect`, `visible`, `incognito`), `navigate`, `setBounds`,
`setVisible`, `setMask` (`holes`), `back`, `forward`, `reload`, `close`,
`focus`.

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

## Native-view bounds and mask

A native child view is a real window stacked **above** the page: it ignores
`display:none` and every HTML layer. The window therefore:

- computes `viewportRect()` in CSS pixels plus `dpr` and sends `setBounds`;
- hides the view (`setVisible: false`) when the tile is `.hidden`, during a
  drag (`is-dragging`), or while `overview-active` / `launcher-active`;
- computes `occluderHoles()` (open `.ctx-menu`, `.ui-hud-menu-popup`,
  `.ui-modal`, notifications/toasts, and this window's own popovers) and sends
  `setMask` so the page shows through only where it is not covered;
- re-sends the mask on the way back from a hide, because a stale hole would
  otherwise clip the page once the window moves.

`requestSync()` coalesces bursts into one sync per animation frame;
`startBoundsSync()` adds a 250 ms fallback interval, a `ResizeObserver` on the
frame wrapper, `MutationObserver`s on the tile grid and body class, and the
window events listed above. `stopBoundsSync()` tears them all down.

## Gotchas

- **The page is not an iframe.** `frameEl` is parked on `about:blank` and hidden
  whenever a tab is native; all real rendering is the child webview. A test must
  assert the IPC *commands*, not the iframe URL (see
  `web/plugin.smoke.mjs`).
- **The home shelf cannot reach the app.** The sandbox omits
  `allow-same-origin`; the only channel is `postMessage`, and `onFrameMessage`
  guards by `event.source`, not by iframe element.
- **`artifact:saved` is the AI navigation path.** The URL lives in `narrative`;
  `payload.url`, `params.url` and `title` are legacy fallbacks. Editing this
  path can silently break AI-driven opens.
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
events + badge + panel mask hole, drag hide/restore, tab close, and AI
`artifact:saved` navigation.

```bash
node --experimental-vm-modules plugins/browser/web/plugin.smoke.mjs
```

## Cross-references

- [`README.md`](README.md) · [`architecture.md`](architecture.md) · [`tools.md`](tools.md) · [`routes.md`](routes.md)
- [`../../../crates/peakd/src/browse.rs`](../../../crates/peakd/src/browse.rs) — the IPC contract.
- [`../../../docs/core/desktop.md`](../../../docs/core/desktop.md) — tile/window host.
