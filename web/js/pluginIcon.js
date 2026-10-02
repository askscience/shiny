/**
 * pluginIcon.js — per-plugin icons.
 *
 * Resolution order for a plugin's icon:
 *   1. the curated KDE app icon mapped to this plugin (`PLUGIN_ICONS`),
 *   2. the plugin's own `web/icon.svg` (served at `/plugins/<name>/icon.svg`),
 *   3. a core-window theme icon, for built-in windows,
 *   4. the caller's `fallback` (`ui/puzzle` by default).
 *
 * The mapped and fallback icons come from the unified UI library
 * (`/ui/icons/`), so they follow the active theme and the user's accent.
 * The plugin's own SVG is inlined the same way.
 */
import { setIcon, loadIconSvg } from '../ui/index.js';
import { isCoreWindow, coreWindowIcon } from './coreWindows.js';

/**
 * Plugin name → shared UI icon. Each name points at a curated icon under
 * `web/ui/icons/apps/` (generated from the KDE Slot-Beauty set by
 * `scripts/kde-icons/convert.py`). A plugin missing from this map falls back
 * to its own `web/icon.svg`, then to `ui/puzzle`.
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

/** The themed SVG text for a plugin: mapped KDE icon, else its own icon. */
async function loadSvg(name) {
  const mapped = PLUGIN_ICONS[name];
  if (mapped) {
    const svg = safeSvg(await loadIconSvg(mapped));
    if (svg) return svg;
  }
  return loadPluginIconSvg(name);
}

/**
 * Create a `<span class="ui-icon">` filled with the plugin's icon. Uses the
 * mapped KDE icon when one exists, else the plugin's own `web/icon.svg`, else
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

  loadSvg(name).then((svg) => {
    if (svg) span.innerHTML = svg;
    else void setIcon(span, isCoreWindow(name) ? coreWindowIcon(name) : fallback, { size, label });
  });
  return span;
}
