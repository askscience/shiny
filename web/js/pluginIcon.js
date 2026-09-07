/**
 * pluginIcon.js — per-plugin icons.
 *
 * Every plugin ships `web/icon.svg` (a 24×24 `currentColor` SVG), served by
 * core at `/plugins/<name>/icon.svg`. This module inlines that SVG into a
 * `<span class="ui-icon">` so it inherits `color` from its host and follows
 * the active theme, falling back to a theme icon for plugins that don't ship
 * an icon (or whose icon fails to load).
 */
import { setIcon } from '../ui/index.js';

const cache = new Map(); // name -> Promise<string|null>

/** Reject SVGs that could execute script/handlers when inlined. */
function safeSvg(text) {
  if (!text) return null;
  if (/<\s*script|on\w+\s*=|javascript:/i.test(text)) return null;
  return text;
}

export function loadPluginIconSvg(name) {
  if (cache.has(name)) return cache.get(name);
  const p = fetch(`/plugins/${name}/icon.svg`)
    .then((res) => (res.ok ? res.text() : null))
    .then(safeSvg)
    .catch(() => null);
  cache.set(name, p);
  return p;
}

/**
 * Create a `<span class="ui-icon">` filled with the plugin's icon. When the
 * plugin has no `web/icon.svg`, `fallback` (a theme icon path) is used.
 */
export function pluginIconEl(name, { size = 16, fallback = 'ui/puzzle', label = null } = {}) {
  const span = document.createElement('span');
  span.className = 'ui-icon';
  if (size) {
    span.style.width = `${size}px`;
    span.style.height = `${size}px`;
  }
  if (label) {
    span.setAttribute('role', 'img');
    span.setAttribute('aria-label', label);
  } else {
    span.setAttribute('aria-hidden', 'true');
  }

  loadPluginIconSvg(name).then((svg) => {
    if (svg) span.innerHTML = svg;
    else void setIcon(span, fallback, { size, label });
  });
  return span;
}
