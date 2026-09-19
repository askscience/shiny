/**
 * pluginsWindow.js — the Plugins window (built-in desktop surface).
 *
 * An app-store style view of the plugin collection: a search bar, category
 * chips, a card grid with each plugin's icon, description, version and
 * lifecycle actions, plus an install control and a collapsible activity log.
 *
 * Installing/uninstalling is server-wide; activate/deactivate is per-user.
 */
import { apiFetch, getToken } from './api.js';
import {
  toast, icon, button, iconButton, badge, emptyState, searchBar, select,
} from '../ui/index.js';
import { pluginIconEl } from './pluginIcon.js';
import { pickFiles } from './files.js';
import { getPluginLayout, setPluginLayout } from './preferences.js';

export const PLUGINS_WINDOW = 'plugins';

let tileEl = null;
let refs = null;
let cleanups = [];
let plugins = [];
let activeSet = new Set();
let query = '';
let category = 'all';
let busy = false;

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text != null) node.textContent = text;
  return node;
}

function on(target, event, handler, opts) {
  target?.addEventListener(event, handler, opts);
  cleanups.push(() => target?.removeEventListener(event, handler, opts));
}

function niceName(p) {
  return p.name.charAt(0).toUpperCase() + p.name.slice(1);
}

/** Cross-window signal: the tray, tiles and desktop all re-read the plugin set. */
function notifyPluginsChanged(opened) {
  localStorage.setItem('plugins.changed', String(Date.now()));
  window.dispatchEvent(new CustomEvent('plugins:changed', { detail: opened ? { opened } : undefined }));
}

/* ── Data ───────────────────────────────────────────────────── */

async function loadPlugins() {
  try {
    const [listRes, activeRes] = await Promise.all([
      apiFetch('/api/plugins'),
      apiFetch('/api/plugins/active'),
    ]);
    plugins = listRes?.data || [];
    activeSet = new Set(activeRes?.data || []);
  } catch (e) {
    plugins = [];
    activeSet = new Set();
    refs.grid.textContent = '';
    refs.empty.classList.remove('hidden');
    refs.empty.textContent = '';
    refs.empty.appendChild(emptyState({
      icon: 'ui/warning',
      title: "Couldn't load plugins",
      body: e.message || 'Unknown error',
    }));
    return;
  }
  render();
}

async function onPluginAction(action, name) {
  if (busy) return;
  if (action === 'uninstall' && !confirm(`Remove plugin "${name}"? Its database tables will remain.`)) {
    return;
  }
  busy = true;
  setBusy(true);

  const endpoint = action === 'uninstall' ? '/api/plugins/uninstall'
    : action === 'activate' ? '/api/plugins/activate'
    : '/api/plugins/deactivate';

  try {
    await apiFetch(endpoint, { method: 'POST', body: JSON.stringify({ name }) });
    toast(action === 'uninstall' ? `Removed ${name}`
      : action === 'activate' ? `Activated ${name}` : `Deactivated ${name}`);
    notifyPluginsChanged(action === 'activate' ? name : undefined);
  } catch (err) {
    toast(err.message || 'Action failed', { type: 'error' });
  } finally {
    busy = false;
    setBusy(false);
    await refreshActivity();
  }
}

function setBusy(value) {
  refs.grid.querySelectorAll('button').forEach((b) => { b.disabled = value; });
}

/* ── Rendering ──────────────────────────────────────────────── */

function categoriesOf() {
  const set = new Set();
  for (const p of plugins) set.add((p.category || '').trim() || 'Other');
  return [...set].sort((a, b) => a.localeCompare(b));
}

function filtered() {
  const q = query.trim().toLowerCase();
  return plugins.filter((p) => {
    const cat = (p.category || '').trim() || 'Other';
    if (category !== 'all' && cat !== category) return false;
    if (!q) return true;
    return [p.name, p.description, p.summary, cat]
      .filter(Boolean)
      .some((s) => String(s).toLowerCase().includes(q));
  });
}

function renderCategories() {
  refs.categories.textContent = '';
  const cats = ['all', ...categoriesOf()];
  for (const cat of cats) {
    const chip = el('button', 'plugin-chip');
    chip.type = 'button';
    chip.dataset.category = cat;
    chip.textContent = cat === 'all' ? 'All' : cat;
    chip.classList.toggle('is-active', cat === category);
    on(chip, 'click', () => {
      category = cat;
      renderCategories();
      render();
    });
    refs.categories.appendChild(chip);
  }
}

