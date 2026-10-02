# Desktop (window manager)

Shiny's desktop is a Hyprland-style window manager that runs entirely in the
browser. `web/js/desktop.js` owns the *state* — workspaces, focus, fullscreen
and the tiling layout — while `web/js/tiles.js` owns the *DOM*: it mounts one
tile window per plugin surface and asks `desktop.js` how to arrange them. The
two are deliberately separate: layout math never touches plugin markup, and
plugin markup never decides where it goes.

The desktop is scoped per traveler (see [web-ui.md](web-ui.md) and
`web/js/preferences.js`), so workspaces, focus, fullscreen and floating-window
geometry are all restored per account. On a vertical phone-shaped screen
(`MOBILE_PORTRAIT_QUERY`) the whole workspace system stands down and the
desktop becomes a single column of windows — a *view* over the stored
arrangement, never a rewrite of it.

## Module map

| Module | Role |
|---|---|
| `web/js/desktop.js` | Workspaces, focus, fullscreen, layout engine, keybindings, workspace bar |
| `web/js/tiles.js` | Plugin-window DOM, core windows, map tile, window chrome, catalogs |
| `web/js/fullscreen.js` | Real (browser) fullscreen, dedicated workspace, chrome reveal |
| `web/js/fullscreenBubble.js` | Top-bar bubble for a fullscreen window (close / exit) |
| `web/js/coreWindows.js` | Registry of built-in windows (`settings`, `plugins`) |
| `web/js/overview.js` | Mission-Control window grid (three-finger swipe up) |
| `web/js/launcher.js` | Full-screen plugin launcher (three-finger swipe down) |
| `web/js/gestures.js` | Trackpad gesture → desktop action vocabulary |
| `web/js/contextMenu.js` | Right-click / long-press menus and the popup engine |
| `web/js/viewport.js` | Breakpoint queries (phone / mobile portrait) |
| `web/js/preferences.js` | Per-user storage for all of the above |

## Window lifecycle

A *window surface* is a module with a small, stable contract. `tiles.js` calls
it lazily, once per window, and only while the surface is wanted.

```
mount()        → HTMLElement     create (or return) the window's root element
unmount()      → void            tear the element down and drop listeners
getElement()   → HTMLElement|null the currently mounted element
wireEvents()   → void            one-time global listeners (optional)
contextMenu(ctx) → Entry[]       app entries spliced into the window menu (optional)
selectSection(id) → void         deep-link a section (optional, core windows)
```

### Plugin surfaces

A bundled plugin with a window ships `plugins/<name>/web/plugin.js`, which
`tiles.js` imports dynamically (`import(\`../plugins/${p.name}/plugin.js\`)`).
The import specifier is *relative on purpose*: a root-absolute `/plugins/…`
would resolve against the kiosk proxy origin and the window would never mount.
Only plugins whose session-active set includes them **and** whose catalog entry
has `surface: true` are loaded. `refreshCatalog()`:

1. calls `refreshActivePlugins()` (`web/js/activePlugins.js`) for the session set;
2. fetches `/api/plugins` for the catalog (description/category/version/surface);
3. dynamically imports each wanted surface, stores the module, calls
   `wireEvents()`;
4. unmounts and forgets surfaces that are no longer wanted;
5. re-delivers the last `agent:actions` payload if a surface was wired late, so
   a plugin activated in the same agent response as its tool call still sees it.

`deactivatePlugin(name)` POSTs `/api/plugins/deactivate`, then writes the
`plugins.changed` localStorage stamp and dispatches `plugins:changed`.
`activatePlugin(name)` does the same against `/api/plugins/activate` and adds
`detail.opened` so the desktop can focus (raise) the new window.

### Core windows

Built-in windows live in `web/js/coreWindows.js`:

```js
export const CORE_WINDOWS = {
  settings: { title: 'Settings', icon: 'ui/settings', load: () => import('./settingsWindow.js') },
  plugins:  { title: 'Plugins',  icon: 'ui/puzzle',   load: () => import('./pluginsWindow.js') },
};
```

