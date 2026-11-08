# Browser plugin — `web/plugin.js` window surface

`plugins/browser/web/plugin.js` is the Browser window. Core builds the `.tile`
and the plugin mounts a `section.tile.browser-tile` inside it. The chrome is
HTML; the **page** is a web view **item in the shell's own scene**
(`crates/liteaura`), driven from here over `window.ipc`. The one thing rendered
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
import { createPasswordsPanel, fillScript, readScript } from './passwords.js';
import { createLoginAssist } from './login-assist.js';
import { createImportPanel } from './import-panel.js';
import { openCoreWindow } from '../../js/tiles.js';
import { icon, iconButton, searchBar, tabStrip, tab, themeMode } from '../../ui/index.js';
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

Note `window.__liteauraViewEvent` is defined at **module scope**, not inside
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
  `liteaura:view:` `paste` command, which sets the system clipboard and triggers
  the engine's Paste action on the active tab.
- "Paste and go" reads the clipboard via `readClipboardText()` and navigates.

## Shield and ad blocking

The toolbar shield toggles the shell's filter (`liteaura:filter:`) and persists the
choice (`POST /api/browser/settings`). Two things make the on/off state visible:

- Half the filter is a **document-start scriptlet** (the YouTube ad-skip, see
  `crates/liteaura/src/youtube.rs`), and a view only picks up the new script
  collection when a document is created — so `toggleShield()` reloads the active
  tab's page (`liteaura:view:` `reload`) right after telling the shell. Flipping the
  shield shows or hides ads on the page in front of the user, not only on the
  next visit.
- The shell pushes `window.__liteauraShield` only when something changes, so the
  window polls `liteaura:shield:query` every `SHIELD_POLL_MS` (3 s) while it is
  mounted. That keeps the button's "N blocked" count live — the visible proof
  the filter is working on a page that looks the same either way.

The core Settings window's own ad-block switch writes the same preference, but
only this window has the shell bridge: it listens for `settings:change` and
re-pushes (and reloads the open page) when that save lands.

## DOM and CSS

- Root: `<section class="tile browser-tile" data-plugin="browser">`.
- Every style is scoped under `.browser-tile`; class prefix is **`browser-`**:
  `browser-tabstrip`, `browser-tab` (hooks on the shared `.ui-tabstrip` /
  `.ui-tab`), `browser-tab-mask`, `browser-newtab`, `browser-tabcount`,
  `browser-tabs-panel`, `browser-tabs-row`, `browser-tab-close` (the switcher
  row's), `browser-passwords-panel`, `browser-passwords-btn`, `browser-pw-*`
  (`browser-pw-offer` is the save / fill offer), `browser-bar`,
  `browser-address`, `browser-nav--forward`, `browser-frames`, `browser-frame`,
  `browser-shield` (`.is-on`/`.is-off`), `browser-downloads-panel`,
  `browser-downloads-head`, `browser-downloads-title`, `browser-downloads-clear`,
  `browser-downloads-list`, `browser-downloads-badge`, `browser-download`,
  `browser-download is-completed` (state class), `browser-menu`,
  `browser-history-panel`, `browser-bookmarks-panel`, `browser-status`.
- The toolbar controls are **core components** (`iconButton`, `searchBar`,
  `icon`); `plugin.css` only arranges them (geometry, no colours). Back/forward
  share `ui/arrow-left` and forward is mirrored in CSS (`browser-nav--forward`).
- **Look.** The window's colours, type and surfaces are core CSS:
  `web/css/tiles.css`, section "Browser plugin window" (selectors prefixed
  `.browser-tile` so they outrank the later-loaded plugin sheet). The editorial
  skin: hairline under the bar and strip, no glass plate; tabs are flat with
  mono labels and the active one is a grey pill (`--ed-pill`); popovers, the
  downloads panel and the status strip carry `.hud-menu`, which gives them the
  theme's dark-island token scope (the top-bar status menus'), with flat dark
  glass (`--ed-bar-bg`), no drop shadow and mono uppercase headings. The accent
  is kept for small signals (shield on, bookmarked star, downloads badge and
  progress, the search engine's check). History, bookmarks and downloads rows
  cascade in with `listReveal` when their panel opens (not on every
  re-render).
- The home surface is a sandboxed iframe: `sandbox="allow-scripts"` with
  `referrerpolicy="no-referrer"`, `srcdoc` set to
  `homeDocument(news, bookmarks, { palette, animate })`. Its inline click
  handler is allowlisted by an exact CSP hash
  (`web/js/tests/cspHomeShelfHash.test.mjs`) — restyling the page never
  touches that script.
- **Theming the home surface.** A sandboxed document inherits no custom
  properties, so `homePalette()` reads the theme's resolved values from the
  shell's root (`--bg`, `--bg-elevated`, `--text`, `--text-secondary`,
  `--muted`, `--accent`, `--accent-contrast`, `--ed-pill`, `--ed-pill-hover`,
  `--ed-pill-dark`, `--ed-pill-dark-ink`, `--font-display`, `--font-body`,
  `--font-mono`) plus the theme mode and the motion language, and writes them
  into the page's own `:root`. The page background is transparent (the
  window's surface shows through): the frame gets the same `color-scheme` as
  the page and the viewport drops its white canvas while the active frame
  carries `data-home`. `theme:change` / `appearance:change` →
  `refreshHomeTheme()` re-reads the palette a few times (a theme's stylesheet
  lands after its event) and rebuilds open home tabs from `tab.homeData`
  without refetching and without the entrance cascade.
