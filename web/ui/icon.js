/**
 * icon — inline SVG icons from the active theme.
 *
 * Theme icons live at /themes/<theme>/icons/<group>/<name>.svg and use
 * stroke/fill="currentColor", so they inherit color from CSS and follow
 * the accent automatically. Icons are fetched once and cached; theme
 * assets are trusted (shipped with the app), so inline injection is safe.
 *
 * Usage:
 *   const el = icon('ui/close', { size: 16 });
 *   await setIcon(existingEl, 'artifacts/route');
 */

import { getActiveTheme, themeUrl } from './theme-loader.js';

const cache = new Map(); // `${theme}:${name}` -> Promise<string|null>

function loadSvg(name) {
  const theme = getActiveTheme();
  const key = `${theme}:${name}`;
  if (!cache.has(key)) {
    cache.set(key, fetch(themeUrl(`icons/${name}.svg`))
      .then((res) => (res.ok ? res.text() : null))
      .catch(() => null));
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

/** Create a span that fills itself with the themed SVG once loaded. */
export function icon(name, { size = 18, label = null, className = '' } = {}) {
  const el = document.createElement('span');
  if (className) el.className = className;
  prepare(el, size, label);
  loadSvg(name).then((svg) => {
    if (svg) el.innerHTML = svg;
    else el.classList.add('ui-icon--missing');
  });
  return el;
}

/** Replace the content of an existing element with a themed icon. */
export async function setIcon(el, name, { size = null, label = null } = {}) {
  prepare(el, size, label);
  const svg = await loadSvg(name);
  if (svg) el.innerHTML = svg;
  else el.classList.add('ui-icon--missing');
  return el;
}

/** Drop cached icons (e.g. after a theme switch). */
export function clearIconCache() {
  cache.clear();
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
