# Themes & icons

Two systems keep plugin output visually consistent with the user's theme and
accent: the **theme layer** (`web/themes/`) and the **shared icon library**
(`web/ui/icons/`).

---

## Themes

A theme is a folder under `web/themes/<name>/`. Installed themes are listed in
[`web/themes/themes.json`](../../web/themes/themes.json).

| File | Role |
|---|---|
| `theme.json` | Manifest: display name, mode (dark/light), etc. |
| `tokens.css` | Design tokens (`--accent`, surfaces, text, spacing, radii). |
| `components.css` | Component styling (`web/ui/ui.css` overrides). |
| `app.css` | Theme-specific overrides. |
| `icons/` | Optional per-theme **icon overrides** (SVG, `currentColor`). |

Bundled themes: **noir**, **neumorphic**, **pitch** (dark) and **light**,
**neumorphic-light** (light). The default is `pitch`.

### Live switching

[`web/ui/theme-loader.js`](../../web/ui/theme-loader.js) loads a theme by
swapping the `#theme-tokens` and `#theme-components` `<link>` elements and then
the app-level `#theme-app` link. The inline bootstrap in `index.html` applies the
stored theme before first paint. A change dispatches a **`theme:change`** window
event so live surfaces (maps, canvases, glow) can repaint.

### Accent & gradient

[`web/ui/appearance.js`](../../web/ui/appearance.js) owns the per-user accent
colour and gradient. It writes the CSS custom properties and dispatches
**`appearance:change`**, which repaints everything including the Leaflet map
colours and the voice bar glow. Settings → Appearance exposes them.

---

## The shared icon library

There is **one** icon library at [`web/ui/icons/`](../../web/ui/icons). The HUD,
the core windows, the desktop, Files and every plugin's identity glyph come from
it. Browse the catalog in
[`web/ui/icons/INDEX.md`](../../web/ui/icons/INDEX.md) and the prose guide in
[`web/ui/icons/README.md`](../../web/ui/icons/README.md).

### Resolution

[`web/ui/icon.js`](../../web/ui/icon.js) resolves an icon **by name**:

1. the active theme's override at `/themes/<theme>/icons/<name>.svg`, else
2. the shared icon at `/ui/icons/<name>.svg`.

Use it in JS:

```js
import { icon, setIcon } from '/ui/index.js';

el.appendChild(icon('ui/folder-open', { size: 16 }));
await setIcon(el, 'ui/save', { size: 16 });
```

…or in static HTML: `<span data-icon="ui/search" data-icon-size="16">` (hydrated
at boot).

**Always reuse a name from the catalog** instead of drawing new SVG: the icons
are `currentColor` and follow the theme and accent for free.

### Namespaces

- `ui/*` — core UI (`ui/settings`, `ui/puzzle`, `ui/close`, `ui/chevron-*`,
  `ui/search`, `ui/list`, `ui/grid`, `ui/power`, `ui/folder`, `ui/file`,
  `ui/doc`, `ui/image`, `ui/video`, `ui/music`, `ui/archive`, `ui/download`,
  `ui/home`, `ui/trash`, `ui/monitor`, …).
- `hud/*` — top-bar chips (`hud/wifi-0…4`, `hud/ethernet`, `hud/bluetooth`,
  `hud/volume-*`, `hud/battery-0…4`, `hud/battery-charging`, `hud/clock`).
- `apps/<plugin>` — plugin identity glyphs.
- `artifacts/*` — artifact dock icons resolved through `THEME_ICONS`.

### KDE Slot-Beauty curation

The HUD, core-window, file-type and `apps/` glyphs are curated from the KDE
**Slot-Beauty** icon set (GPL-3.0). The raw set (thousands of extra app,
mimetype and category glyphs) lives in `assets/iconsets/` (not committed). The
curator is [`scripts/kde-icons/`](../../scripts/kde-icons):

- `convert.py` — sanitize + normalize SVGs to `currentColor`.
- `mapping.json` — KDE source name → Shiny name.
- `source/{dark,light}` — the ~60 committed curated SVGs.

To pull in one more icon, follow `web/ui/icons/README.md`.

**Folders are the one coloured exception**: the `ui/folder` glyph is real
artwork (not `currentColor`) and follows the accent/theme mode, so it is shipped
in dark and light variants.

---

## Plugin icons

A plugin's identity icon is resolved by
[`web/js/pluginIcon.js`](../../web/js/pluginIcon.js) in this order:

1. the shared library icon mapped to it (`PLUGIN_ICONS` → an `apps/<name>`
   glyph),
2. the plugin's own `web/icon.svg` (served at `/plugins/<name>/icon.svg`),
3. the core-window icon (Settings → `ui/settings`, Plugins → `ui/puzzle`),
4. the fallback `ui/puzzle`.

Bundled plugins all have a curated `apps/<name>` icon. A third-party plugin can
ship `web/icon.svg`. The style for those is deliberately bolder than the thin
theme icons (which paint at 16–20px): `stroke-width="2"` on a 24×24 grid, round
caps/joins, soft corners, one filled accent, tight framing. Keep it
single-colour and script-free — core inlines the file and rejects `<script>`,
`on*=` handlers and `javascript:` URLs.

The resolved icon appears in the **top-bar plugin tray** (grouped by manifest
`category`), the **launcher**, the window/tile dot, and the **Plugins window**.

---

## The ambient window glow

Every plugin window carries a soft ambient glow behind its content, installed by
core ([`web/ui/components/glow.js`](../../web/ui/components/glow.js) + the
`.tile-glow` block in `web/css/tiles.css`). Two tiers:

- **Tier 0** — a seeded two-blob radial gradient from the user's accent plus a
  per-window partner hue. Free for every window.
- **Tier 1** — the plugin's real subject (artwork, PDF page, photo, map view)
  via `setTileGlow`, `setTileGlowFromUrl`, `glowGradient` or `glowFromDrawable`
  from the UI library.

The blur is **baked once at thumbnail size** (no per-frame CSS `blur()`), which
is why it is cheap. Light themes brighten instead of darken. The map uses the
**rim** variant (drawn on top, masked to the edges). Settings → Appearance →
*Blurred window background* removes it globally by hiding `.tile-glow`; plugins
never check the flag. A theme tunes it with the `.tile` tokens
`--glow-brightness`, `--glow-opacity`, `--glow-permeability`, `--glow-veil`.

See [plugin UI conventions](../plugins/authoring.md#window-surface) and
[PLUGINS.md §19](../../PLUGINS.md).
