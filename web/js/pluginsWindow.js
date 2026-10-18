import { confirmDialog } from '../ui/index.js';
import { element as el, chip as uiChip } from '../ui/index.js';
/**
 * pluginsWindow.js — the Plugins window (built-in desktop surface).
 *
 * An app-store style view of the plugin collection: a search bar, category
 * chips, a card grid with each plugin's icon, description, version and
 * lifecycle actions, plus an install control and a collapsible activity log.
 *
 * Its second tab is the **Craft store**: the Craft App Suite listed from the
 * core's catalog (`/api/plugins/store`), where each app's own web build is
 * installed as a web-app plugin and kept updated (`craftStore.js`).
 *
 * Installing/uninstalling is server-wide; activate/deactivate is per-user.
 */
import { apiFetch, getToken } from './api.js';
import {
  toast, icon, button, iconButton, badge, emptyState, searchBar, select, toggle,
} from '../ui/index.js';
import { pluginIconEl } from './pluginIcon.js';
import { pickFiles } from './files.js';
import { getPluginLayout, setPluginLayout, getStoreAutoUpdate, setStoreAutoUpdate } from './preferences.js';
import { loadStore, refreshStore, installCraftApp, setCraftMcp, craftAppBusy, storeSnapshot } from './craftStore.js';
import { t } from './i18n.js';

export const PLUGINS_WINDOW = 'plugins';

let tileEl = null;
let refs = null;
let cleanups = [];
let plugins = [];
let activeSet = new Set();
let query = '';
let category = 'all';
let busy = false;
/** 'installed' (the plugin list) or 'store' (the Craft store). */
let tab = 'installed';
let storePollTimer = null;
/** Apps that were mid-install at the last store render. */
let storeBusySeen = new Set();



function on(target, event, handler, opts) {
  target?.addEventListener(event, handler, opts);
  cleanups.push(() => target?.removeEventListener(event, handler, opts));
}

function niceName(p) {
  return p.name.charAt(0).toUpperCase() + p.name.slice(1);
}

/** What a user reads for a category; the stored value stays English. */
function categoryLabel(cat) {
  return cat === 'Other' ? t('plugins.categoryOther', null, 'Other') : cat;
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
      title: t('plugins.loadFailed', null, "Couldn't load plugins"),
      body: e.message || t('plugins.unknownError', null, 'Unknown error'),
    }));
    return;
  }
  render();
}

async function onPluginAction(action, name) {
  if (busy) return;
  if (action === 'uninstall'
    && !await confirmDialog(t('plugins.uninstallConfirm', { name },
      `Remove plugin "${name}"? Its database tables will remain.`))) {
    return;
  }
  if (!refs || busy) return;
  busy = true;
  setBusy(true);

  const endpoint = action === 'uninstall' ? '/api/plugins/uninstall'
    : action === 'activate' ? '/api/plugins/activate'
    : '/api/plugins/deactivate';

  try {
    await apiFetch(endpoint, { method: 'POST', body: JSON.stringify({ name }) });
    toast(action === 'uninstall' ? t('plugins.removed', { name }, `Removed ${name}`)
      : action === 'activate' ? t('plugins.activated', { name }, `Activated ${name}`)
        : t('plugins.deactivated', { name }, `Deactivated ${name}`));
    notifyPluginsChanged(action === 'activate' ? name : undefined);
  } catch (err) {
    toast(err.message || t('plugins.actionFailed', null, 'Action failed'), { type: 'error' });
  } finally {
    busy = false;
    setBusy(false);
    await refreshActivity();
  }
}

