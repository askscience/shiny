# Shared UI components

All first-party window controls come from `web/ui/index.js`. Plugin code owns
its data, actions and domain content. The UI library owns reusable controls,
keyboard and touch interaction, theme tokens and responsive control sizing.
Settings and the plugin manager use the same library.

Import relatively from a served plugin entry point:

```js
import {
  element, toolbar, toolButton, menuButton, input, field, select,
  controlGrid, controlCard, dial, mediaCard, modal, placePopover,
} from '../../ui/index.js';
```

The import depth must match the served module URL. See the existing plugin
entry point when adding a module; source directories and installed URLs are
not always identical.

## Component inventory

| Purpose | Public exports | Contract |
|---|---|---|
| Actions | `button`, `iconButton`, `toolButton`, `menuButton` | Shared theme, focus, disabled state and motion. Give icon actions a translated `label`. |
| Domain actions | `controlButton` | An unskinned native button with `type="button"` and the shared interaction marker, for keys, cells and other domain layouts. |
| DOM construction | `element(tag, className, text)` | Routes button/input/textarea/select creation through the shared factories; adds layout classes without removing `.ui-*` classes. Replaces plugin-local `h`/`el` implementations. |
| Form controls | `input`, `textarea`, `select`, `selectControl`, `slider`, `checkbox`, `toggle`, `toggleRow`, `searchBar` | `select()` returns a wrapper with `.select` and `.setOptions()`. `selectControl()` returns the actual select for compact toolbars and has `.setOptions()`. Both use the same in-page popup. |
| Labels | `field({ label, control, hint, htmlFor })` | Associates the label with the control, including a wrapped select. Generates a control ID when needed. |
| Numeric dials | `dial` | One control for Image adjustments and Studio parameters: pointer drag, wheel, arrow keys, Home/End, reset, ARIA values and cleanup. |
| Layout | `toolbar`, `row`, `stack`, `controlGrid`, `controlCard`, `controlGroup` | Shared arrangement; callers supply content and translated labels. `toolbar(children, { className, label })` supports domain layout hooks. |
| Selection | `chip`, `segmented` | Chip state has `aria-pressed`; segmented choices support roving keyboard focus and a selection indicator. |
| Surfaces | `card`, `panel`, `section`, `divider`, `modal`, `sheet`, `tooltip`, `confirmDialog`, `promptDialog`, `menuSurface`, `menuItem` | Reuse shared surfaces and dialog focus/lifecycle handling. |
| Media lists | `mediaCard` | Shared keyboard-activatable artwork card used by Radio and YouTube; accepts title, subtitle, source, position, duration and action. Optional `prefix` retains domain layout hooks. |
| Data and feedback | `list`, `listItem`, `stat`, `badge`, `avatar`, `keyValue`, `toast`, `notify`, `spinner`, `skeleton`, `progress`, `emptyState` | Reuse these instead of introducing independent status, loading or empty-state systems. |
| Popover geometry | `placePopover(popup, anchor)` | Measures the visible viewport, clamps width, chooses above/below placement and limits scrolling height, including a soft keyboard. Call after showing the popup; placement follows scrolling/rotation and releases its observers on close. |
| Motion | `transition`, `disposeMotion`, `revealRegion`, `enterSurface` | Finite, cancellable transitions that respect reduced motion. |

`toolButton(icon, label, onClick, options)` is the compact icon action. Options
include `danger`, `className`, `variant`, `disabled` and `iconSize`.
`menuButton({ icon, label, text, onClick, className })` adds the shared menu
chevron and menu ARIA attributes. Its `.caption` is available for an account
name. `menuSurface({ className, onEscape })` and `menuItem({ label, onClick,
disabled, className })` provide a shared menu with arrow/Home/End navigation.
Call `.focusFirst()` after showing and positioning the surface; Escape restores
focus. The owner updates `aria-expanded` when opening/closing its menu.

Do not assign `className` after creating a shared control: that discards its
base classes. Use `classList.add()` for layout hooks. If a reusable component is
missing, add it to this library and migrate similar implementations together.
Editor canvases, spreadsheet cells, audio timelines, terminal output and map
rendering remain domain content.

`await confirmDialog(message)` returns a boolean; `await promptDialog(message,
initialValue)` returns a string or `null` when cancelled. Both use the shared
modal, preserve focus and fit the viewport. Use them instead of blocking native
`window.confirm`/`window.prompt`. Capture any editor selection before awaiting a
dialog; focus moves into the dialog while it is open.

## Search

`searchBar()` renders a pill: the input is bare and `.ui-search` itself carries
the plate, the radius and a 1px ring in every state (the theme's border token at
rest, recoloured to the accent on focus — the width never changes), with the
magnifier as a round action at the end (`.ui-search-action` — it returns the
caret to the input and is kept out of the tab order). Pass `onClear` and the
clear button sits between the input and the disc.

The pill is 34px by default — the same height as the icon buttons it sits beside
in a window's toolbars — and flat, so its shadow cannot bleed out of the row.
A page-level search opts into the larger floating pill the way Radio and YouTube
do:

```css
.radio-tile .ui-search {
  --search-size: 40px;
  --search-shadow: var(--shadow-sm, 0 2px 8px rgba(0, 0, 0, 0.12));
}
```

