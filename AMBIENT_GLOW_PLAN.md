# Ambient Glow (Blur) Effect — Rollout Plan for Every Plugin

> **Status: implemented (all phases).** Tier 0 is live in core and every window
> glows; Radio/YouTube were migrated; Image, PDF, Studio, Mail, Calendar,
> Calculator, Word, Calc and Impress have Tier 1 subject overrides; the
> translucency pass and the Traveler map rim are in. Verified in a real browser:
> 12/12 windows render a glow, 4 with live subjects, no page errors. See §11 for
> the as-built file map.

> **Goal.** The Radio and YouTube windows mirror their current subject — station
> artwork / video thumbnail — as a huge blurred, dimmed layer behind the whole
> window. **Every** window should get that effect. This plan makes a *colored*
> glow the universal default (no per-plugin work at all), then lets each plugin
> **upgrade** it with its real subject (photo, PDF page, track color, …).

---

## 1. Two tiers (the core idea)

| Tier | Who provides it | What it is | Work per plugin |
|---|---|---|---|
| **Tier 0 — color glow** | **Core, automatically** | global accent + a second, per-window "partner" color | **zero** — injected for every current *and future* window |
| **Tier 1 — subject glow** | the plugin | real content: photo, page, thumbnail, artwork, waveform | optional, one `setGlow()` call |

This answers the "no subject" problem: most plugins (Calc, Calculator, Calendar,
Mail, Word, Impress…) have no bitmap to blur. Today they'd get nothing. With
Tier 0 they all glow immediately, on-brand, and never look dead.

If a plugin has a real subject, its Tier 1 override replaces the color
gradient; when the subject goes away the window **falls back to its color
glow** instead of turning off. Every window is always alive.

---

## 2. How the effect works today (the recipe)

Two plugins implement it, identically. Radio:

- **CSS** — `web/css/tiles.css:617-629`
  ```css
  .radio-glow {
    position: absolute;
    inset: -10%;
    background-size: cover;
    background-position: center;
    filter: blur(90px) saturate(1.3) brightness(0.5);
    opacity: 0;
    transition: opacity var(--duration-slow) var(--ease-out);
    pointer-events: none;
    z-index: 0;
  }
  .radio-glow--on { opacity: 0.5; }
  ```
- **JS** — `plugins/radio/web/plugin.js`
  - created in `mountRadioTile()` (line ~495) and appended as the tile's first child;
  - updated in `renderHero()` (lines ~332-341):
    ```js
    if (art) {
      glowEl.style.backgroundImage = `url("${art}")`;
      glowEl.classList.add('radio-glow--on');
    } else {
      glowEl.style.backgroundImage = '';
      glowEl.classList.remove('radio-glow--on');
    }
    ```

YouTube is the same, one-for-one: `.yt-glow` at `web/css/tiles.css:921-933`,
created in `mountYoutubeTile()` (`plugins/youtube/web/plugin.js:256-259`) and
updated in `renderHero()` (lines ~55-64).

### The anatomy

| Piece | Value | Why |
|---|---|---|
| Layer | absolutely positioned child of `.tile`, `inset: -10%` | bleeds past the rounded frame so the blur has no hard edge |
| `filter` | `blur(90px) saturate(1.3) brightness(0.5)` | 90px melts detail into ambient color; dimmed so text stays readable |
| `background-size` | `cover`, centered | source can be any aspect ratio |
| `opacity` | `0 → 0.5`, `--duration-slow` transition | fades in/out, never snaps |
| `pointer-events` | `none` | never intercepts clicks |
| `.tile` base | already `position: relative; overflow: hidden` (`tiles.css:104-122`) | no per-plugin positioning needed |

### The one hard constraint (read before planning any plugin)

The glow shines **through** whatever is painted above it. It is visible where
the plugin's content is transparent or translucent, and hidden behind opaque
panels. Today:

