/**
 * launcher.js — the plugin launcher (three-finger swipe down).
 *
 * A full-screen grid of every installed plugin, each drawn with its own icon,
 * over the blurred desktop — Launchpad / GNOME app grid, not a dialog. Tapping
 * an inactive plugin activates it; tapping an active one focuses (or opens) its
 * window. It is the top-bar plugin tray at full size — the same `/api/plugins`
 * list and the same activate/focus paths — so the gesture is a second door onto
 * the same room.
 *
 * The gesture is recognised natively (crates/peakd/src/gestures.rs) and mapped
 * here by gestures.js; Escape, an upward swipe and a click on the blurred
 * desktop all dismiss.
 */
import { apiFetch } from './api.js';
import { pluginIconEl } from './pluginIcon.js';

let panel = null;
let grid = null;
let open = false;
let active = new Set();
let plugins = [];

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
    active = new Set(activeRes?.data || []);
  } catch (_) {
    // Keep the last known list on a transient error.
  }
  render();
}

async function choose(p) {
  if (active.has(p.name)) {
    window.dispatchEvent(new CustomEvent('plugin:focus', { detail: { name: p.name } }));
    closeLauncher();
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
  // The same signal the tray and the Plugins page send, so the desktop mounts
  // the new window; `opened` makes it come to the front.
  localStorage.setItem('plugins.changed', String(Date.now()));
  window.dispatchEvent(new CustomEvent('plugins:changed', { detail: { opened: p.name } }));
  closeLauncher();
}

function render() {
  if (!grid) return;
  grid.textContent = '';
  if (!plugins.length) {
    const empty = document.createElement('p');
    empty.className = 'launcher-empty';
    empty.textContent = 'No plugins installed';
    grid.appendChild(empty);
    return;
  }
  for (const p of plugins) {
    const isActive = active.has(p.name);
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = 'launcher-item';
    btn.classList.toggle('is-active', isActive);
    btn.title = p.description || label(p.name);
    btn.setAttribute('aria-label', `${label(p.name)}${isActive ? ' (open)' : ''}`);

    const iconWrap = document.createElement('span');
    iconWrap.className = 'launcher-item-icon';
    iconWrap.appendChild(pluginIconEl(p.name, { size: 40, fallback: 'ui/puzzle' }));

    const name = document.createElement('span');
    name.className = 'launcher-item-name';
    name.textContent = label(p.name);

    // A running app is marked by the dot under its name, the way an app grid
    // does — no "Open/Activate" words.
    const dot = document.createElement('span');
    dot.className = 'launcher-item-dot';
    dot.setAttribute('aria-hidden', 'true');

    btn.append(iconWrap, name, dot);
    btn.addEventListener('click', () => void choose(p));
    grid.appendChild(btn);
  }
}

export function isLauncherOpen() {
  return open;
}

export function openLauncher() {
  if (open) return;
  open = true;
  window.dispatchEvent(new CustomEvent('overlay:open', { detail: { which: 'launcher' } }));
  panel?.classList.remove('hidden');
  document.body.classList.add('launcher-active');
  void refresh();
}

export function closeLauncher() {
  if (!open) return;
  open = false;
  panel?.classList.add('hidden');
  document.body.classList.remove('launcher-active');
}

export function toggleLauncher() {
  if (open) closeLauncher();
  else openLauncher();
}

export function initLauncher() {
  if (panel) return;
  panel = document.getElementById('plugin-launcher');
  grid = document.getElementById('plugin-launcher-grid');
  if (!panel || !grid) return;

  // Like Launchpad, a click on the blurred desktop (outside every icon) leaves.
  panel.addEventListener('click', (e) => {
    if (e.target === panel) closeLauncher();
  });

  window.addEventListener('keydown', (e) => {
    if (!open || e.key !== 'Escape') return;
    e.preventDefault();
    e.stopImmediatePropagation();
    closeLauncher();
  }, true);

  window.addEventListener('overlay:open', (e) => {
    if (open && e.detail?.which !== 'launcher') closeLauncher();
  });
  window.addEventListener('plugins:changed', () => {
    if (open) void refresh();
  });
}
