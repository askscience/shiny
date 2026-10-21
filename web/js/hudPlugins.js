import { controlButton } from '../ui/components/button.js';
/**
 * hudPlugins.js — the plugin icon tray in the top HUD bar.
 *
 * One icon per installed plugin (active or inactive), sitting to the right of
 * the separator after the weather widget. Clicking an inactive icon activates
 * the plugin; clicking an active icon brings its window to the front (where it
 * has one). Neither touches focus mode — that is the eye button, the icon's
 * right-click menu, or the AI.
 *
 * This replaces the old phone-only "open windows" switcher — the tray shows
 * every plugin, not just the ones currently open, and is visible at all sizes.
 *
 * Two states ride on an icon: `is-active` (the plugin is running — the accent
 * plus the underline) and `is-focused` (focus mode, an explicit choice the user
 * or the AI makes, possibly for several plugins — the accent plus a plate).
 */
import { apiFetch } from './api.js';
import { refreshActivePlugins, refreshPluginCatalog } from './activePlugins.js';
import { hudDivider } from './hudDivider.js';
import { pluginIconEl } from './pluginIcon.js';
import { isPluginFocused } from './desktop.js';
import { t } from './i18n.js';

const trayEl = document.getElementById('hud-plugins');

let plugins = [];       // full /api/plugins list
let activeSet = new Set(); // session-active names

function label(name) {
  return name.charAt(0).toUpperCase() + name.slice(1);
}

/** Accessible name for a tray icon — "active" (running) and "focused" (in focus
 *  mode) are different states and both belong in it. */
function trayLabel(name) {
  const state = activeSet.has(name) ? 'active' : 'inactive';
  return `${label(name)} (${state}${isPluginFocused(name) ? ', focused' : ''})`;
}

async function refresh(force = false) {
  try {
    const [list, active] = await Promise.all([
      refreshPluginCatalog({ force }),
      refreshActivePlugins({ force }),
    ]);
    plugins = list || [];
    activeSet = new Set(active || []);
  } catch (_) {
    // Keep the last known list on transient errors.
  }
  render();
}

function buildButton(p) {
  const active = activeSet.has(p.name);

  const btn = controlButton();
  btn.type = 'button';
  btn.className = 'hud-plugin-btn';
  btn.dataset.plugin = p.name;
  btn.classList.toggle('is-active', active);
  btn.classList.toggle('is-focused', isPluginFocused(p.name));
  btn.title = active
    ? t('hud.pluginActive', { name: label(p.name) }, `${label(p.name)} — active`)
    : t('hud.pluginInactive', { name: label(p.name) }, `${label(p.name)} — tap to activate`);
  btn.setAttribute('aria-label', trayLabel(p.name));
  btn.setAttribute('aria-pressed', String(active));

  btn.appendChild(pluginIconEl(p.name, { size: 24, fallback: 'ui/puzzle' }));

  // A running-app indicator, like an underline tab: present only when active,
  // so an inactive plugin is just a dimmed glyph with no plate or ring.
  const line = document.createElement('span');
  line.className = 'hud-plugin-line';
  line.setAttribute('aria-hidden', 'true');
  btn.appendChild(line);

  btn.addEventListener('click', () => void onClick(p, active));
  return btn;
}

function render() {
  if (!trayEl) return;
  trayEl.textContent = '';

  // Group plugins by their declared category (install order preserved within
  // each group) and separate the groups with a thin divider.
  const groups = [];
  const index = new Map();
  for (const p of plugins) {
    const cat = (p.category || '').trim() || t('plugins.categoryOther');
    let g = index.get(cat);
    if (!g) {
      g = { category: cat, items: [] };
      index.set(cat, g);
      groups.push(g);
    }
    g.items.push(p);
  }

  groups.forEach((group, gi) => {
    if (gi > 0) trayEl.appendChild(hudDivider());
    for (const p of group.items) trayEl.appendChild(buildButton(p));
  });

  trayEl.classList.toggle('hidden', plugins.length === 0);
}

/** Focus mode is applied to the tray without a rebuild: the focused plugins'
 *  glyphs wear the accent, like the window ring and the workspace dot. */
function syncFocusFlags() {
  if (!trayEl) return;
  trayEl.querySelectorAll('button[data-plugin]').forEach((btn) => {
    const name = btn.dataset.plugin;
    btn.classList.toggle('is-focused', isPluginFocused(name));
    btn.setAttribute('aria-label', trayLabel(name));
  });
}

async function onClick(p, active) {
  if (active) {
    // Bring the plugin's window forward (no-op for tool-only plugins).
    window.dispatchEvent(new CustomEvent('plugin:focus', { detail: { name: p.name } }));
    return;
  }
  try {
    await apiFetch('/api/plugins/activate', {
      method: 'POST',
      body: JSON.stringify({ name: p.name }),
    });
  } catch (err) {
    window.dispatchEvent(new CustomEvent('app:toast', {
      detail: { message: err.message || t('plugins.activateFailed', { name: label(p.name) }), type: 'error' },
    }));
    return;
  }
  // Mirror the Plugins page signal so every consumer refreshes. The opened
  // name rides along so the desktop can focus (raise) the new window.
  localStorage.setItem('plugins.changed', String(Date.now()));
  window.dispatchEvent(new CustomEvent('plugins:changed', { detail: { opened: p.name } }));
}

/**
 * Activate a plugin if it is not active yet and bring its window forward —
 * the tray's click behaviour, for other menus (the clock's calendar shortcut).
 */
export async function openPlugin(name) {
  const plugin = plugins.find((item) => item.name === name) || { name };
  await onClick(plugin, activeSet.has(name));
}

export function initHudPlugins() {
  if (!trayEl) return;
  window.addEventListener('plugins:changed', () => void refresh(true));
  // Focus mode is a desktop state change — repaint the glyphs, don't refetch.
  window.addEventListener('desktop:changed', syncFocusFlags);
  window.addEventListener('storage', (e) => {
    if (e.key === 'plugins.changed') void refresh(true);
  });
  void refresh();
}