- **Home layout.** A mono date eyebrow, a 34 px display title ("Home") and a
  mono meta line (`N bookmarks · N stories`); bookmarks as grey square tiles
  (favicon, letter fallback) with mono captions; the news as hairline rows with
  a grey thumbnail tile, mono `SOURCE · 2H · TOPIC` meta, headline and snippet;
  Refresh is a grey pill whose arrow is CSS (`::after`), so the handler's
  "Refreshing…" text swap keeps it. Blocks enter in a CSS cascade inside the
  frame (`.rv` with `--i`), following `data-motion` (kinetic wipe, fluid rise,
  depth blur) and `prefers-reduced-motion`.
- A bookmark tile posts `browser:open-bookmark`. A start page is a tab
  waiting for a destination, so the bookmark opens **in that tab**.
  `openInNewTab(from, url)` reuses the tab it comes from (or, with none, the
  active tab) whenever that tab is a blank start page, which covers the grid,
  the bookmarks popover and links handed over by core; anywhere else it opens a
  new tab. News cards always navigate their own tab.

## Address bar suggestions

`address-suggest.js` (`createAddressSuggest`) completes the address bar from
`GET /api/browser/suggest` on every keystroke:

- **Inline completion.** Typing `you` fills in `youtube.com` with the added
  tail selected: Enter goes there, the next letter replaces it. Only the tail is
  added, so the typed letters (scheme, `www.`, capitals) stay as typed.
  Deleting or pasting never completes. A reply for older text is dropped.
- **The list** (`.browser-suggest`, one of `browserPopovers()` so the native
  page is masked under it) hangs under the address field (`placeSuggest`):
  bookmarks (star), visited pages (clock) and past searches (magnifier).
  ArrowDown/ArrowUp move through it, Enter or a click opens the row (a past
  search is searched again), Escape drops the inline tail and closes it, and
  leaving the bar closes it.

## Custom events and globals

### Listens

| Event | Source | Effect |
|---|---|---|
| `artifact:saved` | core artifact store | `onArtifactSaved` navigates to `art.narrative` (falls back to `payload.url`/`params.url`/`title`) for `artifact_type === 'browser_page'` |
| `url:open` | core link router (`web/js/linkRouter.js`) | `openCoreUrl` — a link clicked anywhere in the app opens as a new tab, or in the active tab when that is a blank start page |
| `message` | sandboxed home surface | `browser:open-card` → `news/click` POST + navigate; `browser:open-bookmark` → open in the start page's own tab; `browser:refresh-news` → reload the shelf |
| `theme:change` | core theme | `pushSettings()`; `refreshHomeTheme()` repaints open home tabs |
| `appearance:change` | core appearance (accent) | `refreshHomeTheme()` |
| `settings:change` | core Settings window (browser section) | `onSettingsChanged` — re-fetches `/api/browser/settings`, re-pushes to the shell, and reloads the active page when `adblock` changed |
| `resize`, `desktop:changed`, `overlay:open`, `fullscreen:change`, `menu:change` | core | `requestSync()` for native-view bounds/mask |

### Global callbacks the shell calls

| Global | Payload | Purpose |
|---|---|---|
| `window.__liteauraViewEvent(event)` | `{ id, type, ... }` | Per-tab `title`, `url`, `load` (`phase: started/finished`), `new-window`, `context-menu` (`menu` = `{x,y,linkUrl,editable,selection}`), `page-js` (`token`, `result` — the answer to `runJs`); and non-tab `download` events (`applyDownload`) |
| `window.__liteauraShield(state)` | `{ enabled, blocked, rules }` | Updates the shield button title / `browserSettings` |
| `window.__liteauraDownloads(list)` | `Download[]` | Replaces the live downloads list (answer to `liteaura:downloads:list`) |

