# Web UI

The frontend is a **no-build ES-module app** served from `web/`. There is no
bundler: [`web/index.html`](../../web/index.html) loads the shell CSS, a
synchronous theme bootstrap, and then the module graph under
[`web/js/`](../../web/js). Static shell assets are served with
`Cache-Control: no-cache` (revalidated, 304 when unchanged), so a server
restart or source edit is picked up on the next reload. API and SSE responses
stay `no-store`.

---

## Boot sequence

1. **Theme bootstrap** (inline script in `index.html`) reads the stored theme
   from `localStorage` (`ui.theme.name`), validates it against the cached theme
   index, sets `data-theme` on `<html>`, and points `#theme-tokens` /
   `#theme-components` at the right CSS — before first paint, to avoid a flash.
   [`early-i18n.js`](../../web/js/early-i18n.js) tags the same element with the
   stored interface language (`lang`, `data-locale`, `dir`) in the same pass.
2. The **boot splash** (`#boot-screen`) is painted; `web/js/bootScreen.js` fades
   it once the app or the sign-in screen is ready. A 15 s timeout is the safety
   net.
3. `web/js/app.js` (module entry) boots: interface language
   (`initI18n()` — see [interface language](i18n.md)), session/auth,
   preferences, theme loader, UI library, then the desktop shell.
4. `web/js/bootScreen.js`, `web/js/session.js` and `web/js/auth.js` decide
   between the **sign-in screen** and the desktop.
5. Active plugins are loaded dynamically (see below) and their window surfaces
   mounted.


---

## Module map (`web/js/`)

The shell is split by concern. Grouped:

### Language

| Module | Role |
|---|---|
| `i18n.js` | Translation runtime: `web/i18n/*.json` catalogs, `t()`/`tp()`, `data-i18n*`, `<html lang>`/`dir`, `locale:change`. See [interface language](i18n.md). |
| `early-i18n.js` | Tags `<html>` with the stored interface language before first paint. |

### Desktop & windows

| Module | Role |
|---|---|
| `desktop.js` | Desktop model: workspaces, layout, persistence, focus mode. |
| `tiles.js` | Tile grid, window chrome creation, `ensureWindowChrome`, the eye button, glow wiring. |
| `fullscreen.js`, `fullscreenBubble.js` | Fullscreen handoff and the glass exit bubble. |
| `overview.js` | Window overview (all open windows, live). |
| `launcher.js` | Plugin/app grid launcher. |
| `viewport.js` | Viewport/scale handling. |
| `gestures.js` | Maps `trackpad:gesture` events onto the desktop. |
| `contextMenu.js` | The single right-click menu engine. |
| `linkRouter.js` | Kiosk link routing: sends external link clicks to the chosen link-handler plugin (see below). |
| `menuState.js` | Shared menu state helpers. |
| `menuFactory.js` | The shared HUD-popup lifecycle (build once, open/close, outside click, Escape, placement). |
| `hudStream.js` | Shared SSE + poll fallback for the top-bar chips. |
| `officeMenu.js` | Shared document-menu footer and date label for Calc/Impress/Word/PDF. |

### HUD & host panels

| Module | Role |
|---|---|
| `hudLeft.js` | Clock + weather, left HUD. |
| `hudPlugins.js` | Plugin tray, grouped by manifest `category`; lights the focused plugins' underline. |
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
| `voice.js`, `transcriptGuard.js`, `textInput.js`, `sphere.js` | Voice bar, STT/TTS wiring, the compose chat panel (pending wheel + live step line, "Thinking" fold, image/song blocks, close), the sphere. |
| `dockStep.js`, `voiceGlyph.js`, `toolIcons.js` | The assistant's status line and the voice bar's glyph: the dock bubble, the same line or the running tool's icon inside the bar (with its swap fade), and the tool → catalogue-icon map. |
| `thinkToggle.js` | Chat head's brain button + voice-bar menu entry: Ollama thinking on/off, gated by the model's capability (`GET /api/ai/thinking`). |
| `artifacts.js`, `artifactStore.js` | Artifact cards + dock. |
| `files.js` | `saveOrDownload` / `openWithPlugin` desktop integration. |
| `api.js`, `auth.js`, `session.js` | Fetch wrapper, auth, session bootstrap. |

### Settings & plugins

| Module | Role |
|---|---|
| `settingsWindow.js`, `preferences.js` | Settings window + per-user preferences. |
| `pluginsWindow.js`, `activePlugins.js`, `pluginIcon.js` | Plugin manager (installed list + the Craft store tab), active set, icon resolution (theme icon → `icon.svg` → `icon.png`). |
| `craftStore.js` | Craft App Suite catalog + the background updater (install/update via `/api/plugins/store`, including an install still missing its AI control); also toggles an app's AI control (its MCP server). |
| `webAppSurface.js` | Window surface for a `kind = "web"` plugin: the iframe that hosts its static site, plus the page half of live AI control (detects `window[<app>].request` and answers the core's control-channel proxy) and the file-input shim that gives every app's Open… the shell's picker. |
| `filePicker.js` | The shell's file picker: a compact **floating window** (`ui/floating`: the desktop's own title bar with only close, outside the tiling) with the Files window's thumbnail grid (`files-*` classes, `/api/files/thumb`) browsing the user's home through the Files plugin. Returns picked entries, or real `File` objects for the caller's realm. Used by `files.js` and by every web app's `<input type=file>` through the surface shim. |
| `appSaves.js` | Follows the shell's `shiny:file-saved` reports — a web app saved through the browser APIs inside its iframe — and turns them into the toast with the real path plus the `files:external` refresh the Files window listens for. |
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

Plugins reuse the theme-agnostic component library for controls. Plugin-specific
CSS may arrange domain content; shared styling and motion remain in core:

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

