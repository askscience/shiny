/**
 * hudPlugins.js — the plugin icon tray in the top HUD bar.
 *
 * One icon per installed plugin (active or inactive), sitting to the right of
 * the separator after the weather widget. Clicking an inactive icon activates
 * the plugin; clicking an active icon focuses its window (where it has one).
 *
 * This replaces the old phone-only "open windows" switcher — the tray shows
 * every plugin, not just the ones currently open, and is visible at all sizes.
 */
import { apiFetch } from './api.js';
import { pluginIconEl } from './pluginIcon.js';

const trayEl = document.getElementById('hud-plugins');

let plugins = [];       // full /api/plugins list
let activeSet = new Set(); // session-active names

function label(name) {
  return name.charAt(0).toUpperCase() + name.slice(1);
}

async function refresh() {
  try {
    const [listRes, activeRes] = await Promise.all([
      apiFetch('/api/plugins'),
      apiFetch('/api/plugins/active'),
    ]);
    plugins = listRes?.data || [];
    activeSet = new Set(activeRes?.data || []);
  } catch (_) {
    // Keep the last known list on transient errors.
  }
  render();
}

function buildButton(p) {
  const active = activeSet.has(p.name);

  const btn = document.createElement('button');
  btn.type = 'button';
  btn.className = 'hud-plugin-btn';
  btn.dataset.plugin = p.name;
  btn.classList.toggle('is-active', active);
  btn.title = `${label(p.name)} — ${active ? 'active' : 'tap to activate'}`;
  btn.setAttribute('aria-label', `${label(p.name)} (${active ? 'active' : 'inactive'})`);
  btn.setAttribute('aria-pressed', String(active));

  btn.appendChild(pluginIconEl(p.name, { size: 18, fallback: 'ui/puzzle' }));
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
    const cat = (p.category || '').trim() || 'Other';
    let g = index.get(cat);
    if (!g) {
      g = { category: cat, items: [] };
      index.set(cat, g);
      groups.push(g);
    }
    g.items.push(p);
  }

  groups.forEach((group, gi) => {
    if (gi > 0) {
      const sep = document.createElement('span');
      sep.className = 'hud-divider';
      sep.setAttribute('aria-hidden', 'true');
      trayEl.appendChild(sep);
    }
    for (const p of group.items) trayEl.appendChild(buildButton(p));
  });

  trayEl.classList.toggle('hidden', plugins.length === 0);
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
      detail: { message: err.message || `Could not activate ${label(p.name)}`, type: 'error' },
    }));
    return;
  }
  // Mirror the Plugins page signal so every consumer refreshes.
  localStorage.setItem('plugins.changed', String(Date.now()));
  window.dispatchEvent(new CustomEvent('plugins:changed'));
}

export function initHudPlugins() {
  if (!trayEl) return;
  window.addEventListener('plugins:changed', () => void refresh());
  window.addEventListener('storage', (e) => {
    if (e.key === 'plugins.changed') void refresh();
  });
  void refresh();
}