| Plugin | Main content container | Background | Glow visible? |
|---|---|---|---|
| Radio | grid / hero | transparent + `--glass` | **yes (reference)** |
| YouTube | grid / hero | transparent + `--glass` | **yes (reference)** |
| Mail | `.mail-list`, `.mail-reader` | `--bg` (opaque), `--surface` (0.86) | in chrome only |
| Calendar | `.calendar-grid`, `.calendar-detail` | `--bg` (opaque), `--surface` | in chrome only |
| Calc | `.calc-grid` | `--bg` (opaque) | in chrome only |
| Calculator | `.calculator-keys`, `.calculator-display` | `--bg` / `--surface` | in chrome only |
| Word | `.word-editor` | transparent | **yes** |
| Impress | `.impress-stage` | `--bg` (opaque) | in chrome only |
| Image | `.image-stage` | `--bg` (opaque) | in chrome only |
| PDF | `.pdf-canvas` | `--bg` (opaque) | in chrome only |
| Studio | arranger / device panels | `--bg` / `--surface` | in chrome only |
| Traveler (map) | `#map` | opaque Leaflet tiles | only above the map |

**Important:** every plugin's top bar / toolbars / headers use `var(--glass)`
(translucent), so even with fully opaque content the Tier 0 glow is *visibly*
present at the top of every window. That is enough to say "every window has the
effect" with zero per-plugin work. Making it read across the whole surface is a
later enhancement (translucency pass or rim variant — see §6).

---

## 3. Tier 0 — the universal color glow

### 3.1 Colors: accent + one partner (not true random)

Recommendation: **don't use `Math.random()`.** Per-load randomness is jarring
(the UI looks different every refresh), can produce clashing or muddy
combinations, and breaks visual identity/screenshots. Use a **deterministic
seed from the plugin name** instead: each window keeps its own stable color
personality, and the *set* of windows looks varied — which is the effect you
actually want.

- **Color A = the global accent** (`var(--accent)`) — always one of the two, so
  every window stays on-brand and follows the user's accent setting.
