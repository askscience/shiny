# Shiny themes

A theme skins the **Shiny UI library** (`/ui/`). The library owns component
structure and behavior; a theme owns every visual decision: colors, typography,
radii, motion, icons.

The app is monochrome by design. Users personalize a single **accent color**
and one **gradient** in *Settings → Appearance* — themes must consume the
accent tokens rather than introduce their own colors.

## Layout

```
themes/
  themes.json            ← installed theme index: ["noir", "yourtheme"]
  noir/                  ← the default theme
    theme.json           ← manifest
    tokens.css           ← design tokens (:root custom properties)
    components.css       ← visual skin for every .ui-* component
    icons/               ← SVG icons, stroke/fill="currentColor"
      ui/                ←   generic UI glyphs (close, chevron, check, …)
      artifacts/         ←   artifact type/theme icons (plan, route, food, …)
      insights/          ←   insight card icons (weather, places, events)
      hud/               ←   HUD icons (clock, …)
```

The **base** icon set lives in `/ui/icons/<group>/<name>.svg` (the UI
library) — its catalog is [`web/ui/icons/INDEX.md`](../ui/icons/INDEX.md) and
its authoring guide is [`web/ui/icons/README.md`](../ui/icons/README.md).
Selectable icon sets (Infinity, Slot-Beauty) live in `/ui/iconsets/<set>/`, and
the resolver in `web/ui/icon.js` tries the active set, then the theme, then the
base. A theme's `icons/` is therefore a true **override set**: it only needs to
ship the icons it draws differently — anything it omits falls back to the
active icon set and then the base catalog. Bundled themes currently override
just one glyph: the **light** themes (`light`, `neumorphic-light`) ship their
own `icons/ui/folder.svg` artwork.

### Icon sources

The base catalog mixes in-house line glyphs with the coloured **Slot-Beauty**
KDE icon set ([L4ki/Slot-Plasma-Themes](https://github.com/L4ki/Slot-Plasma-Themes),
**GPL-3.0**); the coloured **Infinity** set
([rogts/infinity-icon-theme](https://github.com/rogts/infinity-icon-theme),
**GPL-3.0**, Breeze-derived) is curated as the default selectable set. Both are
regenerated with `scripts/kde-icons/convert.py` — see
[`web/ui/icons/README.md`](../ui/icons/README.md). The converter strips the KDE
`<style>` block, rewrites symbolic fills to `currentColor` (coloured artwork
keeps its paint) and derives a `viewBox` from the source.

Only the curated source SVGs (`scripts/kde-icons/source/`) and the generated
icons are committed; the raw sets live in `assets/iconsets/` and are ignored.
Coloured icons follow the user's accent when *Settings → Appearance →
Accent-tinted icons* is enabled; with it off (the default) they show the set's
native colours. Live icons repaint on `iconset:change`, `tint:change` and
`accent:change`.

Themes are plain static files — no build step, no backend involvement.
`/themes/` is served directly; `themes.json` exists because directories
can't be listed over HTTP.

## Creating a theme

1. Copy `noir/` to `themes/yourtheme/`.
2. Edit `theme.json` — `name` must match the folder name. Declare your
   accent and gradient **presets** (shown as swatches in Settings).
3. Rewrite `tokens.css` — the full token contract is documented inline in
   `noir/tokens.css`. All of it is required; the UI will not render
   correctly with missing tokens.
4. Restyle `components.css` — one rule block per component. Only visual
   properties (color, background, border, shadow, font); layout lives in
   `/ui/ui.css` and is shared.
5. Override only the icons you want drawn differently, keeping the shared
   names and `currentColor`; anything you omit uses `/ui/icons/`.
6. Add `"yourtheme"` to `themes/themes.json`. Users can now pick it in
   *Settings → Appearance → Theme*.

## Contracts a theme must honor

- **Accent slots** — `appearance.js` rewrites `--accent`, `--accent-2`,
  `--accent-soft`, `--accent-glow`, `--accent-contrast`, `--gradient-accent`,
  `--gradient-text`, `--gradient-mesh` at runtime from the user's choice.
  Use them; never hardcode brand colors.
- **Semantic colors** — `--ok`, `--warn`, `--error` are functional, not
  decorative. Keep them legible.
- **Icons** — always `stroke="currentColor"` (or `fill`), 24×24 viewBox,
  1.5 stroke width for visual rhythm. They inherit color from CSS and follow
  the theme (coloured artwork only follows the accent when *Accent-tinted
  icons* is on).
- **Motion** — restrained: single soft entrances, no looping decoration.
  Honor `prefers-reduced-motion` (already handled in `/ui/ui.css` for
  reveals/spinners; keep it that way in your additions).
- **Scrollbars** — centralized. `/ui/ui.css` defines one scrollbar for the
  whole app from the `--scrollbar-size`, `--scrollbar-thumb`,
  `--scrollbar-thumb-hover` and `--scrollbar-track` tokens. Windows and
  plugins must not restyle scrollbars; retune the tokens instead. Hide one
  only deliberately, with `scrollbar-width: none` plus a matching
  `::-webkit-scrollbar { display: none }`.
- **Modes** — declare `modes: ["dark"]` or `["light"]` in the manifest;
  the first mode drives the map tile flavor.

## How plugins relate to themes

Plugins never ship CSS. Their UI (artifacts, insight cards) is JSON that
core renders through `/ui/` components — so plugin content always matches
the active theme and the user's accent automatically. If plugin web assets
land (see PLUGINS.md roadmap), they must load `/ui/` and use the same
components instead of shipping styles.
