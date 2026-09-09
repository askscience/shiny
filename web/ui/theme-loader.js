/**
 * theme-loader — discovers themes under /themes/, activates the stored one.
 *
 * A theme is a folder /themes/<name>/ containing:
 *   theme.json      manifest (name, label, accents, gradients, …)
 *   tokens.css      design tokens (colors, type, radii, motion)
 *   components.css  visual skin for every .ui-* component
 *   icons/          SVG icons with stroke/fill="currentColor"
 *
 * Installed themes are listed in /themes/themes.json (ServeDir cannot
 * list directories, so the index is maintained manually).
 */

const THEME_KEY = 'ui.theme.name';
const FALLBACK_THEME = 'noir';

let activeTheme = FALLBACK_THEME;
let manifest = null;

export async function listThemes() {
  try {
    const res = await fetch('/themes/themes.json');
    if (res.ok) {
      const list = await res.json();
      if (Array.isArray(list) && list.length) return list;
    }
  } catch (_) { /* fall through */ }
  return [FALLBACK_THEME];
}

export function getActiveTheme() {
  return activeTheme;
}

export function getThemeManifest() {
  return manifest;
}

/** URL of an asset inside the active theme (icons, images, …). */
export function themeUrl(path) {
  return `/themes/${activeTheme}/${path}`;
}

function setThemeHrefs(theme) {
  const tokens = document.getElementById('theme-tokens');
  const components = document.getElementById('theme-components');
  if (tokens) tokens.href = `/themes/${theme}/tokens.css`;
  if (components) components.href = `/themes/${theme}/components.css`;
}

/**
 * A theme may ship an optional app-surface override (`app` in theme.json,
 * e.g. "app.css") that loads AFTER the app CSS so it can restyle chrome the
 * library doesn't own. Themes without it keep the link disabled so the
 * existing UI renders exactly as before.
 */
function setAppHref(theme, manifest) {
  const app = document.getElementById('theme-app');
  if (!app) return;
  const appFile = manifest && manifest.app;
  const url = appFile ? `/themes/${theme}/${appFile === true ? 'app.css' : appFile}` : '';
  if (url) {
    app.href = url;
    app.disabled = false;
  } else {
    app.disabled = true;
    app.href = url; // empty while disabled → no request, no effect
  }
}

async function loadManifest(theme) {
  try {
    const res = await fetch(`/themes/${theme}/theme.json`);
    if (res.ok) return await res.json();
  } catch (_) { /* fall through */ }
  return { name: theme, label: theme, accents: [], gradients: [] };
}

function applyTheme(theme) {
  activeTheme = theme;
  document.documentElement.dataset.theme = theme;
  setThemeHrefs(theme);
}

/** Boot: pick stored theme, point the two <link> slots at it, load manifest. */
export async function initThemeLoader() {
  const themes = await listThemes();
  const stored = localStorage.getItem(THEME_KEY);
  applyTheme(themes.includes(stored) ? stored : themes[0]);
  manifest = await loadManifest(activeTheme);
  setAppHref(activeTheme, manifest);
  return manifest;
}

/** Switch theme at runtime (persists globally, not per user). */
export async function setTheme(name) {
  const themes = await listThemes();
  if (!themes.includes(name)) return false;
  localStorage.setItem(THEME_KEY, name);
  applyTheme(name);
  manifest = await loadManifest(name);
  setAppHref(name, manifest);
  window.dispatchEvent(new CustomEvent('theme:change', { detail: { theme: name } }));
  return true;
}