function setBusy(value) {
  refs?.grid.querySelectorAll('button').forEach((b) => { b.disabled = value; });
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
    const chip = uiChip({ label: cat === 'all' ? t('plugins.categoryAll', null, 'All') : categoryLabel(cat), active: cat === category });
    chip.type = 'button';
    chip.dataset.category = cat;
    chip.addEventListener('click', () => {
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
  head.append(iconWrap, badge(categoryLabel(cat), { tone: active ? 'accent' : 'neutral' }));

  const name = el('h3', 'plugin-card-name');
  name.textContent = niceName(p);
  const desc = el('p', 'plugin-card-desc');
  desc.textContent = p.description || p.summary || '';

  const meta = el('div', 'plugin-card-meta');
  meta.append(
    el('span', null, `v${p.version}`),
    el('span', null, t('plugins.apiLevel', { level: p.api_level }, `API ${p.api_level}`)),
  );

  const actions = el('div', 'plugin-card-actions');
  if (active) {
    if (p.surface || p.web_app) {
      const openBtn = button({
        label: t('plugins.open', null, 'Open'),
        variant: 'primary',
        size: 'sm',
        onClick: () => window.dispatchEvent(new CustomEvent('plugin:focus', { detail: { name: p.name } })),
      });
      actions.appendChild(openBtn);
    }
    actions.appendChild(button({
      label: t('ctx.deactivate'),
      variant: 'ghost',
      size: 'sm',
      onClick: () => void onPluginAction('deactivate', p.name),
    }));
  } else {
    actions.appendChild(button({
      label: t('ctx.activate'),
      variant: 'primary',
      size: 'sm',
      onClick: () => void onPluginAction('activate', p.name),
    }));
  }
  const removeBtn = iconButton({
    icon: 'ui/trash',
    variant: 'ghost',
    size: 'sm',
    label: t('plugins.remove', { name: niceName(p) }, `Remove ${niceName(p)}`),
    onClick: () => void onPluginAction('uninstall', p.name),
  });
  removeBtn.classList.add('plugin-card-remove');
  actions.appendChild(removeBtn);

  const card_children = [head, name, desc, meta, actions];

  // Per-window mode for active plugins with a window (the keyboard is bottom
  // chrome, so it has no window mode).
  if (active && (p.surface || p.web_app) && p.name !== 'keyboard') {
    const mode = select({
      options: [
        { value: 'tile', label: t('plugins.layoutTile', null, 'Tile') },
        { value: 'full', label: t('ctx.fullscreen') },
      ],
      value: getPluginLayout(p.name),
      onChange: (value) => {
        setPluginLayout(p.name, value);
        toast(value === 'full'
          ? t('plugins.opensAsFull', { name: niceName(p) }, `${niceName(p)}: opens as full screen`)
          : t('plugins.opensAsTile', { name: niceName(p) }, `${niceName(p)}: opens as tile`),
        { type: 'info' });
      },
    });
    mode.classList.add('plugin-card-mode');
    card_children.push(mode);
  }

  card.append(...card_children);
  return card;
}

function render() {
  const inStore = tab === 'store';
  refs.categoriesRow.classList.toggle('hidden', inStore);
  refs.installBtn.classList.toggle('hidden', inStore);
  refs.storeActions.classList.toggle('hidden', !inStore);
  if (inStore) renderStore();
  else renderInstalled();
}

function renderInstalled() {
  const list = filtered();
  refs.grid.textContent = '';

  refs.meta.textContent = plugins.length
    ? t('plugins.storeMeta', { active: activeSet.size, total: plugins.length },
      `${activeSet.size} active · ${plugins.length} installed`)
    : '';

  if (!plugins.length) {
    refs.empty.classList.remove('hidden');
    refs.empty.textContent = '';
    refs.empty.appendChild(emptyState({
      icon: 'ui/puzzle',
      title: t('plugins.none', null, 'No plugins installed'),
      body: t('plugins.noneBody', null,
        "PEAK'D! is running in its bare form. Install a plugin to add tools, routes, and skills."),
    }));
    return;
  }
  if (!list.length) {
    refs.empty.classList.remove('hidden');
    refs.empty.textContent = '';
    refs.empty.appendChild(emptyState({
      icon: 'ui/search',
      title: t('plugins.noMatch', null, 'No matching plugins'),
      body: t('plugins.noMatchBody', null, 'Try a different search or category.'),
    }));
    return;
  }

  refs.empty.classList.add('hidden');
  for (const p of list) refs.grid.appendChild(pluginCard(p));
}

/* ── Craft store ────────────────────────────────────────────── */

/** File sizes as a person reads them (the app builds are megabytes). */
function formatSize(bytes) {
  if (!bytes) return '';
  const mb = bytes / (1024 * 1024);
  return `${mb.toFixed(mb < 10 ? 1 : 0)} MB`;
}

/** The store card's icon: the installed app's own icon, or its initial. */
function storeIcon(app) {
  if (!app.installed_version) {
    const disc = el('span', 'store-card-letter');
    disc.textContent = (app.name || app.id).charAt(0);
    return disc;
  }
  return pluginIconEl(app.id, { size: 26, fallback: 'ui/puzzle' });
}

async function openCraftApp(app) {
  if (!activeSet.has(app.id)) await onPluginAction('activate', app.id);
  window.dispatchEvent(new CustomEvent('plugin:focus', { detail: { name: app.id } }));
}

async function installFromStore(id, name) {
  try {
    await installCraftApp(id);
    toast(t('plugins.storeInstallingToast', { name }, `Installing ${name}…`));
    scheduleStorePoll();
    render();
  } catch (e) {
    toast(e.message || t('plugins.installFailed', null, 'Install failed'), { type: 'error' });
  }
}

/** Download and install every app that is missing or outdated. */
async function installAllFromStore() {
  const apps = (storeSnapshot()?.apps || [])
    .filter((app) => !craftAppBusy(app) && (!app.installed_version || app.update_available));
  if (!apps.length) {
    toast(t('plugins.storeAllCurrent', null, 'Every Craft app is up to date'));
    return;
  }
  const bytes = apps.reduce((sum, app) => sum + (app.asset_size || 0), 0);
  const size = bytes ? ` (~${formatSize(bytes)})` : '';
  if (!await confirmDialog(t('plugins.storeInstallAllConfirm', { count: apps.length, size },
    `Install ${apps.length} apps${size}?`))) {
    return;
  }
  for (const app of apps) {
    try {
      await installCraftApp(app.id);
    } catch (e) {
      toast(e.message || t('plugins.installFailed', null, 'Install failed'), { type: 'error' });
    }
  }
  toast(t('plugins.storeQueued', { count: apps.length }, `Installing ${apps.length} apps`));
  scheduleStorePoll();
  render();
}

function storeCard(app) {
  const installed = !!app.installed_version;
  const card = el('article', 'plugin-card store-card');
  card.dataset.plugin = app.id;
  card.classList.toggle('is-inactive', !installed);
  const head = el('div', 'plugin-card-head');
  const iconWrap = el('span', 'plugin-card-icon');
  iconWrap.appendChild(storeIcon(app));
  head.append(iconWrap, badge(
    installed ? t('plugins.storeInstalled', null, 'Installed') : t('plugins.storeNew', null, 'New'),
    { tone: installed ? 'accent' : 'neutral' },
  ));

  const name = el('h3', 'plugin-card-name');
  name.textContent = app.name;
  const desc = el('p', 'plugin-card-desc');
  desc.textContent = app.tagline || '';

  const meta = el('div', 'plugin-card-meta');
  if (installed) meta.append(el('span', null, `v${app.installed_version}`));
  if (app.latest_version && !(installed && app.latest_version === app.installed_version)) {
    meta.append(el('span', null, `v${app.latest_version}`));
  }
  if (app.asset_size) meta.append(el('span', null, formatSize(app.asset_size)));

  const status = el('p', 'store-card-status');
  if (craftAppBusy(app)) {
    status.classList.add('is-busy');
    status.textContent = app.state === 'downloading'
      ? t('plugins.storeDownloading', { percent: app.progress ?? 0 }, `Downloading ${app.progress ?? 0}%`)
      : t('plugins.storeInstallingNow', null, 'Installing…');
  } else if (app.state === 'failed') {
    status.classList.add('is-error');
    status.textContent = app.error || t('plugins.installFailed', null, 'Install failed');
  } else if (installed && app.update_available) {
    status.textContent = t('plugins.storeUpdateReady', null, 'Update available');
  } else if (installed) {
    status.textContent = t('plugins.storeUpToDate', null, 'Up to date');
  } else if (app.error) {
    status.textContent = app.error;
  } else if (app.latest_version) {
    status.textContent = t('plugins.storeVersion', { version: app.latest_version }, `Version ${app.latest_version}`);
  } else {
    status.textContent = t('plugins.storeUnknown', null, 'Version unknown');
  }

  const actions = el('div', 'plugin-card-actions');
  if (craftAppBusy(app)) {
    // The status line already says what is happening; no action to take.
  } else if (!installed) {
    actions.appendChild(button({
      label: t('plugins.storeInstall', null, 'Install'),
      variant: 'primary',
      size: 'sm',
      onClick: () => void installFromStore(app.id, app.name),
    }));
  } else {
    if (app.update_available) {
      actions.appendChild(button({
        label: t('plugins.storeUpdate', null, 'Update'),
        variant: 'primary',
        size: 'sm',
        onClick: () => void installFromStore(app.id, app.name),
      }));
    }
    actions.appendChild(button({
      label: t('plugins.open', null, 'Open'),
      variant: app.update_available ? 'ghost' : 'primary',
      size: 'sm',
      onClick: () => void openCraftApp(app),
    }));
  }
  const removeBtn = iconButton({
    icon: 'ui/trash',
    variant: 'ghost',
    size: 'sm',
    label: t('plugins.remove', { name: app.name }, `Remove ${app.name}`),
    onClick: () => void onPluginAction('uninstall', app.id),
  });
  removeBtn.classList.add('plugin-card-remove');
  if (installed) actions.appendChild(removeBtn);

  card.append(head, name, desc, meta, status);
  if (app.mcp_available) card.appendChild(aiControlRow(app));
  card.appendChild(actions);
  return card;
}

/** On/off switch for an app's AI control (its MCP server). The capability is
 *  focus-scoped by the core, not by this switch: once installed, the agent can
 *  drive the app only while its window is in focus mode. */
function aiControlRow(app) {
  const row = el('div', 'plugin-card-mcp');
  const label = el(
    'span',
    'plugin-card-mcp-label',
    t('plugins.storeAiControl', null, 'AI control'),
  );
  if (app.mcp_installed && app.mcp_version) {
    label.title = t(
      'plugins.storeAiControlVersion',
      { version: app.mcp_version },
      `AI control v${app.mcp_version}`,
    );
  }
  const aiToggle = toggle({
    checked: !!app.mcp_installed,
    onChange: (value) => void setCraftAiControl(app, value),
  });
  aiToggle.setAttribute(
    'aria-label',
    t('plugins.storeAiControlName', { name: app.name }, `AI control for ${app.name}`),
  );
  aiToggle.disabled = craftAppBusy(app);
  row.append(label, aiToggle);
  return row;
}

async function setCraftAiControl(app, enable) {
  if (enable) {
    const size = app.mcp_size ? ` (~${formatSize(app.mcp_size)})` : '';
    const confirmed = await confirmDialog(
      t(
        'plugins.storeAiControlConfirm',
        { name: app.name, size },
        `Give the AI control of ${app.name}${size}? It can drive the app only while its window is in focus mode.`,
      ),
    );
    if (!confirmed) {
      render(); // put the switch back where it was
      return;
    }
  }
  try {
    await setCraftMcp(app.id, enable);
    toast(enable
      ? t('plugins.storeAiControlOn', { name: app.name }, `${app.name}: AI control is ready`)
      : t('plugins.storeAiControlOff', { name: app.name }, `${app.name}: AI control removed`));
  } catch (e) {
    toast(e.message || t('plugins.installFailed', null, 'Install failed'), { type: 'error' });
  }
  scheduleStorePoll();
  render();
}

function renderStore() {
  const data = storeSnapshot();
  const apps = (data?.apps || []).filter((app) => {
    const q = query.trim().toLowerCase();
    if (!q) return true;
    return [app.name, app.id, app.tagline].filter(Boolean)
      .some((s) => String(s).toLowerCase().includes(q));
  });
  refs.grid.textContent = '';
  const total = (data?.apps || []).length;
  refs.meta.textContent = data
    ? t('plugins.storeMetaApps', { count: total }, `${total} apps · updated automatically`)
    : '';

  if (!data) {
    refs.empty.classList.remove('hidden');
    refs.empty.textContent = '';
    refs.empty.appendChild(emptyState({
      icon: 'ui/download',
      title: t('plugins.storeUnavailable', null, 'Store unavailable'),
      body: t('plugins.storeUnavailableBody', null,
        'The catalog could not be loaded. Check the connection and try again.'),
    }));
    return;
  }
  if (!apps.length) {
    refs.empty.classList.remove('hidden');
    refs.empty.textContent = '';
    refs.empty.appendChild(emptyState({
      icon: 'ui/search',
      title: t('plugins.noMatch', null, 'No matching plugins'),
      body: t('plugins.noMatchBody', null, 'Try a different search or category.'),
    }));
    return;
  }
  refs.empty.classList.add('hidden');
  for (const app of apps) refs.grid.appendChild(storeCard(app));
  reportFinishedInstalls(data.apps || []);
  scheduleStorePoll();
}

/** Tell the other surfaces (tray, launcher, tiles) when an app landed. */
function reportFinishedInstalls(apps) {
  const busyNow = new Set(apps.filter(craftAppBusy).map((app) => app.id));
  const finished = [...storeBusySeen].filter((id) => !busyNow.has(id));
  storeBusySeen = busyNow;
  if (finished.some((id) => apps.find((app) => app.id === id)?.installed_version)) {
    notifyPluginsChanged();
  }
}

/** While something is downloading or the catalog is refreshing, keep polling. */
function scheduleStorePoll() {
  if (storePollTimer) {
    clearTimeout(storePollTimer);
    storePollTimer = null;
  }
  if (tab !== 'store' || !refs) return;
  const data = storeSnapshot();
  const busyNow = (data?.apps || []).some(craftAppBusy) || data?.refreshing;
  if (!busyNow) return;
  storePollTimer = setTimeout(async () => {
    storePollTimer = null;
    try { await loadStore(); } catch (_) { /* the status line reports it */ }
    if (refs && tab === 'store') renderStore();
  }, 900);
}

/* ── Install ────────────────────────────────────────────────── */

async function uploadArchive(file) {
  if (busy || !file) return;
  if (!/\.(zip|tar\.gz|tgz)$/i.test(file.name)) {
    toast(t('plugins.badArchive', null, 'File must be .zip or .tar.gz'), { type: 'error' });
    return;
  }
  if (!refs || busy) return;
  busy = true;
  setBusy(true);
  refs.installBtn.setLoading(true);
  try {
    const form = new FormData();
    form.append('file', file);
    const res = await apiFetch('/api/plugins/install', { method: 'POST', body: form });
    toast(t('plugins.installed', { name: res?.data?.installed || 'plugin' },
      `Installed: ${res?.data?.installed || 'plugin'}`));
    notifyPluginsChanged(res?.data?.installed);
  } catch (e) {
    toast(e.message || t('plugins.installFailed', null, 'Install failed'), { type: 'error' });
  } finally {
    busy = false;
    refs.installBtn.setLoading(false);
    setBusy(false);
    await refreshActivity();
  }
}

/* ── Activity log ───────────────────────────────────────────── */

/** Replace the log body with one grey note (loading / empty / unavailable). */
function activityNote(text) {
  refs.activity.textContent = '';
  refs.activity.appendChild(el('div', 'plugins-activity-empty', text));
}

async function refreshActivity() {
  if (!refs.activity) return;
  try {
    const headers = {};
    const token = getToken();
    if (token) headers.Authorization = `Bearer ${token}`;
    const res = await fetch('/api/plugins/install.log', { headers });
    if (!res.ok) {
      activityNote(t('plugins.activityUnavailable', null, 'Activity unavailable.'));
      return;
    }
    const text = (await res.text()).trim();
    if (!text) {
      activityNote(t('plugins.activityEmpty', null, 'No install events yet.'));
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
    activityNote(t('plugins.activityUnavailable', null, 'Activity unavailable.'));
  }
}

/* ── Surface ────────────────────────────────────────────────── */

function mountPlugins() {
  if (tileEl) return tileEl;

  tileEl = el('section', 'tile plugins-tile');
  tileEl.dataset.plugin = PLUGINS_WINDOW;

  const root = el('div', 'plugins-window');

  // Two views share the window: the installed plugins, and the Craft store —
  // the same card grid over the Craft App Suite's web builds.
  const tabs = el('div', 'plugins-tabs');
  tabs.setAttribute('role', 'tablist');
  const tabInstalled = uiChip({
    label: t('plugins.tabInstalled', null, 'Installed'),
    active: tab === 'installed',
    onClick: () => setTab('installed'),
  });
  const tabStore = uiChip({
    label: t('plugins.tabStore', null, 'Craft apps'),
    active: tab === 'store',
    onClick: () => setTab('store'),
  });
  tabInstalled.dataset.tab = 'installed';
  tabStore.dataset.tab = 'store';
  tabs.append(tabInstalled, tabStore);

  const toolbar = el('div', 'plugins-toolbar');
  const search = searchBar({
    placeholder: t('plugins.search', null, 'Search plugins…'),
    onInput: (value) => { query = value; render(); },
  });
  const installBtn = button({
    label: t('plugins.install', null, 'Install'), icon: 'ui/upload', variant: 'primary', size: 'sm',
  });
  on(installBtn, 'click', async () => {
    const [file] = await pickFiles({ accept: '.zip,.tar.gz,.tgz,application/zip,application/gzip' });
    if (file) void uploadArchive(file);
  });
  toolbar.append(search, installBtn);

  // Store-only controls. Auto-update is on by default: an installed app is a
  // static site the store can replace safely.
  const storeActions = el('div', 'plugins-store-actions hidden');
  const checkBtn = button({
    label: t('plugins.storeCheck', null, 'Check for updates'),
    icon: 'ui/refresh', variant: 'ghost', size: 'sm',
  });
  on(checkBtn, 'click', async () => {
    checkBtn.setLoading(true);
    try {
      await refreshStore();
    } catch (e) {
      toast(e.message || t('plugins.storeUnavailable', null, 'Store unavailable'), { type: 'error' });
    } finally {
      checkBtn.setLoading(false);
      if (refs && tab === 'store') render();
    }
  });
  const installAllBtn = button({
    label: t('plugins.storeInstallAll', null, 'Install all'),
    icon: 'ui/download', variant: 'primary', size: 'sm',
  });
  on(installAllBtn, 'click', () => void installAllFromStore());
  const autoLabel = el('span', 'plugins-store-auto-label',
    t('plugins.storeAuto', null, 'Update automatically'));
  const autoToggle = toggle({
    checked: getStoreAutoUpdate(),
    onChange: (value) => setStoreAutoUpdate(value),
  });
  autoToggle.setAttribute('aria-label', t('plugins.storeAuto', null, 'Update automatically'));
  storeActions.append(checkBtn, installAllBtn, autoLabel, autoToggle);

  const metaRow = el('div', 'plugins-categories-row');
  const categories = el('div', 'plugins-categories');
  categories.setAttribute('role', 'toolbar');
  categories.setAttribute('aria-label', t('plugins.categories', null, 'Plugin categories'));
  const meta = el('span', 'plugins-store-meta');
  metaRow.append(categories, meta);

  const grid = el('div', 'plugins-store-grid');
  const empty = el('div', 'plugins-empty hidden');

  const details = el('details', 'plugins-activity-details');
  details.appendChild(el('summary', null, t('plugins.installActivity', null, 'Install activity')));
  const activity = el('div', 'plugins-activity');
  activity.appendChild(
    el('div', 'plugins-activity-empty', t('plugins.activityLoading', null, 'Loading…')),
  );
  details.appendChild(activity);
  on(details, 'toggle', () => { if (details.open) void refreshActivity(); });

  root.append(tabs, toolbar, storeActions, metaRow, grid, empty, details);
  tileEl.appendChild(root);

  refs = {
    grid, empty, meta, categories, categoriesRow: metaRow, activity, installBtn, details,
    tabInstalled, tabStore, storeActions,
  };

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

  on(window, 'plugins:changed', () => void refreshActiveTab());
  cleanups.push(() => {
    if (storePollTimer) clearTimeout(storePollTimer);
    storePollTimer = null;
  });

  renderCategories();
  if (tab === 'store') void setTab('store');
  else void loadPlugins();
  return tileEl;
}

/** Switch between the installed list and the store. */
function setTab(next) {
  tab = next;
  refs?.tabInstalled?.setActive(tab === 'installed');
  refs?.tabStore?.setActive(tab === 'store');
  if (tab === 'store') {
    // Remember who was mid-install so completing one can announce it once.
    storeBusySeen = new Set(
      (storeSnapshot()?.apps || []).filter(craftAppBusy).map((app) => app.id),
    );
    render(); // paint the cached catalog immediately…
    void loadStore()
      .then(() => { if (refs && tab === 'store') render(); })
      .catch(() => { if (refs && tab === 'store') render(); });
  } else {
    void loadPlugins();
  }
}

/** Reload whatever the visible tab shows (a plugin landed or was removed). */
async function refreshActiveTab() {
  if (tab === 'store') {
    try { await loadStore(); } catch (_) { /* the status line reports it */ }
    if (refs && tab === 'store') render();
    return;
  }
  await loadPlugins();
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
    {
      type: 'item',
      label: t('plugins.refresh', null, 'Refresh'),
      icon: 'ui/refresh',
      onClick: () => void refreshActiveTab(),
    },
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