- **Color B = a partner hue**, the accent's hue rotated by a per-window seeded
  angle. If the accent is white/gray (monochrome themes like Noir's default),
  give the partner a saturation floor so it still blooms into real color.

This is essentially what the theme's existing `--accent` / `--accent-2` /
`--gradient-accent` system already does, extended with per-window variety.

### 3.2 Robust stacking (so injection needs no per-plugin CSS)

Today radio/yt lift their content with `z-index: 1` because the glow sits at
`z-index: 0`. For a *universal* injection we don't want to touch every
plugin's rules. Instead:

- add `isolation: isolate` to `.tile` (this guarantees the tile is a stacking
  context, so a negative-z child cannot escape), and
- give `.tile-glow` `z-index: -1`.

A negative-z child paints **above the tile's own background/border and below
all content**, in every plugin, with no content changes. (`.tile` already has
`container-type: inline-size`, which applies layout containment and most likely
already creates a stacking context; `isolation: isolate` makes it explicit and
guaranteed.)

### 3.3 CSS — add to `web/css/tiles.css`

```css
.tile { isolation: isolate; } /* keep the glow layer inside the window */

/* Ambient glow — the shared blurred-mirror layer for plugin windows.
   Plugins ship no CSS of their own, so this lives in core. */
.tile-glow {
  position: absolute;
  inset: -10%;
  background-size: cover;
  background-position: center;
  filter: blur(90px) saturate(1.3) brightness(0.5);
  opacity: 0;
  transition: opacity var(--duration-slow) var(--ease-out);
  pointer-events: none;
  z-index: -1;               /* above the tile surface, below all content */
}
.tile-glow--on { opacity: var(--glow-opacity, 0.5); }

/* Tier 0 — universal color glow: the global accent plus a per-window partner
   hue. Both are custom properties, so a theme/accent change repaints for free;
   JS only supplies the seeded values (and keeps them fresh, see 3.5). */
.tile-glow--default {
  background-image:
    radial-gradient(65% 75% at var(--glow-x, 20%) var(--glow-y, 0%),
      var(--glow-a, var(--accent)) 0%, transparent 68%),
    radial-gradient(80% 90% at var(--glow-x2, 82%) var(--glow-y2, 100%),
      var(--glow-b, var(--accent-2)) 0%, transparent 72%);
}

/* Tier 1 — subject override paints over the color gradient. */
.tile-glow--subject { background-image: none; } /* inline style supplies it */

/* Never composite a hidden window. */
.tile.hidden .tile-glow { display: none; }

@media (prefers-reduced-motion: reduce) {
  .tile-glow { transition: none; }
}
```

### 3.4 JS — new `web/ui/components/glow.js`

```js
/**
 * Ambient glow — shared blurred-mirror layer for plugin windows.
 * Tier 0: every window gets a seeded accent+partner color glow for free.
 * Tier 1: a plugin overrides it with its real subject via setGlow().
 * See AMBIENT_GLOW_PLAN.md; Radio/YouTube are the reference users.
 */
import { cssVar, hexToRgb } from '../appearance.js';

/** Stable, well-spread angle per window — "random" without per-load jitter. */
function seedAngle(name) {
  let h = 2166136261;
  for (let i = 0; i < name.length; i++) {
    h ^= name.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return Math.abs(h) % 360;
}

function rgbToHsl(r, g, b) { /* standard conversion */ }
function hslToHex(h, s, l) { /* standard conversion */ }

/** Second color: the accent's hue rotated per window. A white/gray accent
 *  gets a saturation floor so monochrome themes still bloom into color. */
function partnerColor(hex, angle) {
  const [r, g, b] = hexToRgb(hex || '#ffffff');
  let [h, s, l] = rgbToHsl(r, g, b);
  if (s < 0.18) { s = 0.55; l = 0.55; }
  return hslToHex((h + angle) % 360, Math.min(s + 0.05, 0.9),
                  Math.min(Math.max(l, 0.42), 0.62));
}

function paintDefault(glowEl, name) {
  const angle = seedAngle(name);
  const accent = cssVar('--accent') || '#ffffff';
  glowEl.style.setProperty('--glow-a', accent);
  glowEl.style.setProperty('--glow-b', partnerColor(accent, angle));
  glowEl.style.setProperty('--glow-x',  `${15 + (angle % 55)}%`);
  glowEl.style.setProperty('--glow-y',  `${(angle * 3) % 35}%`);
  glowEl.style.setProperty('--glow-x2', `${60 + (angle % 35)}%`);
  glowEl.style.setProperty('--glow-y2', `${70 + (angle % 30)}%`);
}

/** Create + show the glow. Safe to call once per tile. */
export function installGlow(tileEl, name) {
  let glow = tileEl.querySelector(':scope > .tile-glow');
  if (!glow) {
    glow = document.createElement('div');
    glow.className = 'tile-glow';
    glow.setAttribute('aria-hidden', 'true');
    tileEl.prepend(glow);
  }
  paintDefault(glow, name);
  glow.classList.add('tile-glow--default', 'tile-glow--on');
  return glow;
}

/** Repaint the Tier 0 colors (theme / accent changed). */
export function refreshGlow(glowEl, name) {
  if (glowEl && glowEl.classList.contains('tile-glow--default')) paintDefault(glowEl, name);
}

/** Tier 1: override with any CSS <image> — url("…") or a gradient.
 *  Pass null to fall back to the window's color glow (never fully off). */
export function setGlow(glowEl, imageCss) {
  if (!glowEl) return;
  if (imageCss) {
    glowEl.style.backgroundImage = imageCss;
    glowEl.classList.add('tile-glow--subject', 'tile-glow--on');
  } else {
    glowEl.style.backgroundImage = '';
    glowEl.classList.remove('tile-glow--subject');
    glowEl.classList.add('tile-glow--on');   // back to Tier 0 colors
  }
}

export const glowUrl = (url) => (url ? `url("${url}")` : null);

/** Downscale a canvas/img/video frame to a tiny JPEG data URL. */
export function glowFromDrawable(source, size = 64) { /* as in §3.5 */ }
```

Add to `web/ui/index.js`:
```js
export { installGlow, refreshGlow, setGlow, glowUrl, glowFromDrawable } from './components/glow.js';
```

### 3.5 Where Tier 0 is injected (one place, all plugins)

`web/js/tiles.js` already runs `ensureWindowChrome(el, name)` for **every** tile
in `renderTiles()` (lines 270 and 290), including the map tile. Add the glow
there:

```js
import { installGlow, refreshGlow } from '../ui/index.js';

function ensureWindowChrome(el, name) {
  if (!el) return;
  installGlow(el, name);                 // ← universal, idempotent
  if (el.querySelector(':scope > .tile-header')) return;
  /* …existing header code… */
}
```

Then keep the seeded colors fresh when the user changes theme or accent:

```js
// once, at module init
for (const evt of ['theme:change', 'appearance:change']) {
  window.addEventListener(evt, () => {
    document.querySelectorAll('#tile-grid > .tile').forEach((el) => {
      refreshGlow(el.querySelector(':scope > .tile-glow'), el.dataset.plugin || '');
    });
  });
}
```

Because the gradient reads `var(--glow-a, var(--accent))` and `var(--glow-b,
var(--accent-2))` through CSS custom properties, a plain theme change already
repaints correctly even without the listener; the listener only refreshes the
*seeded* partner hue. Both paths are cheap.

**Result:** every window — including plugins written after this change, and the
map — gets a distinct, on-brand ambient glow with zero plugin code.

---

## 4. Tier 1 — per-plugin subject overrides

Each plugin keeps its `glowEl` reference (or re-queries `tileEl`), and calls
`setGlow(glowEl, …)` when its subject changes; `setGlow(glowEl, null)` returns
it to the Tier 0 color glow. The rest of this section is the "upgrade" plan.

| Plugin | Subject (override) | Tier 0 fallback it inherits | Effort |
|---|---|---|---|
| **Image** | the photo being edited (canvas snapshot) | seeded accent glow | S |
| **PDF** | current page render (reuse thumb cache) | seeded accent glow | S |
| **Word** | first embedded image, else title-seeded gradient | seeded accent glow | S |
| **Calculator** | result-driven gradient (error bloom) | seeded accent glow | S |
| **Impress** | slide-theme gradient | seeded accent glow | M |
| **Studio** | selected/playing track color (+ optional live scope) | seeded accent glow | M |
| **Mail** | deterministic sender-hash color pair | seeded accent glow | M |
| **Calendar** | month hue × day event load | seeded accent glow | M |
| **Calc** | heatmap of numeric cells as a tiny canvas | seeded accent glow | M |
| **Traveler (map)** | Carto tile block at map center | seeded accent glow | M |
| **Radio** | station artwork (existing behaviour) | seeded accent glow | migrate |
| **YouTube** | video thumbnail (existing behaviour) | seeded accent glow | migrate |
| **Hello / Keyboard** | no window surface — N/A | — | — |

### Details

- **Image** (`image-tile`) — `mountImageTile()` (line 686) grabs the glow.
  `loadPixels()` (174) and `renderRaw()` (166) call
  `setGlow(glow, glowFromDrawable(canvasEl))`, **debounced ~120 ms** because
  `renderRaw` runs per streamed preview frame. No image → `setGlow(glow, null)`.
- **PDF** (`pdf-tile`) — `renderMain()` (206) already resolves the page URL; add
  the cheap `pageUrl(id, currentPage, THUMB_DPI)` (48 dpi, usually rail-cached)
  and `setGlow(glow, glowUrl(thumbUrl))`. Reuses `renderCache`; no new fetch.
- **Word** (`word-tile`) — `.word-editor` is already transparent, so this is the
  best-value office plugin. Use `editorEl.querySelector('img')?.src` if present,
  else a warm title-seeded gradient. *Verify first* that the ODT→HTML converter
  preserves embedded images (grep of `src/` found none).
- **Calculator** (`calculator-tile`) — `renderDisplay()` (54) is the single choke
  point: neutral accent when idle, brighter bloom scaled by `lastResult`, brief
  warm/red bloom from `equals()` (88) on error.
- **Impress** (`impress-tile`) — no bitmaps in decks. Build a gradient from the
  slide theme tokens already in CSS at `tiles.css:2065-2069`
  (`--im-accent`, `--im-dark-a`, `--im-dark-b`); update on `openDeck()` (412),
  theme change (740) and `selectSlide()` (405). `setGlow` accepts gradients.
- **Studio** (`studio-tile`) — `trackColor(i)` (371) already provides the DAW
  palette: build a gradient from the selected/playing track. Transport functions
  `renderArrangementAndPlay` (783), `playSaved` (910), `stopPlayback` (771)
  toggle live intensity. Ambitious option: throttle the scope canvases
  (`scopeTraceEl`/`scopeSpecEl`, 4239-4241) through `glowFromDrawable` at ~10 fps.
- **Mail** (`mail-tile`) — `openMessage()` (323) / `renderMessage()` (710): hash
  the sender address to a stable two-color pair (same idea as Gmail avatars).
  Optional real imagery: sender-domain favicon — but that adds a network
  dependency, so keep the hash gradient as default.
- **Calendar** (`calendar-tile`) — hue from the month, intensity from that day's
  event count; update at `selectDate()` (225) / `renderGrid()` (122).
- **Calc** (`calc-tile`) — render numeric cells as a ~64×32 heatmap canvas
  (normalized to min/max, theme accent ramp) → `glowFromDrawable`. One pass on
  `openSheet()` (392) and the end of `persist()` (351). Simpler fallback: hash
  gradient like Word.
- **Traveler** (`tile--map`) — the map is opaque, so the behind-glow only shows
  above the map (i.e. nowhere). Build a Carto tile block at the map center (same
  `MAP_TILES` / mode switch as `web/js/map.js:7-13,62`; the Leaflet layer already
  sets `crossOrigin: 'anonymous'`), composite 3×3 tiles to a small canvas, and
  feed `glowFromDrawable`. Update on `moveend`/`zoomend` (debounced ~250 ms) and
  `setMapTheme` (132). **Because the map covers the glow, this only becomes
  visible with the rim variant (§6) — treat as phase 3 / optional.**
- **Radio / YouTube** — migrate to `installGlow` + `setGlow(glowUrl(art))` /
  `setGlow(glowUrl(current.thumbnail))`. Watch them closely: their current
  content blocks carry `z-index: 1` for the old `z-index: 0` glow; with the new
  `z-index: -1` glow those lifts become harmless but can be cleaned up.

---

## 5. Rollout order

| Phase | Scope | Why |
|---|---|---|
| **0a** | `.tile-glow` CSS + `glow.js` + injection in `ensureWindowChrome` | **every window glows at once** — the whole ask, in one file pair |
| **0b** | Migrate Radio/YouTube to the shared layer | proves the override path; their real subjects still win |
| **1** | Image, PDF, Studio | real-image subjects, highest wow per line |
| **2** | Mail, Calendar, Calculator, Word, Calc, Impress | derived gradients / small heatmap |
| **3** | Translucency or rim pass + Traveler | only needed for opaque, full-bleed content |

Phase 0a alone satisfies "every window has the blur effect." Everything after
is optional polish and can be cherry-picked per plugin.

---

## 6. Making the glow read across opaque surfaces (phase 3, optional)

Tier 0 is visible in every window's translucent header/toolbar. To make it read
across the *whole* window, pick one per plugin:

- **A. Translucency pass** — replace opaque `background: var(--bg)` on the main
  content with e.g. `color-mix(in srgb, var(--bg) 86%, transparent)`. Readable
  (~14:1 contrast) and matches how Radio/YouTube already feel. Best for
  Word, Mail, Calendar, Calc, Calculator, Studio, Impress.
- **B. Rim/vignette variant** — a second, on-top glow masked to the window edges
  (`z-index` above content, `pointer-events: none`). The only option for content
  that must stay opaque and full-bleed: Image stage, PDF canvas, the Leaflet map.
  ```css
  .tile-glow--rim {
    inset: 0; z-index: 3;
    filter: blur(48px) saturate(1.2) brightness(0.7);
    mask-image: radial-gradient(115% 100% at 50% 50%, transparent 55%, #000 100%);
    -webkit-mask-image: radial-gradient(115% 100% at 50% 50%, transparent 55%, #000 100%);
  }
  ```
  Cost: a second blurred layer per window — add it only to the few opaque
  windows, not universally.

---

## 7. Cross-cutting concerns

- **Performance.** `blur(90px)` is GPU work. Tier 0 uses gradients (no image
  decode, no network) and a 64px source for Tier 1. Let the browser composite —
  only `opacity` transitions; never animate `filter`. `.tile.hidden .tile-glow
  { display: none }` keeps unfocused windows free. On phones consider
  `--glow-opacity: 0.35` and `blur(64px)`.
- **Light themes.** `brightness(0.5)` turns white pages gray. In light mode use
  `brightness(0.9) saturate(1.15)` and opacity ~0.35 (drive from a theme
  selector or a variable set by `theme-loader.js`). Verify on `light` and
  `neumorphic-light`.
- **Monochrome accents.** Noir's default accent is pure white; the partner-color
  saturation floor (§3.4) is what gives those windows actual color. Test with the
  default accent, not just a colorful one.
- **Theme/accent changes.** The gradient reads CSS variables, so it repaints
  automatically; the `theme:change` / `appearance:change` listener only refreshes
  the seeded partner hue. Both paths are cheap.
- **Window opacity.** `--window-opacity` (Settings → Desktop) changes how much
  desktop bleeds through; check the glow at 100% and 60%.
- **Determinism.** Seeded by plugin name → stable across reloads, users and
  screenshots. Never `Math.random()`.
- **Accessibility.** `aria-hidden`, `pointer-events: none`, and no transition
  under `prefers-reduced-motion`. The glow is ambient, not a focus indicator —
  it must not compete with `.tile--focused`.
- **Plugins ship no CSS** — that is exactly why Tier 0 must be injected in core
  (`web/js/tiles.js`) and styled in `web/css/tiles.css`.
- **Live reload.** Core CSS/JS (`web/`) is served directly and picks up on
  refresh. Plugin surfaces are served from `data/plugins/<name>/web/`, so edits
  to `plugins/<name>/web/plugin.js` must be copied there (or the plugin
  reinstalled) before they show.

---

## 8. Verification

1. Apply the Tier 0 changes (`web/css/tiles.css`, `web/ui/components/glow.js`,
   `web/ui/index.js`, `web/js/tiles.js`).
2. Hard-refresh the GUI, activate several plugins. **All** windows should now
   show a distinct glow in their header area, and each should look different.
3. Cycle accents and themes → every window repaints, stays on-brand.
4. Check light theme, reduced motion, phone width, and both window-opacity
   settings.
5. For a Tier 1 plugin: apply the change, copy `plugins/<name>/web/plugin.js` →
   `data/plugins/<name>/web/plugin.js`, refresh, confirm the subject override
   appears and falls back to the color glow when the subject closes.
6. Confirm Radio/YouTube are pixel-identical to before their migration.

---

## 9. Open questions

1. **Partner color strategy.** Seeded hue rotation off the global accent (my
   recommendation), or accent + the theme's `--accent-2` verbatim (less variety,
   zero JS), or a fully random per-window palette?