They behave exactly like plugin surfaces once opened (tile, focus, fullscreen,
workspaces, context menu) but are *opened/closed* instead of
activated/deactivated. `openCoreWindow(name)` adds the name to `coreOpen`,
lazily imports the module through `ensureCoreSurface()`, re-renders and focuses.
`openCoreWindowSection(name, section)` additionally calls `selectSection()`.
`closeCoreWindow(name)` exits fullscreen first (core windows have no
`plugins:changed` signal), unmounts the surface and re-renders. Core windows are
listed in `surfacePlugins()` even while their module is still importing, so a
restored window is never pruned from its saved workspace before it mounts.

### Window chrome

`ensureWindowChrome(el, name)` runs once per element and prepends:

- the tier-0 **ambient glow** (`installGlow`, see [themes-icons.md](themes-icons.md));
- a `.tile-header` holding `.tile-header-controls` — **close** (deactivate or
  close a core window) and **full screen** (`toggleWindowFullscreen`);
- a centered `.tile-header-title` (or, when the plugin nominates a top bar with
  `data-window-bar`, `tile-header--merged` merges the controls into that bar);
- a `.tile-resize` grip (visible only in the Windows layout).

`syncWindowChrome()` mirrors the in-app fullscreen state onto the full button.
The title bar doubles as the drag handle in Windows layout; `DRAG_IGNORE`
keeps controls, inputs and ARIA roles from starting a drag.

## Tile grid

`#tile-grid` is the fixed desktop surface under the HUD. Windows are
`.tile[data-plugin]` children with the same element identity across renders:
`renderTiles()` reconciles in place rather than wiping and re-appending, because
detaching a tile reloads any `<iframe>` inside it (a YouTube player would
restart). Classes on the grid select the layout:

| Grid class | Meaning |
|---|---|
| `tile-grid--master` | Tiling: master pane + stacked windows |
| `tile-grid--columns` | Legacy flex-wrap column layout |
| `tile-grid--windows` | Floating, draggable, resizable windows |
| `tile-grid--single` | Exactly one visible window |
| `tile-grid--phone` | Phone breakpoint: one window at a time |

`renderTiles()` also detects newly-opened names (`opening`) and calls `bumpZ`
*before* layout so a reopened window lands on top; toggles `tiles-active` on
`<body>`; renders the workspace bar; re-emits `artifact:dock`; nudges Leaflet
with `map:resize`; and focuses the last opened window. Workspace switches
animate with `animateWorkspaceSlide(dir)` (Web Animations API, skipped under
`prefers-reduced-motion`).

## Layouts

`getDesktopLayout()` (from `web/js/preferences.js`) returns the merged,
clamped layout config; `setLayout(patch)` persists and notifies. The default is:

```js
const DEFAULT_DESKTOP_LAYOUT = {
  mode: 'master',       // Tiling | 'columns' | 'windows'
  master_ratio: 0.7,    // master fraction (0.25–0.85)
  orientation: 'left',  // 'left' | 'right' | 'top' | 'bottom'
  gap: 8,               // px between windows (0–40)
  stack_weights: {},    // pluginName -> relative stack track size
};
```

`applyLayout(grid, items)` removes the previous layout classes, clears the
splitter overlay, then branches:

- **Windows** (`mode === 'windows'`) — `applyWindowsLayout()` sets
  `display:block` and positions each `.tile--window` in pixels from
  `getWindowsGeom()`. Geometry is `{ x, y, w, h, z }` per plugin, clamped to
  `WIN_MIN_W = 280`, `WIN_MIN_H = 200` and the grid box. Each window carries a
  stacking order (`z`); `bumpZ(name)` raises one without a DOM pass and persists
  on a 250 ms debounce (`saveGeomSoon`/`flushGeom`). Dragging is compositor-only
  (`transform: translate`, one rAF per frame) and resizing writes width/height;
  both commit on pointer-up. `wireWindowInteractions()` raises on pointerdown.
