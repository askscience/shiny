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

The **shared** icon set lives in `/ui/icons/<group>/<name>.svg` (the UI
library) — its catalog is [`web/ui/icons/INDEX.md`](../ui/icons/INDEX.md) and
its authoring guide is [`web/ui/icons/README.md`](../ui/icons/README.md). A
theme's `icons/` is an **override set**: it only needs to ship the icons it
draws differently — anything it omits falls back to the shared icon.

### Icon sources

Most shared icons are line glyphs drawn in-house. The HUD, core-window, file
type and plugin (`/ui/icons/apps/`) glyphs are curated from the **Slot-Beauty**
KDE icon themes ([L4ki/Slot-Plasma-Themes](https://github.com/L4ki/Slot-Plasma-Themes),
**GPL-3.0**) and regenerated with `scripts/kde-icons/convert.py` — see
`KDE_ICONS_PLAN.md` and [`web/ui/icons/README.md`](../ui/icons/README.md). The
converter strips the KDE `<style>` block, rewrites fills to `currentColor` and
derives a `viewBox` from the source, so the icons follow each theme and the
user's accent like any other.

The one deliberate exception is the **folder** glyph: it keeps the original
KDE folder artwork (colour + gradient). The shared `ui/folder.svg` is the dark
artwork, so the **light** themes (`light`, `neumorphic-light`) each ship their
own `icons/ui/folder.svg` (light artwork) as an override. Only the curated
source SVGs (`scripts/kde-icons/source/`) and the generated icons are
committed; the raw ~180 MB sets live in `assets/iconsets/` and are ignored.

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
  1.5 stroke width for visual rhythm. They inherit color from CSS and
  follow the accent automatically.
- **Motion** — restrained: single soft entrances, no looping decoration.
  Honor `prefers-reduced-motion` (already handled in `/ui/ui.css` for
  reveals/spinners; keep it that way in your additions).
- **Modes** — declare `modes: ["dark"]` or `["light"]` in the manifest;
  the first mode drives the map tile flavor.

## How plugins relate to themes

Plugins never ship CSS. Their UI (artifacts, insight cards) is JSON that
core renders through `/ui/` components — so plugin content always matches
the active theme and the user's accent automatically. If plugin web assets
land (see PLUGINS.md roadmap), they must load `/ui/` and use the same
components instead of shipping styles.