2. **Rim by default?** Add the on-top rim to *every* window now so opaque
   content also glows, or keep Tier 0 behind-only (cheapest) and add the rim per
   plugin in phase 3?
3. **Translucency pass.** OK to make opaque main panels slightly translucent, or
   keep them opaque and use rim-only for those windows?
4. **Traveler.** Include the map aura (extra tile fetches) or skip?
5. **User control.** Add a Settings → Desktop "Ambient glow" toggle / intensity
   slider (`--glow-opacity` already makes this a one-liner)?

---

## 10. File-by-file change summary

| File | Change |
|---|---|
| `web/css/tiles.css` | `.tile { isolation: isolate }`; add `.tile-glow`, `--default`, `--subject` (+ optional `--rim`); per-plugin translucency later |
| `web/ui/components/glow.js` | **new** — `installGlow`, `refreshGlow`, `setGlow`, `glowUrl`, `glowFromDrawable`, color seeding |
| `web/ui/index.js` | export the glow helpers |
| `web/js/tiles.js` | call `installGlow(el, name)` in `ensureWindowChrome`; refresh on theme/accent change |
| `plugins/radio/web/plugin.js`, `plugins/youtube/web/plugin.js` | migrate to `setGlow` (phase 0b) |
| `plugins/image/web/plugin.js` | `setGlow` from canvas (debounced) |
| `plugins/pdf/web/plugin.js` | `setGlow` from current page thumb |
| `plugins/studio/web/plugin.js` | `setGlow` from track color / transport state |
| `plugins/mail/web/plugin.js` | `setGlow` sender-hash gradient |
| `plugins/calendar/web/plugin.js` | `setGlow` month/day-load gradient |
| `plugins/calculator/web/plugin.js` | `setGlow` result-driven gradient |
| `plugins/word/web/plugin.js` | `setGlow` first image, else title gradient |
| `plugins/calc/web/plugin.js` | `setGlow` cell-value heatmap canvas |
| `plugins/impress/web/plugin.js` | `setGlow` slide-theme gradient |
| `web/js/map.js` + `web/js/tiles.js` | map rim glow (phase 3) |
| `data/plugins/<name>/web/plugin.js` | copy of each edited surface (served copy) |

