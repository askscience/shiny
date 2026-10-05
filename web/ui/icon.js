/**
 * icon — inline SVG icons from the unified UI library, with per-set and
 * per-theme overrides.
 *
 * Resolution order for a name:
 *   1. the active icon set at /ui/iconsets/<set>/<name>.svg (if it ships it),
 *   2. the active theme's override at /themes/<theme>/icons/<name>.svg,
 *   3. the shared/base icon at /ui/icons/<name>.svg.
 *
 * Sets and themes both only need to carry the glyphs they draw differently.
 * Symbolic icons use stroke/fill="currentColor" and inherit color from CSS.
 * Coloured artwork can follow the user's accent when "Accent-tinted icons" is
 * on (Settings → Appearance): each icon's generated palette (index.json) is
 * remapped onto an accent-derived ramp, the same trick the folder has always
 * used. With the option off, coloured icons keep their native paint.
 *
 * Usage:
 *   const el = icon('ui/close', { size: 16 });
 *   await setIcon(existingEl, 'artifacts/route');
 */

import { getActiveTheme, themeUrl } from './theme-loader.js';
import { getActiveIconset, setHas, paletteFor, tintEnabled } from './iconset-loader.js';
import { cssVar, hexToRgb } from './appearance.js';

const cache = new Map(); // `${set}:${theme}:${name}` -> Promise<string|null>

/**
 * Names that fall back to another icon when neither the active set nor a theme
 * ships them. The well-known folder variants only exist in the Infinity set;
 * under Slot-Beauty they resolve to the generic folder (and light themes can
 * still override that).
 */
const ALIASES = {
  'ui/folder-desktop': 'ui/folder',
  'ui/folder-documents': 'ui/folder',
  'ui/folder-downloads': 'ui/folder',
  'ui/folder-music': 'ui/folder',
  'ui/folder-pictures': 'ui/folder',
  'ui/folder-public': 'ui/folder',
  'ui/folder-templates': 'ui/folder',
  'ui/folder-videos': 'ui/folder',
};

/**
 * Fallback palette for the Slot-Beauty folder artwork, used when the base
 * index ships no palette. Each entry is [source hex, position] where position
 * 0 = near-black (deep shadow) and 1 = near-white (top-lit highlight); the
 * accent is scaled between `SHADE_FLOOR` and `SHADE_CEIL` at that position.
 */
const FOLDER_BLUES = [
  ['#3a435f', 0.10], // body shadow
  ['#2c5ba0', 0.30], // body deep
  ['#4077cb', 0.45], // body mid
  ['#4b7fcd', 0.55], // gradient stop (dark)
  ['#5294e2', 0.72], // body light / fold
  ['#739bd9', 0.92], // gradient stop (light)
];
const SHADE_FLOOR = 0.45; // darkest factor applied to the accent
const SHADE_CEIL = 1.18;  // brightest factor (a touch of top light)

/** Mix a hex toward black (t<1) or white (t>1); t in [0, ~1.2]. */
function shadeHex(hex, t) {
  const [r, g, b] = hexToRgb(hex);
  const mix = (c) => (t <= 1 ? c * t : c + (255 - c) * (t - 1));
  return `#${[r, g, b].map((c) => Math.round(mix(c)).toString(16).padStart(2, '0')).join('')}`;
}

/**
 * Remap every colour of a palette onto an accent-derived light→dark ramp.
 * Works on solid fills and gradient stops alike (plain text replacement), so
 * the artwork keeps its shading while following the accent.
 */
export function tintSvg(svg, palette) {
  if (!svg || !Array.isArray(palette) || !palette.length) return svg;
  const accent = (cssVar('--accent') || '#5294e2').trim();
  let out = svg;
  for (const [src, pos] of palette) {
    const t = SHADE_FLOOR + (SHADE_CEIL - SHADE_FLOOR) * Number(pos);
    const dest = shadeHex(accent, t);
    out = out.replaceAll(src, dest).replaceAll(src.toUpperCase(), dest);
  }
  return out;
}

/** Back-compat: the Slot-Beauty folder palette as a tint (see FOLDER_BLUES). */
export function colorizeFolder(svg) {
  return tintSvg(svg, FOLDER_BLUES);
}

async function fetchSvg(url) {
  try {
    const res = await fetch(url);
    return res.ok ? await res.text() : null;
  } catch (_) {
    return null;
  }
}

