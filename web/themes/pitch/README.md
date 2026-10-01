# Pitch

A flat, very dark theme for Shiny — the reference is a minimalist **Android
launcher** (Launcher3, Nothing OS, a bare AOSP home screen).

Flat rectangles on a near-black canvas. No frosted glass, no dual-shadow
extrusion, no ambient colour bloom, no film grain, no gradients. Separation
comes from exactly three things: a tonal step in the neutral ramp, a 1px
hairline, and the accent used as *ink*.

## Files

```
pitch/
  theme.json      manifest: dark mode, accent + gradient presets
  tokens.css      the token contract (every custom property the UI reads)
  components.css  visual skin for every .ui-* component
  app.css         app-surface override — chrome, the desktop, and all
                  17 plugin windows. Loads after /css/*.css (including
                  the 7,700-line tiles.css), so it has the last word.
```

## Icons — read this before shipping a new theme

Pitch ships a **complete** icon set (128 files: 15 `hud/`, 90 `ui/`, plus
`artifacts/` and `insights/`), byte-identical to the other themes.

That is deliberate, and it is not what `/themes/README.md` describes. The docs
say a theme's `icons/` is an *override* set — ship only what you draw
differently and "anything it omits falls back to the shared icon" at
`/ui/icons/`. `ui/icon.js` implements exactly that fallback. **But the shared
set only holds 45 icons** (2 hud + 43 ui); the other 60 live exclusively inside
the per-theme folders. A new theme that ships an empty `icons/` therefore loses
60 glyphs and renders them as grey `ui-icon--missing` squares.

The real fix is to populate `/ui/icons/` so the documented fallback is true —
that is a change to the UI library, not to a theme, so it was left alone.

## How it is wired in

- Registered in `/themes/themes.json`.
- `DEFAULT_THEME` in `/ui/theme-loader.js` → `'pitch'`.
- The pre-paint bootstrap in `/index.html` hardcodes the same default (it cannot
  import a module, so the two must stay in step).

Switching themes at runtime still works — Settings → Appearance → Theme — and
Pitch's rules all live behind the theme's own stylesheet, so nothing leaks into
the other themes.

## Design rules this theme enforces

1. **Depth is a lighter fill, never a shadow.** `--shadow-*` are `none`, so every
   `box-shadow: var(--shadow-raise-sm)` in the shared CSS disappears. Elevation
   is a step up the neutral ramp plus a hairline.
2. **One radius scale, one hairline weight.** 4/6/8/10/14px, and
   `rgba(255,255,255,.08)`. Anything that needs a second border to read is
   over-designed.
3. **The accent is ink, not paint.** Selection, focus, the active workspace, a
   focused window's border. Never a gradient, never a bloom, never the fill of a
   large surface. Default accent is a cold white, so the UI has no colour until
   the user picks one.
4. **One solid thing per screen.** Exactly one accent *fill* per view: the user's
   chat turn, the calculator's `=`, a playing pad.
5. **Blur is zero.** `--glass-blur: 0px` plus a hard `backdrop-filter: none` on
   the surfaces that pass a literal blur. Nothing needs a backdrop to read
   through because nothing is translucent.
6. **Pills become rectangles.** Every `border-radius: 999px` rule in the shared
   CSS is overridden. Genuinely circular affordances (a knob, an avatar, the
   voice lip, a running-app dot) keep their round shape.

## Deliberately kept

- **Document canvases stay themselves.** A PDF page is white, a spreadsheet cell
  grid keeps its hairlines, an Impress slide keeps its palette, a terminal stays
  near-black, a browser viewport keeps white. Inverting a document's own
  background would be wrong, not minimal — each is framed by a hairline so it
  reads as a document inside a window rather than a hole in it.
- **Studio's per-track colour.** It is data (which track is which), not
  decoration, so it survives into a monochrome UI.
- **Semantic colours.** `--ok` / `--warn` / `--error` stay legible as state;
  urgency is a 2px left hairline rather than a filled banner.

## One thing this theme does not do

It changes no JavaScript. The ambient glow (`glow.js`) still writes `--glow-*`
per window; Pitch just sets the opacity to zero and hides the layer, so
switching back to Noir or Neumorphic restores the glow intact.

See `/root/test/inspiration/README.md` for the source references these rules
were derived from (AOSP `system_neutral1` dark tones, Material 3 tonal
elevation).