---

## 11. As-built (implemented)

Decisions taken while building: seeded accent + partner hue (not per-load
random); Tier 0 behind-content glow universal, with the on-top rim only for the
opaque map; translucency pass at 88% on the large opaque panels; light themes
use `brightness(0.9)` + `--glow-opacity: 0.35`.

| File | Change |
|---|---|
| `web/css/tiles.css` | `.tile { isolation: isolate }`; `.tile-glow` / `--default` / `--subject` / `--rim`; hidden-window + reduced-motion rules; translucency pass appended |
| `web/ui/components/glow.js` | **new** — `installGlow`, `glowFor`, `refreshGlow`, `createRim`, `setGlow`, `setTileGlow`, `glowUrl`, `glowGradient`, `glowFromDrawable`, `partnerColor` |
| `web/ui/index.js` | re-exports the glow helpers |
| `web/js/tiles.js` | `installGlow` in `ensureWindowChrome` (every window); theme/accent refresh; map rim + debounced Carto tile sampling |
| `web/js/map.js` | dispatches `map:view` on init, `moveend`/`zoomend` and theme change |
| `plugins/radio|youtube/web/plugin.js` | migrated to `setTileGlow` (no visual change) |
| `plugins/image|pdf|studio|mail|calendar|calculator|word|calc|impress/web/plugin.js` | Tier 1 subject overrides |
| `data/plugins/<name>/web/plugin.js` | served copies synced (`data/` is git-ignored) |