function pluginCard(p) {
  const active = activeSet.has(p.name);
  const card = el('article', 'plugin-card');
  card.classList.toggle('is-inactive', !active);
  card.dataset.plugin = p.name;

  const head = el('div', 'plugin-card-head');
  const iconWrap = el('span', 'plugin-card-icon');
  iconWrap.appendChild(pluginIconEl(p.name, { size: 26, fallback: 'ui/puzzle' }));
  const cat = (p.category || '').trim() || 'Other';
  head.append(iconWrap, badge(cat, { tone: active ? 'accent' : 'neutral' }));

  const name = el('h3', 'plugin-card-name');
  name.textContent = niceName(p);
  const desc = el('p', 'plugin-card-desc');
  desc.textContent = p.description || p.summary || '';

  const meta = el('div', 'plugin-card-meta');
  meta.append(el('span', null, `v${p.version}`), el('span', null, `API ${p.api_level}`));

  const actions = el('div', 'plugin-card-actions');
  if (active) {
    if (p.surface) {
      const openBtn = button({
        label: 'Open',
        variant: 'primary',
        size: 'sm',
        onClick: () => window.dispatchEvent(new CustomEvent('plugin:focus', { detail: { name: p.name } })),
      });
      actions.appendChild(openBtn);
    }
    actions.appendChild(button({
      label: 'Deactivate',
      variant: 'ghost',
      size: 'sm',
      onClick: () => void onPluginAction('deactivate', p.name),
    }));
  } else {
    actions.appendChild(button({
      label: 'Activate',
      variant: 'primary',
      size: 'sm',
      onClick: () => void onPluginAction('activate', p.name),
    }));
  }
  const removeBtn = iconButton({
    icon: 'ui/trash',
    variant: 'ghost',
    size: 'sm',
    label: `Remove ${niceName(p)}`,
    onClick: () => void onPluginAction('uninstall', p.name),
  });
  removeBtn.classList.add('plugin-card-remove');
  actions.appendChild(removeBtn);

  const card_children = [head, name, desc, meta, actions];

  // Per-window mode for active plugins with a window (the keyboard is bottom
  // chrome, so it has no window mode).
  if (active && p.surface && p.name !== 'keyboard') {
    const mode = select({
      options: [
        { value: 'tile', label: 'Tile' },
        { value: 'full', label: 'Full screen' },
      ],
      value: getPluginLayout(p.name),
      onChange: (value) => {
        setPluginLayout(p.name, value);
        toast(`${niceName(p)}: opens as ${value === 'full' ? 'full screen' : 'tile'}`, { type: 'info' });
      },
    });
    mode.classList.add('plugin-card-mode');
    card_children.push(mode);
  }

  card.append(...card_children);
  return card;
}

function render() {
  const list = filtered();
  refs.grid.textContent = '';

  refs.meta.textContent = plugins.length
    ? `${activeSet.size} active · ${plugins.length} installed`
    : '';

  if (!plugins.length) {
    refs.empty.classList.remove('hidden');
    refs.empty.textContent = '';
    refs.empty.appendChild(emptyState({
      icon: 'ui/puzzle',
      title: 'No plugins installed',
      body: "PEAK'D! is running in its bare form. Install a plugin to add tools, routes, and skills.",
    }));
    return;
  }
  if (!list.length) {
    refs.empty.classList.remove('hidden');
    refs.empty.textContent = '';
    refs.empty.appendChild(emptyState({
      icon: 'ui/search',
      title: 'No matching plugins',
      body: 'Try a different search or category.',
    }));
    return;
  }

  refs.empty.classList.add('hidden');
  for (const p of list) refs.grid.appendChild(pluginCard(p));
}

/* ── Install ────────────────────────────────────────────────── */

async function uploadArchive(file) {
  if (busy || !file) return;
  if (!/\.(zip|tar\.gz|tgz)$/i.test(file.name)) {
    toast('File must be .zip or .tar.gz', { type: 'error' });
    return;
  }
  busy = true;
  setBusy(true);
  refs.installBtn.setLoading(true);
  try {
    const form = new FormData();
    form.append('file', file);
    const res = await apiFetch('/api/plugins/install', { method: 'POST', body: form });
    toast(`Installed: ${res?.data?.installed || 'plugin'}`);
    notifyPluginsChanged(res?.data?.installed);
  } catch (e) {
    toast(e.message || 'Install failed', { type: 'error' });
  } finally {
    busy = false;
    refs.installBtn.setLoading(false);
    setBusy(false);
    await refreshActivity();
  }
}