- **Tiling** (`mode === 'master'`) — the focused window (or the first) is the
  **master**; the rest are the **stack**. Orientation `top`/`bottom` puts the
  master on a full-width row and the stack in columns; `left`/`right` puts the
  master in a full-height column and the stack in rows. `master_ratio` sizes the
  seam; `stack_weights` sizes each stack track.
- **Columns** (`mode === 'columns'`) — falls back to the CSS flex-wrap layout
  in `web/css/tiles.css`; `applyLayout` just marks focus.
- **Fullscreen / single** — when a window is fullscreen, or only one is visible,
  the grid becomes a single 1×1 track and the visible tile gets `.tile--full`.

A vertical phone screen forces `mode: 'columns'` in `getDesktopLayout()` without
touching the stored value, so the choice returns with the screen.

### Splitters

In Tiling mode an invisible overlay (`grid.__splitterLayer`) hosts drag handles:

| Handle | Drag axis | What it changes |
|---|---|---|
| Master seam | across the split | `master_ratio` (clamped 0.25–0.85) |
| Stack gap *i* | along the stack | the two adjacent `stack_weights` |

`SPLITTER_MIN_HIT = 12` keeps a grab area even on a thin gap; `STACK_MIN_SIZE
= 80` is the smallest a stacked window may shrink to. The overlay is rebuilt by
`buildSplitters()` and re-positioned on resize by `repositionSplitters()`.

## Workspaces

A workspace is `{ id, windows: [pluginName], focus, fullscreen, name? }`. The
manager tracks `workspaces`, `activeWs`, and caches the active workspace's
`focus` / `fullscreen`.

- `ensureWindows(names)` migrates a first load (one workspace with everything),
  prunes deactivated surfaces, repairs the fullscreen rule and adds newly
  activated windows to the active workspace. It deliberately does nothing when
  `names` is empty on the first render, so a reload does not wipe saved
  workspaces before the active set has loaded.
- `reconcile()` enforces the **fullscreen rule**: a fullscreen app must live in
  a workspace by itself. A stray window that breaks the rule is moved to a
  shelter workspace; a fullscreen app whose workspace no longer holds it is
  forgotten. It is idempotent and runs from every mutation and render.
- `workspaceLock(ws)` returns the plugin that seals a workspace (its fullscreen
  app) or `null`. A sealed workspace accepts nothing else and its app cannot
  leave, so no window can ever be tiled behind a fullscreen one.
- `workspaceLabel(wsOrIndex)` names a workspace; a dedicated fullscreen
  workspace is just the app's name ("Youtube", not "Workspace 3 — Youtube").
- `activeWindowNames(allNames)` lists the active workspace's still-existing
  windows in order. On a phone it returns every name (one column, no workspaces).

### Mutations and helpers

| Function | Effect |
|---|---|
| `createWorkspace()` | Push an empty workspace and switch to it |
| `removeWorkspace()` | Merge the active workspace into a neighbour (refuses the last) |
| `dropWorkspace(id)` | Drop an *empty* workspace (fullscreen.js cleanup) |
| `switchWorkspace(dirOrIndex)` | `'next'`, `'prev'` or a 0-based index |
| `moveWindow(name, toId)` | Move then focus; refuses into/out of a sealed workspace |
| `moveWindowByIndex(name, idx)` | `idx` may be `'new'`; out-of-range auto-creates (capped at 9) |
| `focusWindow(name)` | Focus, switching workspace if needed, and raise |
| `cycleFocus(names, dir)` | Rotate focus through the active workspace |
| `toggleFullscreen(name, force)` | In-app fullscreen state |
| `isolateInNewWorkspace(name)` | Desktop half of real fullscreen; returns a token |
| `restoreFromNewWorkspace(name, token)` | Undo the isolation, dropping the dedicated workspace |

`getDesktopSnapshot()` (1-based indices) is sent to the AI on every request so
it knows which windows live in which workspace; the identical `workspace_*` and
`desktop_*` tools are applied back through `agent:actions`.

### Workspace bar