Verification: Playwright (system Chrome) against the live instance — 12/12 tiles
render `.tile-glow`, subjects present for Traveler (sampled tile rim), Studio
(track colours), Calendar (month hue), Calculator (result); light theme applies
`brightness(0.9)` / opacity `0.35`; no page errors.

---

## 12. Theme support (opaque-surface themes: Neumorphic)

Soft-UI themes make every surface the *same opaque colour* (`--glass`,
`--bg`, `--surface` are all `#26292f` dark / `#e4e8ef` light), so the glow was
completely hidden — and `brightness(0.5)` muddied it against graphite.

The intensity is now theme-tunable from CSS instead of being written inline by
JS, with three knobs on `.tile`:

| Variable | Default | Meaning |
|---|---|---|
| `--glow-brightness` | `0.5` (`0.9` light) | filter brightness of the glow |
| `--glow-opacity` | `0.5` (`0.35` light) | how strong the light is |
| `--glow-permeability` | `88%` | how much the content panels let it through |

`glow.js` only seeds the colours/positions and toggles `data-glow-light` on the
tile; the dark/light numbers live in `tiles.css`, so a theme's `app.css`
(loaded after the core CSS) can override them.

The Neumorphic themes then add, scoped to `.tile` so the rest of the app is
untouched:

```css
.tile {
  --glass:   color-mix(in srgb, var(--bg-elevated) 52%, transparent);
  --surface: color-mix(in srgb, var(--bg-elevated) 72%, transparent);
  --glow-permeability: 72%;
  --glow-brightness: 0.82;
  --glow-opacity: 0.55;
}
.tile[data-glow-light] { --glow-brightness: 0.95; --glow-opacity: 0.4; }
```