/* ── Activity log ───────────────────────────────────────────── */

async function refreshActivity() {
  if (!refs.activity) return;
  try {
    const headers = {};
    const token = getToken();
    if (token) headers.Authorization = `Bearer ${token}`;
    const res = await fetch('/api/plugins/install.log', { headers });
    if (!res.ok) {
      refs.activity.innerHTML = '<div class="plugins-activity-empty">Activity unavailable.</div>';
      return;
    }
    const text = (await res.text()).trim();
    if (!text) {
      refs.activity.innerHTML = '<div class="plugins-activity-empty">No install events yet.</div>';
      return;
    }
    const lines = text.split('\n').slice(-60);
    refs.activity.textContent = '';
    for (const line of lines) {
      let cls = 'ok';
      if (/reject|failed|missing/i.test(line)) cls = 'error';
      else if (/deactivate|uninstall/i.test(line)) cls = 'warn';
      refs.activity.appendChild(el('div', `plugins-activity-line ${cls}`, line));
    }
  } catch (_) {
    refs.activity.innerHTML = '<div class="plugins-activity-empty">Activity unavailable.</div>';
  }
}

/* ── Surface ────────────────────────────────────────────────── */

function mountPlugins() {
  if (tileEl) return tileEl;

  tileEl = el('section', 'tile plugins-tile');
  tileEl.dataset.plugin = PLUGINS_WINDOW;

  const root = el('div', 'plugins-window');

  const toolbar = el('div', 'plugins-toolbar');
  const search = searchBar({
    placeholder: 'Search plugins…',
    onInput: (value) => { query = value; render(); },
  });
  const installBtn = button({ label: 'Install', icon: 'ui/upload', variant: 'primary', size: 'sm' });
  on(installBtn, 'click', async () => {
    const [file] = await pickFiles({ accept: '.zip,.tar.gz,.tgz,application/zip,application/gzip' });
    if (file) void uploadArchive(file);
  });
  toolbar.append(search, installBtn);

  const metaRow = el('div', 'plugins-categories-row');
  const categories = el('div', 'plugins-categories');
  categories.setAttribute('role', 'toolbar');
  categories.setAttribute('aria-label', 'Plugin categories');
  const meta = el('span', 'plugins-store-meta');
  metaRow.append(categories, meta);

  const grid = el('div', 'plugins-store-grid');
  const empty = el('div', 'plugins-empty hidden');

  const details = el('details', 'plugins-activity-details');
  details.appendChild(el('summary', null, 'Install activity'));
  const activity = el('div', 'plugins-activity');
  activity.innerHTML = '<div class="plugins-activity-empty">Loading…</div>';
  details.appendChild(activity);
  on(details, 'toggle', () => { if (details.open) void refreshActivity(); });

  root.append(toolbar, metaRow, grid, empty, details);
  tileEl.appendChild(root);

  refs = { grid, empty, meta, categories, activity, installBtn, details };

  // Drag a plugin archive anywhere over the window to install it.
  ['dragenter', 'dragover'].forEach((evt) => {
    on(tileEl, evt, (e) => {
      if (![...(e.dataTransfer?.types || [])].includes('Files')) return;
      e.preventDefault();
      e.stopPropagation();
      root.classList.add('is-dragging');
    });
  });
  ['dragleave', 'drop'].forEach((evt) => {
    on(tileEl, evt, (e) => {
      e.preventDefault();
      e.stopPropagation();
      root.classList.remove('is-dragging');
    });
  });
  on(tileEl, 'drop', (e) => {
    const f = e.dataTransfer?.files?.[0];
    if (f) void uploadArchive(f);
  });

  on(window, 'plugins:changed', () => void loadPlugins());

  renderCategories();
  void loadPlugins();
  return tileEl;
}

function unmountPlugins() {
  for (const dispose of cleanups) {
    try { dispose(); } catch (_) { /* ignore */ }
  }
  cleanups = [];
  tileEl?.remove();
  tileEl = null;
  refs = null;
}

export function getPluginsElement() {
  return tileEl;
}

export function pluginsContextMenu() {
  return [
    { type: 'item', label: 'Refresh', icon: 'ui/refresh', onClick: () => void loadPlugins() },
  ];
}

export default {
  name: PLUGINS_WINDOW,
  icon: 'ui/puzzle',
  mount: mountPlugins,
  unmount: unmountPlugins,
  getElement: getPluginsElement,
  contextMenu: pluginsContextMenu,
};