`renderWorkspaceBar()` draws `#workspace-bar`: a `+` button, the dots, and a `−`
button. Each dot is the workspace number, or the **app's own icon** for a
dedicated fullscreen workspace (`.is-app`); the active dot carries `.is-active`.
The bar is always visible on a wide screen (even with no windows, so workspaces
can be created/managed) and hidden on a phone. It is re-rendered on every
`desktop:changed` event.

## Keyboard shortcuts

`wireShortcuts()` binds these on `document` (`Alt` is the "Super" mod). Keys are
ignored while a text field is focused, and `Ctrl`/`Meta` combos are left to the
browser.

| Shortcut | Action |
|---|---|
| `Alt+Enter` | Toggle real fullscreen for the focused window (`app:fullscreen-toggle`) |
| `Alt+1`…`Alt+9` | Jump to workspace 1–9 |
| `Alt+H` / `Alt+L` | Cycle focus to the previous / next window |
| `Alt+,` / `Alt+.` | Previous / next workspace |
| `Alt+N` | New workspace |
| `Alt+Shift+N` | Remove the active workspace |

`Alt+Enter` only announces the gesture; `fullscreen.js` owns the action, so the
shortcut and the window's own button agree. The same operations are reachable
from the context menu and the Settings → Desktop page, and are mirrored for the
AI through `desktop_fullscreen`, `desktop_focus`, `workspace_create`,
`workspace_remove`, `workspace_switch` and `workspace_move`.

## Real fullscreen and the bubble

`web/js/fullscreen.js` implements the user-facing fullscreen, which is more than
the browser API. `enterWindowFullscreen(name)`:

1. calls `isolateInNewWorkspace(name)` — a brand-new dedicated workspace;
2. calls `toggleFullscreen(name, true)` — the in-app state, which doubles as the
   fallback when the browser refuses the request;
3. stores `session = { plugin, token }`, adds `body.fs-active`, announces
   `fullscreen:change`, then requests real fullscreen on `document.documentElement`
   **synchronously** from the user gesture (a rejection is not an error — the
   in-app state stands).

While a top-docked fullscreen window is up, the top bar is away whatever
`autohide_bar` says; it comes back from the top edge once the pointer insists
there, and `#tile-grid` slides down by `--hud-header-total` so the app's own top
bar is never covered. `exitWindowFullscreen()` leaves browser fullscreen, hands
the workspace back via `restoreFromNewWorkspace`, and announces the change.
Escape, the button, and the browser's own exit all funnel through
`onFullscreenChange()`.

The window gives up its own title-bar stripe (`body.fs-active .tile--full >
.tile-header { display: none }`), except a *merged* bar that is really the
plugin's toolbar. Its close / exit controls move into `#fullscreen-bubble`
(`web/js/fullscreenBubble.js`), a glass pill on the left of the top bar that
rides the same top-edge reveal. `build(name)` renders **Deactivate/Close** and
**Exit full screen**, reusing `deactivatePlugin` / `closeCoreWindow` /
`exitWindowFullscreen`.

### Chrome reveal

`fullscreen.js` also owns the desktop-wide bar/voice-bar autohide. Cached
immersion settings (`getImmersive()`: `autohide_bar`, `bar_position`,
`autohide_orb`) are published to `<html>` by `applyImmersive()` as
`data-bar-pos`, `data-autohide-bar`, `data-autohide-orb`. Constants:

| Constant | Value | Meaning |
|---|---|---|
| `EDGE_ZONE` | 120 px | keep a shown bar up |
| `REVEAL_ZONE` | 28 px | only this close summons it |
| `BAR_DWELL_MS` | 700 ms | pointer must *stay* in the reveal strip |
| `ORB_ZONE` | 200 px | bottom band that reveals the voice bar |
| `ORB_HALF` | 340 px | …within this many px of centre |
| `ENTER_HOLD_MS` | 1600 ms | show chrome briefly after entering fullscreen |

A fullscreen window overrides the bar's autohide for the top position; other bar
positions, and every other workspace, keep the desktop-wide preference. On touch
screens (`@media (hover: none)` in `web/css/fullscreen.css`) the bars simply
stay visible — there is no hover to bring them back.