`field.js`'s `select()` deliberately never opens the platform popup. On X11,
QtWebEngine 6.8 crashes the whole shell (SIGSEGV in `QXcbWindow::handleNativeEvent`,
mid-teardown of the popup's native window) as soon as a native `<select>` dropdown
opens — which is what made choosing a theme or an icon set in Settings → Appearance
restart the kiosk. The component keeps the styled `<select>` as the closed field
and draws the open list itself (`.ui-select-menu`, an in-page popover). `selectControl()` returns the same shared control without a wrapper for
compact toolbars. Legacy raw selects can be armed with `useSelectMenu(el)`, but
new plugin controls should use the shared factories. The upstream fix is in Qt 6.9.2 (QTBUG-135036).

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
core tiles it, applies focus/fullscreen, and calls `unmount()` on window close,
deactivation or uninstall.
Window styles live in core's `web/css/tiles.css`, keyed by the plugin's class
prefix.

> **Development gotcha:** the app serves the **installed** copy at
> `data/plugins/<name>/web/`, not `plugins/<name>/web/`. Copy (or reinstall)
> after editing, or the browser keeps loading the stale file.

---

## Link routing (kiosk)

The shell refuses page popups on purpose — a link must never leave the kiosk —
so app-drawn links (`target="_blank"` anchors from markdown, document previews,
mail, help text) used to be dead: the popup request came back as a `new-window`
event for the `main` view and matched no browser tab. `web/js/linkRouter.js`
now intercepts those clicks first: `initLinkRouter()` (called from `app.js`,
kiosk-only) installs one delegated click listener, and every external `http(s)`
link is handed to the user's **link handler** — the `browser` plugin by default,
or any installed plugin that declares the `open-urls` capability (Settings →
Browser → Links, preference `link.plugin`).

The hand-off mirrors the Files integration: core activates the target, reveals
its window (raised, not focused), sets `window.__pendingUrlOpen` and dispatches
`url:open` `{ plugin, url }`; the plugin registers with `onOpenUrl(plugin,
handler)` from its `wireEvents`. Same-origin, `#`, relative and `mailto:` clicks
keep their default behaviour, and in a normal desktop browser the router is
inert (there the anchor's own `target="_blank"` is the right behaviour). Mail's
HTML messages render in a sandboxed frame that cannot reach the app, so their
frame carries a click bridge of its own (see `plugins/mail/web/plugin.js`).

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

## Module and resource ownership

### Flow: shared layout and motion

`web/ui/flow.css` loads after theme app overrides. It defines shared interaction
structure while consuming the active theme's colors and shape tokens. The core
tokens in `ui.css` are `--motion-feedback` (120 ms), `--motion-reveal` (200 ms),
`--motion-connect` (280 ms), and `--motion-ease`. Themes may override these roles;
plugins consume them rather than define independent timings. Existing window
radius, opacity, blur, shadow, title height, font, icon and accent settings remain
authoritative. There is no additional appearance preference to migrate.

The [component inventory and responsive contract](ui-components.md) documents
all shared controls, font policy and browser verification.

The public UI entry point exports `controlGrid`, `controlCard`, `controlGroup`,
`segmented`, and `controlButton`. `controlButton()` is an unskinned button for
domain-specific layouts; ordinary actions use `button()`/`iconButton()`. First-party
plugin button builders use the shared factories. Editors, terminal rendering,
media, musical timing, and canvases retain their domain behavior.

`transition`, `enterSurface`, `revealRegion`, `disposeMotion`, `motionDuration`,
and `reducedMotion` own finite transitions. Replacement cancels old completion;
reduced motion completes pending state changes; a bounded fallback handles an
occluded renderer. Final styles belong to CSS, not a retained animation. Call
`disposeMotion(root)` on unmount and dispose `segmented().destroy()` with its
owner. Only animate small controls, selection indicators and reveal regions;
do not animate streaming content, large editor surfaces or window dragging.

Settings keeps its 12 existing section IDs and direct navigation. Shared control
cards sit in groups defined by existing translated headings; the grid gains two
columns when the **window**, rather than the screen, is wide enough. A narrow
window turns the section navigation into an icon-only strip — each tab keeps its
`aria-label`/`title`, so nothing loses its name. Provider, voice engine and
backdrop options reveal inside their owning card. Locale rerenders continue to
dispose previous section resources. Visible labels reuse existing translation
catalogs; new UI text must use `t()`/`tp()` keys.

Shared modal and sheet APIs retain `el`, `card`, `open`, `close`, and `isOpen`;
modal also retains `body`. Both add `destroy()` for owner teardown and accept an
optional translated `label` and `removeOnClose` for one-shot surfaces. They trap keyboard focus, restore it on dismissal,
and preserve background inert state for nested dialogs. Reopening during an exit
cannot be hidden by an earlier completion.

Public import paths remain unchanged. `settingsWindow.js` owns the window and
navigation while `settings/` contains section factories and shared controls.
`desktop.js` forwards the existing desktop API to `desktop/workspaces.js`;
`desktop/layout.js` renders geometry and splits, and `desktop/interactions.js`
handles floating-window pointer operations. `voice.js` forwards to recognition
orchestration in `voice/`, alongside capture conversion, wake matching and playback.

`createResources()` owns listeners, timers and abort controllers for a mount. It
disposes them once in reverse order and immediately cleans up resources registered
after disposal. Settings replaces its section resources on locale changes, so
polls and listeners cannot accumulate across rerenders. Existing theme tokens,
translation keys, preference formats and plugin window contracts remain intact.

Playback releases audio object URLs on completion, error and cancellation, aborts
cancelled synthesis requests, and releases microphones granted after barge-in
monitoring has already been cancelled.