### Dispatches / posts

- The sandboxed shelf `parent.postMessage(...)`s `browser:open-card` and
  `browser:refresh-news`; the window handles them in `onFrameMessage` and
  matches `event.source` against each tab's `frameEl.contentWindow` so the
  **tab that sent it** updates, never the active one.

## IPC bridge (`window.ipc.postMessage`)

`nativeAvailable` is true only when `window.ipc.postMessage` exists. Helpers:
`nativeSend` (prefixed `liteaura:view:`), `shellSend` (prefixed non-view JSON),
`shellRaw` (raw string).

| Prefix | Payload | Purpose |
|---|---|---|
| `liteaura:view:` | `{ op, id, … }` | Native child-webview commands |
| `liteaura:settings:` | `{ adblock, downloadsDir, colorScheme }` | Shell settings. `colorScheme` (`"dark"` for Ink, `"light"` for Dusk) is what pages report as `prefers-color-scheme`: a site with a dark style uses it, one without stays as its author made it (no automatic darkening). Chromium takes it only at startup (`--blink-settings=preferredColorScheme`), so the shell saves it in `<data dir>/qtwebengine/color-scheme` and pages follow a theme switch from the next start of the shell. Core `web/ui/theme-loader.js` sends it on every boot and switch; this window re-sends it on `theme:change` / `appearance:change`. |
| `liteaura:filter:` | `{ enabled }` | Live shield toggle |
| `liteaura:download:` | `{ id, action }` | `pause`/`resume`/`cancel`/`remove`/`clear` |
| `liteaura:downloads:list` | raw (no JSON) | Ask the shell to push its live list via `window.__liteauraDownloads` |
| `liteaura:shield:query` | raw (no JSON) | Ask the shell to push the shield state via `window.__liteauraShield` (polled while mounted, see Shield and ad blocking) |

`liteaura:view:` ops (parsed by `crates/liteaura/src/browse.rs` `Command::parse`):
`open` (`url`, `rect`, `visible`, `incognito`), `navigate`, `setBounds`,
`setVisible`, `setMask` (`holes`), `back`, `forward`, `reload`, `close`,
`focus`, `paste` (`text`), `action` (`verb` — one engine action on the page's
focused element: `Cut` / `Copy` / `Paste` / `SelectAll`), and `runJs` (`token`,
`script` — evaluate a script in the page and answer with a `page-js` view event
carrying the same token and the result; the password manager's fill/read go
through `runPageJs`). `setMask` (`holes`) is input only: the app view is drawn
above the page.

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
| `GET /api/browser/vault`, `POST …/vault/{unlock,lock,reset}`, `GET /api/browser/passwords`, `POST …/passwords/{reveal,save,remove}` | the password manager (`passwords.js`) |
| `GET /api/browser/bookmarks`, `POST …/add`, `POST …/remove` | bookmarks panel and the star toggle (a toast confirms the save or says it failed; a failure is also logged with its reason) |
| `GET /api/browser/suggest?q=` | the address bar's completion (`address-suggest.js`) |
| `GET /api/browser/import/sources`, `POST …/import/{profile,file}` | the import panel (`import-panel.js`) |

## Tab model

Each tab is `{ id, sessionId, title, url, home, incognito, native, history,
historyIndex, loaded, watchdog, frameEl }`, plus `pageLoading` (between the
`load` events), `loginWatch` (`{ origin, kind }` of the watched form) and
`loginOffer` (a pending save / fill offer) for `login-assist.js`; a home tab also keeps
`homeData` (`{ news, saved }`, so a theme change repaints without
refetching) and `homePalette` (the resolved tokens it was drawn with).

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
  own popovers (`browserPopovers()`, the tab switcher and password manager
  included) and any higher tile in the "Windows" layout;
- re-sends the mask on the way back from a hide, because a stale hole would
  otherwise clip the page once it moves.

Hiding a page never suspends one the engine still needs: the shell freezes a
hidden page only where `recommendedState` (and `recentlyAudible`) say it may, so
a background tab that is playing audio in the "keep alive" sense — a YouTube tab
you switched away from, a window left in another workspace — keeps playing
exactly like a background tab in a normal browser. Idle hidden pages are still
frozen (see `kPageQml` in `crates/liteaura/shim/liteaura.cpp`).