function loadSvg(name) {
  const set = getActiveIconset() || 'base';
  const theme = getActiveTheme();
  const key = `${set}:${theme}:${name}`;
  if (!cache.has(key)) {
    cache.set(
      key,
      (async () => {
        if (set !== 'base' && await setHas(name)) {
          return fetchSvg(`/ui/iconsets/${set}/${name}.svg`);
        }
        // Theme override first, then the unified base icon set.
        const svg = (await fetchSvg(themeUrl(`icons/${name}.svg`)))
          || (await fetchSvg(`/ui/icons/${name}.svg`));
        if (svg) return svg;
        const alias = ALIASES[name];
        return alias ? loadSvg(alias) : null;
      })(),
    );
  }
  return cache.get(key);
}

/** Apply the accent tint when enabled and the icon has a palette. */
export async function decorateSvg(svg, name) {
  if (!svg || !tintEnabled()) return svg;
  const palette = await paletteFor(name);
  return palette ? tintSvg(svg, palette) : svg;
}

function prepare(el, size, label) {
  el.classList.add('ui-icon');
  if (size) {
    el.style.width = `${size}px`;
    el.style.height = `${size}px`;
  }
  if (label) {
    el.setAttribute('role', 'img');
    el.setAttribute('aria-label', label);
  } else {
    el.setAttribute('aria-hidden', 'true');
  }
  return el;
}

/** Render (and remember) an icon's painter so it can be re-run on changes. */
function paint(el, name) {
  el.dataset.iconName = name;
  loadSvg(name).then(async (svg) => {
    if (!svg) {
      el.classList.add('ui-icon--missing');
      return;
    }
    const palette = tintEnabled() ? await paletteFor(name) : null;
    el.innerHTML = palette ? tintSvg(svg, palette) : svg;
    if (palette) el.dataset.tintable = '1';
    else delete el.dataset.tintable;
  });
}

/** Create a span that fills itself with the themed SVG once loaded. */
export function icon(name, { size = 18, label = null, className = '' } = {}) {
  const el = document.createElement('span');
  if (className) el.className = className;
  prepare(el, size, label);
  paint(el, name);
  return el;
}

/** Replace the content of an existing element with a themed icon. */
export async function setIcon(el, name, { size = null, label = null } = {}) {
  prepare(el, size, label);
  paint(el, name);
  return el;
}

/**
 * Repaint live icons after a set/tint/theme change. `tintable` limits the
 * repaint to coloured icons with a palette — the only ones an accent change
 * can affect.
 */
export function refreshIcons({ tintable = false } = {}) {
  const selector = tintable
    ? '.ui-icon[data-tintable]'
    : '.ui-icon[data-icon-name]';
  document.querySelectorAll(selector).forEach((el) => paint(el, el.dataset.iconName));
}

/** Repaint every live folder glyph (kept for API compatibility). */
export function refreshFolderIcons() {
  document.querySelectorAll('.ui-icon[data-icon-name*="folder"]').forEach((el) => {
    paint(el, el.dataset.iconName);
  });
}

// Accent changes only affect coloured icons while tinting is enabled; icon-set
// and tint switches change every icon.
if (typeof window !== 'undefined') {
  window.addEventListener('appearance:change', () => {
    if (tintEnabled()) refreshIcons({ tintable: true });
  });
  window.addEventListener('accent:change', () => {
    if (tintEnabled()) refreshIcons({ tintable: true });
  });
  window.addEventListener('iconset:change', () => refreshIcons());
  window.addEventListener('tint:change', () => refreshIcons());
}

/** Drop cached icons (e.g. after a theme switch). */
export function clearIconCache() {
  cache.clear();
}

/**
 * The raw themed SVG text for an icon name (set → theme → base). Exposed so
 * callers that need to decide *where* an icon comes from (e.g. plugin icons)
 * can reuse the same lookup instead of duplicating it.
 */
export function loadIconSvg(name) {
  return loadSvg(name);
}

/** Fill every [data-icon] element in a subtree with its themed icon. */
export function hydrateIcons(root = document) {
  root.querySelectorAll('[data-icon]').forEach((el) => {
    if (el.dataset.iconHydrated === el.dataset.icon) return;
    el.dataset.iconHydrated = el.dataset.icon;
    const size = el.dataset.iconSize ? Number(el.dataset.iconSize) : null;
    void setIcon(el, el.dataset.icon, { size });
  });
}
