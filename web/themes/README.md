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
5. Replace the icons, keeping the same file names and `currentColor`.
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
