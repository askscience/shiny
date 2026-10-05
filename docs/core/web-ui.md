# Web UI

The frontend is a **no-build ES-module app** served from `web/`. There is no
bundler: [`web/index.html`](../../web/index.html) loads the shell CSS, a
synchronous theme bootstrap, and then the module graph under
[`web/js/`](../../web/js). All `/api/*` HTML/JS/CSS is served with
`Cache-Control: no-store`, so a server restart or source edit is picked up on
the next reload.

---

## Boot sequence

1. **Theme bootstrap** (inline script in `index.html`) reads the stored theme
   from `localStorage` (`ui.theme.name`), validates it against the cached theme
   index, sets `data-theme` on `<html>`, and points `#theme-tokens` /
   `#theme-components` at the right CSS — before first paint, to avoid a flash.
2. The **boot splash** (`#boot-screen`) is painted; `web/js/bootScreen.js` fades
   it once the app or the sign-in screen is ready. A 15 s timeout is the safety
   net.
3. `web/js/app.js` (module entry) boots: session/auth, preferences, theme
   loader, UI library, then the desktop shell.
4. `web/js/bootScreen.js`, `web/js/session.js` and `web/js/auth.js` decide
   between the **sign-in screen** and the **desktop**.
5. Active plugins are loaded dynamically (see below) and their window surfaces
   mounted.

---

## Module map (`web/js/`)

The shell is split by concern. Grouped:

### Desktop & windows

| Module | Role |
|---|---|
| `desktop.js` | Desktop model: workspaces, layout, persistence. |
| `tiles.js` | Tile grid, window chrome creation, `ensureWindowChrome`, glow wiring. |
| `fullscreen.js`, `fullscreenBubble.js` | Fullscreen handoff and the glass exit bubble. |
| `overview.js` | Window overview (all open windows, live). |
| `launcher.js` | Plugin/app grid launcher. |
| `viewport.js` | Viewport/scale handling. |
| `gestures.js` | Maps `trackpad:gesture` events onto the desktop. |
| `contextMenu.js` | The single right-click menu engine. |
| `menuState.js` | Shared menu state helpers. |

### HUD & host panels

| Module | Role |
|---|---|
| `hudLeft.js` | Clock + weather, left HUD. |
| `hudPlugins.js` | Plugin tray, grouped by manifest `category`. |
| `hudAudio.js`, `audioMenu.js`, `audioShared.js` | Sound chip + menu. |
| `hudBluetooth.js`, `bluetoothMenu.js`, `bluetoothShared.js` | Bluetooth chip + menu. |
| `hudBattery.js`, `batteryMenu.js`, `batteryShared.js` | Battery chip + power quick menu. |
| `hudNetwork.js`, `networkMenu.js` | Network chip + menu. |
| `powerMenu.js`, `powerShared.js` | Power actions. |
| `hudChipsShared.js` | Shared chip rendering. |
| `display.js` | Interface scale control. |

### Chat, voice, agent

| Module | Role |
|---|---|
| `agent.js` | Drives `/api/agent` (JSON + SSE), renders replies. |
| `chatHistory.js` | Conversation list + message bubbles. |
| `voice.js`, `transcriptGuard.js`, `textInput.js`, `sphere.js` | Voice bar, STT/TTS wiring, typed input, the sphere. |
| `artifacts.js`, `artifactStore.js` | Artifact cards + dock. |
| `files.js` | `saveOrDownload` / `openWithPlugin` desktop integration. |
| `api.js`, `auth.js`, `session.js` | Fetch wrapper, auth, session bootstrap. |

### Settings & plugins

| Module | Role |
|---|---|
| `settingsWindow.js`, `preferences.js` | Settings window + per-user preferences. |
| `pluginsWindow.js`, `activePlugins.js`, `pluginIcon.js` | Plugin manager, active set, icon resolution. |
| `appearance.js` | Accent/gradient/theme. |
| `userProfiles.js` | Profile/persona. |
| `coreWindows.js` | Built-in windows (Settings, Plugins, Chats). |
| `fonts.js` | Installed-font picker. |
| `insights/` | Destination insight cards. |
| `touchbar.js`, `touchbarShared.js` | Touch Bar action vocabulary. |
| `gps.js`, `map.js`, `navigator.js`, `navigationApi.js` | Travel surfaces. |
| `keyboard.js` | Virtual keyboard (chrome). |
| `background.js` | Desktop background. |
| `bootScreen.js` | Boot splash. |

---

## The UI component library (`web/ui/`)

Plugins never ship CSS. Every visual surface is built from the theme-agnostic
component library:

| File | Role |
|---|---|
| `index.js` | Public re-exports (`icon`, `button`, `card`, `notify`, `setTileGlow`, …). |
| `theme-loader.js` | Loads the theme manifest/tokens/components and swaps them live. |
| `appearance.js` | Accent + gradient; dispatches `appearance:change`. |
| `icon.js` | Resolves an icon name via active icon set → theme override → base catalog. |
| `reveal.js` | Reveal-on-scroll helper. |
| `components/button.js`, `field.js`, `card.js`, `overlay.js`, `feedback.js`, `data.js` | The `.ui-*` primitives. |
| `components/composites.js` | Higher-level composites (e.g. the artifact panel). |
| `components/notifications.js` | GNOME-style banners. |
| `components/glow.js` | The ambient window glow. |

See [themes & icons](themes-icons.md) for theming and icon resolution, and
[plugin authoring](../plugins/authoring.md) for the window-surface contract.

---

## Loading a plugin window

Core dynamically imports each active plugin's `web/plugin.js` from
`/plugins/<name>/plugin.js`. The default export is the window surface:

```js
export default {
  name: 'word',
  mount: mountWordTile,      // returns the window element (or null)
  unmount: unmountWordTile,
  getElement: getWordTileElement,
  wireEvents: wireWordEvents,
  contextMenu: wordContextMenu,   // optional right-click entries
};
```

`mount()` returns a `section.tile <name>-tile` carrying `data-plugin="<name>"`;
core tiles it, applies focus/fullscreen, and calls `unmount()` on deactivation.
Window styles live in core's `web/css/tiles.css`, keyed by the plugin's class
prefix.

> **Development gotcha:** the app serves the **installed** copy at
> `data/plugins/<name>/web/`, not `plugins/<name>/web/`. Copy (or reinstall)
> after editing, or the browser keeps loading the stale file.

---

## CSS

Global shell styles live in `web/css/` (`app.css`, `tiles.css`, `sphere.css`,
`fullscreen.css`, `overview.css`, `keyboard.css`, `navigator.css`,
`settings.css`, `plugins.css`, `text-input.css`, `background.css`,
`boot.css`). Tokens come from the active theme (see
[themes & icons](themes-icons.md)). Vendored third-party assets sit in
`web/vendor/` (`pdfjs/`, `vosk-browser/`).

---

## No-build rationale

The module graph is plain ES modules with relative imports so the same tree
works under `peakd`/`peakd-mac` and in any browser, and plugins can import the
UI library relative to their own served location. There is no transpile step to
keep in sync with the server.