Themes re-skin the field through `--search-plate` (default `--surface-strong`),
`--search-ink` (default `--text`, the disc, with the glyph cut out in the plate
colour), `--search-shadow` and `--glass-border` (the 1px resting ring), instead
of restyling the inner input; window CSS adjusts only the wrapper
(`.browser-address .ui-search { flex: 1 1 auto }`,
`.radio-tile .ui-search { margin: … }`).

## Dials and lifetime

```js
const brightness = dial({
  label: t('plugin.image.dial.brightness'),
  min: -100, max: 100, step: 1, value: 0, resetValue: 0,
  onInput: value => preview(value),
  onChange: value => save(value),
});
// A server/automation update repaints silently.
brightness.setValue(25);
// When removing the owning view:
brightness.destroy();
```

Drag and wheel changes call `onInput`; pointer release and the wheel debounce
call `onChange`. Discrete keyboard/reset changes commit directly when
`onChange` exists, otherwise call `onInput`. `format(value)` customizes the
readout and accessible value text. `onCancel` optionally handles Escape.
Home/End move to the range limits; Delete/Backspace and double-click reset.

Call `destroy()` on dials and segmented controls when removing their view, and
on dialogs when their owner is destroyed. Use `createResources()` for owned
listeners, observers and timers. The shared select dismisses when its anchor is
removed, the window resizes, or the visual viewport changes.

## Responsive layout

`ui.css` supplies component geometry, themes supply tokens/skin, `flow.css`
supplies common motion, and `responsive.css` loads last to enforce sizing and
reachability. Theme overrides must preserve these interaction contracts.
Neither a plugin stylesheet nor a theme restyles a shared control's own classes
(`.ui-input`, `.ui-search`, `.ui-select`, `.ui-dial`): the look, metrics, focus
ring and font of those belong to the library and the tokens behind it, so one
window can never drift from the rest. Size and arrange a control from its
window's wrapper class instead (`.browser-address .ui-search { flex: 1 1 auto }`,
`.radio-tile .ui-search { margin: … }`).

Each `.tile` is a named `plugin-window` inline-size container. Use
`@container plugin-window (...)` for pane changes: a small floating/tiled window
needs the same adaptation as a phone. Use viewport height queries for short
landscape screens and `pointer: coarse` for touch target sizing. Do not hide
commands just because a window is narrow.

A direct child marked `data-window-bar` merges into the core title bar. On narrow
or short windows, long toolbars scroll horizontally as one row. Inputs and
selects inside toolbars use compact widths instead of full-form width. Below
620px Settings drops the section-navigation labels and keeps an icon-only,
scrolling strip — the tabs stay named through `aria-label`/`title`, and the
active one keeps its accent tint. Mail adapts its panes,
Files keeps its places available in a horizontal strip, and PDF/Impress move
thumbnail rails above their content on narrow windows. Large document surfaces
scroll inside their own pane, rather than expanding the whole window.

Set `min-width: 0` and `min-height: 0` on flex/grid content panes; give the actual
content pane `overflow: auto`. Keep long labels breakable and use
`minmax(0, 1fr)` or `minmax(min(100%, <minimum>), 1fr)` for responsive grids.
Dialogs use dynamic viewport height and safe-area padding. A custom body-level
menu must use `placePopover()` instead of copying fixed-width coordinates.

Phone switching and workspace suspension persist across portrait/landscape
rotation on coarse-pointer phone screens. Stored desktop workspace layouts are
retained. The virtual keyboard has compact geometry for short landscape screens.

## Fonts and embedded content

The default UI font is `system-ui`; theme body/display tokens use the operating
system font stack. Editorial and monospace surfaces use generic local stacks.
The Appearance and Word font pickers can still select installed host fonts from
`/api/fonts`. If that list is unavailable, only generic local families are offered.
Do not add Google Fonts, remote font stylesheets or webfont downloads to plugin UI.

Received HTML email is embedded content: preserve its original typography and
styles. Do not inject UI fonts into its iframe. Likewise, a browser page or a
user-authored document is distinct from the surrounding application controls.
Filmcraft was excluded from this consolidation.

## Verification

Run `scripts/test-web.sh` for syntax, unit and plugin smoke checks. The additional
`scripts/test-responsive-ui.mjs` suite serves source files through an isolated
local fixture server with fake API responses, mounts actual window modules and
core chrome, and checks phone portrait/landscape, desktop and narrow desktop tiles.
It checks overflow, clipped controls, every settings section, touch/mouse select
menus, email typography and external font requests. The map and virtual keyboard
use their core frontends; Hello has no window. Filmcraft is excluded.

The browser suite is optional tooling, not an application build dependency. With
Playwright and its Chromium browser installed in a tooling directory:

```sh
SHINY_PLAYWRIGHT_MODULE=/path/to/node_modules/playwright/package.json \
  node scripts/test-responsive-ui.mjs
SHINY_UI_THEME=light SHINY_PLAYWRIGHT_MODULE=/path/to/node_modules/playwright/package.json \
  node scripts/test-responsive-ui.mjs settings plugins mail
```

`SHINY_UI_SCREENSHOTS` may name an existing output directory. The suite uses
fixtures, so it does not certify live mail servers, media playback, native kiosk
browser embedding, or every possible document. Changes to a plugin's source
must also reach the installed copy before a running installation can load them:
the baseline root (`SYSTEM_PLUGINS_DIR`, the repo's `data/plugins`) is scanned
first, but a same-named plugin in `PLUGINS_DIR` (`~/.local/share/shiny/plugins`
in a kiosk install) **overrides** it, and the override is what gets served. Do
not restart the user's kiosk to refresh source.