Because `--glass`/`--surface` are redefined on the window, every plugin bar,
toolbar and panel inside it goes translucent automatically — no per-plugin
selectors. Verified: 12/12 windows glow in Neumorphic, the traveler rim still
samples a real tile, and Noir/Light are unchanged (0.5/0.5 and 0.9/0.35).

---

## 13. Performance — the disappearing blur

**Problem.** Dragging/resizing a window lagged badly. The culprit was the live
`filter: blur(90px)` on a near-window-sized layer: when a tile resizes, the
browser must re-rasterize that blur *every frame*.

**Measurement** (headless Chrome, 6 windows, 90 scripted resize frames; average
frame time — lower is better):

| Glow filter | avg frame |
|---|---|
| `blur(90px) saturate(1.3) brightness(0.5)` | **29.5 ms** |
| `blur(18px) saturate(1.3) brightness(0.5)` | 20.0 ms |
| `saturate(1.3) brightness(0.5)` (no blur) | 16.9 ms |
| `none` | 16.7 ms |

So the spatial blur was ~100% of the cost; the colour filters are free.

**Fix — move the blur off the resize path:**

1. **No `filter: blur()` at window size.** Tier 0 gradients now use a
   multi-stop falloff that approximates a Gaussian, so they are soft without
   any blur at all.