`requestSync()` coalesces bursts into one sync per animation frame;
`startBoundsSync()` adds a 1 s fallback interval, a `ResizeObserver` on the
frame wrapper, `MutationObserver`s on the tile grid and body class, the core
occluder watcher (which catches a panel's delayed `.hidden` class), and the
window events listed above. `stopBoundsSync()` tears them all down.

## Tab strip and switcher

The strip lives in the window's title bar and is the shared one
(`tabStrip` / `tab` from `web/ui`, the Terminal's too): flat mono capitals,
the active tab a pill, a tab's close on hover, on the active tab and on focus.
The active tab scrolls into view when the strip overflows. A
`.browser-tabcount` button (the open-tab count, as phone browsers show it) opens
the **tab switcher** (`.browser-tabs-panel`): one large row per tab (title,
address, close), plus New tab / New incognito tab.

- In a narrow window (`@container plugin-window (max-width: 560px)`) the strip
  keeps only the active tab at full width next to the count, and every popover
  spans the window.
- In a window up to 700px or on a phone (`applyBrowserCompact`, the width
  every window bar goes compact at) the toolbar is one row — back, reload,
  the address, the menu — and forward, home, bookmark, passwords, shield,
  downloads and incognito move, as the same buttons, into a row of icons at
  the top of the browser menu (`.browser-menu-actions`); the menu's own
  Passwords row steps aside while the lock is there.
- The popovers hang under the toolbar wherever the title row put it
  (`--browser-pop-top`, measured on resize and before a panel opens).
- On touch (`pointer: coarse`) tabs are 36 px tall (the shared component's
  rule) and close buttons, rows and popover actions grow to finger size.
- On a phone (`html[data-phone='1']`) the window has one header row: the
  tab strip is hidden and `placeBar` moves the address row into the merged
  title row — back, reload, the address, the open-tab count
  (`.browser-tabcount--bar`) and the menu. The tab switcher, history,
  bookmarks, downloads, the password manager and the import are full-height
  panes over the page (`paneStack`, floor = `.browser-viewport`), each with
  the shared Back in its `[data-pane-bar]` head; Escape and the phone's Back
  pop them (`panelOpen` / `showPanel`). The menu and the login offer stay
  popovers; wide windows keep every popover. Every panel action is an icon
  with its label, the icon alone on a phone.

## Passwords

`web/passwords.js` builds the password manager popover
(`.browser-passwords-panel`), opened from the toolbar's lock button, the
overflow menu, or "Fill password…" in the page's right-click menu on a field.
It talks to the vault routes ([`routes.md`](routes.md#password-vault)) and
reaches the page only through two callbacks this window passes in:

- `fillPage(user, pass)` → `runPageJs(tab, fillScript(origin, user, pass))`.
  The script checks `location.origin` **inside the page** before touching a
  field, so a navigation between the click and the fill never hands the
  password to another site; it sets fields through the native `value` setter
  and raises `input`/`change`. Answers: `ok`, `user` (a username-first
  sign-in), `nofield`, `origin`. A filled page gets the keyboard (`focus`).
- `readPage()` → `readScript()`: what the user typed, for "Save from page".

`runPageJs` sends a `liteaura:view:` `runJs` with a fresh token and resolves on the
matching `page-js` view event (4 s timeout, `''` on none).

Views: locked (login-password prompt; first use creates the vault; after a
login-password change it asks once for the previous one; Reset deletes
everything), list (this site first, then all; click fills, copy uses
`copyText(…, { sensitive: true })`), edit (site, username, password with a
show toggle and a strong-password suggestion, note). `openWith(values)` opens
the editor prefilled — after the unlock prompt when the vault is locked.

### On its own: save, fill, suggest

`web/login-assist.js` does what a browser's password manager does without
being asked, from the tab's view events:

| Event | What happens |
|---|---|
| `load` `finished` | `takeScript` hands over a login the previous page of the same origin kept in `sessionStorage` (a form that posted to itself); `watchScript` installs the form watcher and answers the form's kind. A **login** form gets the site's saved login filled in when there is exactly one for that very host (not a parent domain's, which the panel still offers on a click) and its fields are empty; with several for that host (often after an import) it says so once per site and session and its "Fill password…" opens the panel to pick one (`fillScript(…, { onlyEmpty: true })`, once more after 1.5 s for a form drawn late); with the vault locked it offers "Unlock" once per site. A **sign-up** form (`autocomplete="new-password"`, or two password fields) gets "Use a strong password?": `generatePassword()` (20 characters, every class, no look-alikes, `crypto.getRandomValues`) goes into the password and its confirmation (`suggestScript`). |
| `load` `started` | The page is being left: `takeScript` reads what the watcher kept (a submit, a click on a form button, Enter in a field, `pagehide`). |
| `url` without a load | A single-page app moved on: the same, then the form is watched again. |

A login is offered only for the origin the window had loaded (a page's own
claim must match it), only when it is new or its password changed ("Save" /
"Update" — the server updates the entry with the same origin and username),
never from an incognito tab and never for a site marked "Never" (kept in
`localStorage`, `browser.passwords.never`). "Save" with the vault locked opens
the unlock prompt and then the editor, prefilled. A background tab keeps its
offer until the user switches to it. The vault still locks after its idle
time and with the server: the fill waits for an unlock.

## Import

"Import from another browser…" in the main menu opens `web/import-panel.js`
(`.browser-import-panel`): the browser profiles found on this computer, one
row each ("Firefox · default-release", with what it brings) that imports its
history and bookmarks, and "From a file…", which picks a password CSV or a
bookmarks HTML with the shared file picker (`pickFiles`) and sends its text.
A password file with the vault locked opens the unlock prompt; once it is
open the window comes back to the import, which finishes by itself. After
passwords the panel reminds the user to delete the CSV. Bookmarks are
reloaded after every import. Routes: [`routes.md`](routes.md#import-from-another-browser).

## Right-click inside a page

A right-click in a page opens the app's own menu over the page — never a
platform menu. The shell forwards the request as a `context-menu` view event
(it must be in `ViewBus::FORWARDED`, `crates/liteaura/src/browse.rs`, like every
view event the plugin hears)
(`{x, y, linkUrl, editable, selection}`; the position is local to the page item,
so the plugin adds the frame origin and divides by `dpr`), and the plugin opens
core's `openContextMenu()` at the cursor. The `.ctx-menu` mask hole keeps it
visible and clickable (see above). Entries map back to `liteaura:view:` commands:
`back` / `forward` / `reload`, Copy Link Address through the clipboard service
(so it lands in the top-bar history), and the edit verbs — Cut / Copy / Paste /
SelectAll — via the generic `action` op. Below them comes the same window
menu every app shows on a right-click (core's `pluginWindowMenu()`: window
verbs plus `browserContextMenu()`). Only the active tab's page opens a menu.
Core's menus ignore the app window's `blur` for a moment after opening: the
right-click moves focus into the page, and that blur must not close the menu
it raced with.

## Gotchas

- **A page view outlives a reload of the app.** It belongs to the shell, not
  to the document that opened it, and a new document (an update reload, a
  crash recovery) restarts its tab ids at `t1`. Two guards close the old
  ones: the shell closes every page view when the app view starts loading
  (`Command::CloseAll`, `crates/liteaura/src/browse.rs`), and `nativeSend`
  keeps the open ids in `sessionStorage` (`web/js/nativeViews.js`) so the new
  document closes what is left at boot (`closeOrphanNativeViews`) — for a
  shell without the first guard. Without them a page stayed on screen,
  neither following its window nor closing with it.
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
  `/api/browser/downloads` rows and live `window.__liteauraViewEvent` events, by
  `id`. Clearing the panel must delete finished rows server-side too.
- **CSS files are not auto-injected**; `ensureStylesheet()` is mandatory in
  `mount`.

## Smoke test

`plugins/browser/web/plugin.smoke.mjs` runs the real module against a DOM shim
and asserts: mount/exports, the toolbar controls, settings push, one sandboxed
home tab with news, navigation POST + `liteaura:view: open`, `setBounds`/`setMask`,
the shield (state + `liteaura:filter:` + persistence), incognito tabs, downloads
events + badge + panel mask hole, a core occluder (the chat-panel case) punching
and clearing its hole, drag hide/restore, tab close, AI `artifact:saved`
navigation, the clipboard contract (context-menu entries,
`clipboard:paste` → `liteaura:view: paste`, picks for other windows ignored), the tab
switcher, and the password manager's wiring (`runJs` bound to the page origin, the
`page-js` answer, focus after a fill). `web/passwords.test.mjs` covers the page
scripts themselves.

```bash
node --experimental-vm-modules plugins/browser/web/plugin.smoke.mjs
```

## Cross-references

- [`README.md`](README.md) · [`architecture.md`](architecture.md) · [`tools.md`](tools.md) · [`routes.md`](routes.md)
- [`../../../web/js/nativeOccluders.js`](../../../web/js/nativeOccluders.js) — the core surfaces that punch through a page.
- [`../../../crates/liteaura/src/browse.rs`](../../../crates/liteaura/src/browse.rs) — the IPC contract.
- [`../../../docs/core/desktop.md`](../../../docs/core/desktop.md) — tile/window host.
