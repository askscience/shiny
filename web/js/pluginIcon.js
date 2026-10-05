/**
 * pluginIcon.js — per-plugin icons.
 *
 * Resolution order for a plugin's icon:
 *   1. the curated icon mapped to this plugin in the active icon set
 *      (`PLUGIN_ICONS` → e.g. `apps/files`; Infinity ships these coloured),
 *   2. the plugin's own `web/icon.svg` (served at `/plugins/<name>/icon.svg`),
 *   3. a core-window theme icon, for built-in windows,
 *   4. the caller's `fallback` (`ui/puzzle` by default).
 *
 * Icons come through the unified resolver (`/ui/index.js`), so they follow the
 * active set, the theme, the accent tint and any theme overrides.
 */
import { setIcon, loadIconSvg, decorateSvg } from '../ui/index.js';
import { isCoreWindow, coreWindowIcon } from './coreWindows.js';

/**
 * Plugin name → shared UI icon. Each name points at a curated icon under the
 * active set's `apps/` group (coloured in the Infinity set). A plugin missing
 * from this map falls back to its own `web/icon.svg`, then to `ui/puzzle`.
 */
export const PLUGIN_ICONS = {
  browser: 'apps/browser',
  calc: 'apps/calc',
  calculator: 'apps/calculator',
  calendar: 'apps/calendar',
  files: 'apps/files',
  hello: 'apps/hello',
  image: 'apps/image',
  impress: 'apps/impress',
  keyboard: 'apps/keyboard',
  mail: 'apps/mail',
  pdf: 'apps/pdf',
  radio: 'apps/radio',
  studio: 'apps/studio',
  terminal: 'apps/terminal',
  traveler: 'apps/traveler',
  word: 'apps/word',
  youtube: 'apps/youtube',
};

const cache = new Map(); // name -> Promise<string|null>

/** Reject SVGs that could execute script/handlers when inlined. */
function safeSvg(text) {
  if (!text) return null;
  if (/<\s*script|on\w+\s*=|javascript:/i.test(text)) return null;
  return text;
}

/**
 * The SVG text for a plugin's own shipped icon, or null when it has none
 * (built-in windows never ship one).
 */
export function loadPluginIconSvg(name) {
  if (cache.has(name)) return cache.get(name);
  // Built-in windows have no web/icon.svg — resolve straight to the fallback.
  if (isCoreWindow(name)) {
    const p = Promise.resolve(null);
    cache.set(name, p);
    return p;
  }
  const p = fetch(`/plugins/${name}/icon.svg`)
    .then((res) => (res.ok ? res.text() : null))
    .then(safeSvg)
    .catch(() => null);
  cache.set(name, p);
  return p;
}

/** The themed SVG text for a plugin: mapped set icon, else its own icon. */
async function loadSvg(name) {
  const mapped = PLUGIN_ICONS[name];
  if (mapped) {
    const svg = safeSvg(await loadIconSvg(mapped));
    if (svg) return { svg, iconName: mapped };
  }
  return { svg: await loadPluginIconSvg(name), iconName: null };
}

/** (Re)render one plugin-icon span from the current set/tint. */
function paint(span, name) {
  const size = Number(span.dataset.pluginSize) || 16;
  const fallback = span.dataset.pluginFallback || 'ui/puzzle';
  const label = span.getAttribute('aria-label');
  loadSvg(name).then(async ({ svg, iconName }) => {
    if (svg) {
      span.innerHTML = await decorateSvg(svg, iconName);
    } else {
      void setIcon(span, isCoreWindow(name) ? coreWindowIcon(name) : fallback,
        { size, label });
    }
  });
}

/**
 * Create a `<span class="ui-icon">` filled with the plugin's icon. Uses the
 * mapped set icon when one exists, else the plugin's own `web/icon.svg`, else
 * `fallback` (a theme icon path) — or the core-window icon for built-ins.
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

  span.dataset.pluginName = name;
  span.dataset.pluginSize = String(size);
  span.dataset.pluginFallback = fallback;
  paint(span, name);
  return span;
}

/** Repaint every live plugin icon (icon-set or tint change). */
export function refreshPluginIcons() {
  document.querySelectorAll('.ui-icon[data-plugin-name]').forEach((span) => {
    paint(span, span.dataset.pluginName);
  });
}

if (typeof window !== 'undefined') {
  window.addEventListener('iconset:change', () => refreshPluginIcons());
  window.addEventListener('tint:change', () => refreshPluginIcons());
}
