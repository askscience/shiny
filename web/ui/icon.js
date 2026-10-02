/**
 * icon — inline SVG icons from the unified UI library, with per-theme overrides.
 *
 * A theme may restyle an icon at /themes/<theme>/icons/<group>/<name>.svg;
 * when it does not ship one, the shared icon at /ui/icons/<group>/<name>.svg is
 * used. New icons therefore belong to `/ui/icons/` (the UI library), and a
 * theme only carries the ones it wants drawn differently. Icons use
 * stroke/fill="currentColor", so they inherit color from CSS and follow the
 * accent automatically. Icons are fetched once and cached; UI assets are
 * trusted (shipped with the app), so inline injection is safe.
 *
 * Usage:
 *   const el = icon('ui/close', { size: 16 });
 *   await setIcon(existingEl, 'artifacts/route');
 */

import { getActiveTheme, themeUrl } from './theme-loader.js';
import { cssVar, hexToRgb } from './appearance.js';

const cache = new Map(); // `${theme}:${name}` -> Promise<string|null>

/**
 * The coloured folder glyph is the one icon that ships real paint instead of
 * `currentColor`. Its KDE artwork is a fixed blue ramp; to let it follow the
 * user's accent we remap those six blues onto an accent-derived ramp at load
 * time. Each entry is [source hex, position] where position 0 = near-black
 * (deep shadow) and 1 = near-white (top-lit highlight); the accent is scaled
 * between `SHADE_FLOOR` and `SHADE_CEIL` at that position.
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

/** Recolor a folder SVG's blues to the current accent (no-op for other icons). */
export function colorizeFolder(svg) {
  if (!svg || !svg.includes('#5294e2')) return svg; // not the folder artwork
  const accent = (cssVar('--accent') || '#5294e2').trim();
  let out = svg;
  for (const [src, pos] of FOLDER_BLUES) {
    const t = SHADE_FLOOR + (SHADE_CEIL - SHADE_FLOOR) * pos;
    const dest = shadeHex(accent, t);
    out = out.replaceAll(src, dest).replaceAll(src.toUpperCase(), dest);
  }
  return out;
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
  const theme = getActiveTheme();
  const key = `${theme}:${name}`;
  if (!cache.has(key)) {
    // Theme override first, then the unified UI icon set.
    cache.set(
      key,
      (async () => (await fetchSvg(themeUrl(`icons/${name}.svg`)))
        || (await fetchSvg(`/ui/icons/${name}.svg`)))(),
    );
  }
  return cache.get(key);
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

/** Render (and remember) an icon's painter so it can be re-run on accent change. */
function paint(el, name) {
  el.dataset.iconName = name;
  loadSvg(name).then((svg) => {
    if (svg) el.innerHTML = colorizeFolder(svg);
    else el.classList.add('ui-icon--missing');
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

/** Repaint every live folder icon after the accent changes. */
export function refreshFolderIcons() {
  document.querySelectorAll('.ui-icon[data-icon-name="ui/folder"]').forEach((el) => {
    loadSvg('ui/folder').then((svg) => { if (svg) el.innerHTML = colorizeFolder(svg); });
  });
}

// The folder tint is a function of the accent, so repaint on every change.
if (typeof window !== 'undefined') {
  window.addEventListener('appearance:change', refreshFolderIcons);
  window.addEventListener('accent:change', refreshFolderIcons);
}

/** Drop cached icons (e.g. after a theme switch). */
export function clearIconCache() {
  cache.clear();
}

/**
 * The raw themed SVG text for an icon name (theme override, then shared
 * library), or null when neither ships it. Exposed so callers that need to
 * decide *where* an icon comes from (e.g. plugin icons) can reuse the same
 * theme-aware lookup instead of duplicating it.
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