2. **Tier 1 images are pre-blurred at thumbnail size.** `glowFromDrawable()`
   draws to a ~64px canvas with `ctx.filter = 'blur(2.5px)'` (with overscan so
   the soft edges are cropped). Scaled up to a window that reads as ~30px of
   blur, but it is baked once instead of recomputed per frame.
3. **Layer area cut** from `inset: -10%` (144% of the tile) to `-6%`.
4. **Rim variant** keeps only its radial mask and colour filter — the mask
   supplies the soft edge, no blur.

**Result:** with-glow vs no-glow resize frame time is now **22.3 ms vs 23.0 ms**
— statistically identical. The effect is free during interaction.

If a device is still marginal, the remaining lever is a transient
`.tile.is-dragging .tile-glow, .tile.is-resizing .tile-glow { opacity: 0 }`
(desktop.js already adds those classes), but the measurements say it is not
needed.

---

## 14. Colour intensity — the theme-colour veil

At full strength the washes read as saturated colour bands rather than ambient
light. A **semi-transparent wash of the window colour** is now laid over the
glow, so the hue persists but is muted toward the surface:

```css
.tile { --glow-veil: color-mix(in srgb, var(--bg) 58%, transparent); }
.tile[data-glow-light] { --glow-veil: color-mix(in srgb, var(--bg) 62%, transparent); }
```

`--bg` is the theme's own surface colour, so it is automatically a
semi-transparent **dark** wash in dark themes (`#000000` Noir, `#26292f`
Neumorphic) and a semi-transparent **light** wash in light themes (`#f4f4f5`
Light, `#e4e8ef` Neumorphic-light). Themes can tune or remove it from their
`app.css` (`--glow-veil: transparent` restores the old strength).

Implementation: the glow image now lives in a `--glow-image` custom property
(set by `setGlow`), and `.tile-glow` composes the veil as the top background
layer:

```css
background-image:
  linear-gradient(var(--glow-veil, transparent), var(--glow-veil, transparent)),
  var(--glow-image, none);
```

That keeps the veil working for *both* Tier 0 gradients and Tier 1 images
(which used to override `background-image` inline and would have dropped the
veil). Cost is one extra solid-colour layer — measured resize frame time is
unchanged (16.7 ms vs 16.7 ms with the glow hidden).