## Overview and launcher

Both are plain web layers raised above the chrome and are opened by the native
trackpad gestures. `web/js/gestures.js` maps a `trackpad:gesture` direction:

| Gesture | Action |
|---|---|
| swipe left | next workspace |
| swipe right | previous workspace |
| swipe up | window overview |
| swipe down | plugin launcher |

While an overlay is open a vertical swipe only *pulls it back*; opening the
other overlay needs a fresh swipe. `window.__shinyGesture(dir)` is a console
hook for testing without the native reader.

**Overview** (`web/js/overview.js`) lays out *every* mounted window from every
workspace as a live grid — the real tiles resized in place, never clones, so a
running video or half-typed document survives. `shape()` picks the column count
nearest a 16:10 cell with a full last row (`PAD = 40`, `GAP = 16`). Each window
gets a `WS n` badge; focusing one switches to its workspace and closes the
overview. It exits real fullscreen first (which it cannot represent honestly)
and blurs focus so typing cannot reach a window behind the grid. Escape or a
backdrop click also closes.

**Launcher** (`web/js/launcher.js`) is a full-screen app grid over the blurred
desktop — the top-bar tray at full size, same `/api/plugins` list and same
activate/focus paths. Tapping an inactive plugin activates it; tapping an active
one focuses its window. A running app gets a dot under its name.

---

## Built-in windows

Settings (`web/js/settingsWindow.js`), Plugins (`pluginsWindow.js`) and Chats
(`chatHistory.js`) are built-in windows, not plugin tiles. They share the window
chrome and the UI library, and are opened from the right of the HUD.

## Preferences

Desktop state is stored per user via `/api/preferences`:

| Key | Effect |
|---|---|
| `desktop.layout` | `tiling` / `columns` / `windows`. |
| `desktop.workspaces` | Workspace count / assignments. |
| `session.remember` | Restore the previous desktop on sign-in. |
| `ui.theme.name` | Active theme. |
| `appearance.*` | Accent + gradient. |
| `desktop.surface.window_background` | The blurred window glow. |
| `power.*` | Power mode + low-power AI. |
| `remote.autostart` / `remote.enabled` | Remote access. |

See [`web/js/preferences.js`](../../web/js/preferences.js) and
[configuration](../configuration.md).

## Theming & notifications

Windows are styled by the active theme's tokens/components and use the shared
icon library; see [themes & icons](themes-icons.md). Events use the core
notification system ([`web/ui/components/notifications.js`](../../web/ui/components/notifications.js))
and the `notify()` helper.

## Source map

| Path | Role |
|---|---|
| [`web/js/desktop.js`](../../web/js/desktop.js) | Workspace/layout model. |
| [`web/js/tiles.js`](../../web/js/tiles.js) | Tile grid + window chrome + glow. |
| [`web/js/fullscreen.js`](../../web/js/fullscreen.js), [`fullscreenBubble.js`](../../web/js/fullscreenBubble.js) | Fullscreen + chrome reveal. |
| [`web/js/overview.js`](../../web/js/overview.js), [`launcher.js`](../../web/js/launcher.js) | Overview + launcher. |
| [`web/js/gestures.js`](../../web/js/gestures.js) | `trackpad:gesture` mapping. |
| [`web/js/contextMenu.js`](../../web/js/contextMenu.js) | Right-click engine. |
| [`web/js/hudLeft.js`](../../web/js/hudLeft.js), [`hudPlugins.js`](../../web/js/hudPlugins.js), [`hud*.js`](../../web/js) | HUD sections + chips. |
| [`web/js/keyboard.js`](../../web/js/keyboard.js) | Virtual keyboard chrome. |
| [`web/js/settingsWindow.js`](../../web/js/settingsWindow.js), [`pluginsWindow.js`](../../web/js/pluginsWindow.js) | Built-in windows. |

Related: [web UI](web-ui.md) · [themes & icons](themes-icons.md) ·
[host panels](../host/README.md).
