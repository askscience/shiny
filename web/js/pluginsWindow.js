/**
 * pluginsWindow.js — the Plugins window.
 *
 * In the editorial window language of Settings and the Installer — a
 * numbered rail, a stage with a mono kicker over a large display title, and a
 * live manifest of mono key/value rows on hairlines — with one object of its
 * own: the selected app as a 3D slab in its brand colour, which tilts with the
 * pointer and idles in the user's motion language.
 *
 *  - Rail: 01 Craft · 02 Installed · 03 Updates · 04 Activity, each with a
 *    mono count.
 *  - Stage: an index of apps — mono number, the name in display type, a
 *    muted line, mono metadata and one arrow pill (Get / Update / Open). A
 *    download fills the row's own hairline. The Craft catalog is the core's
 *    (`/api/plugins/store`), which also finds new Craft apps on its own
 *    (`src/plugins/craft_discovery.rs`). The search in the window bar covers
 *    every app and plugin.
 *  - Manifest: the selected app — the slab, its name, publisher and actions,
 *    then VERSION / SIZE / PUBLISHER / STATUS rows, AI control, how its window
 *    opens, links and removal.
 *
 * As the window narrows the manifest becomes a panel over the stage (opened
 * by picking an app, closed with its back button or Escape), then the rail
 * becomes a numbered strip over the stage — a phone reads the same window.
 *
 * Motion follows Settings → Appearance → Motion (`motionStyle.js`): a section
 * change cascades the stage in with `listReveal()`, a new selection brings the
 * slab in with `cardEnter()` and the manifest rows with `listReveal()`, counts
 * roll, and the slab's idle float, tilt easing and row hovers are per language
 * in `css/plugins.css`.
 *
 * Installing/uninstalling is server-wide; turning a plugin on/off is per user.
 */
import { apiFetch, getToken } from './api.js';
import { refreshActivePlugins, refreshPluginCatalog } from './activePlugins.js';
import {
  element as el, chip as uiChip, listReveal, cardEnter, rollNumber, eyebrow,
  toast, button, iconButton, emptyState, searchBar, select, toggle, confirmDialog,
  reducedMotion, paneStack, pullRefresh,
} from '../ui/index.js';
import { pluginIconEl } from './pluginIcon.js';
import { pickFiles } from './files.js';
import { getPluginLayout, setPluginLayout, getStoreAutoUpdate, setStoreAutoUpdate } from './preferences.js';
import {
  loadStore, refreshStore, installCraftApp, setCraftMcp, craftAppBusy, storeSnapshot,
  storeWaitingForNetwork, storeLoadFailed,
} from './craftStore.js';
import { t } from './i18n.js';

export const PLUGINS_WINDOW = 'plugins';

const SECTIONS = ['craft', 'installed', 'updates', 'activity'];

let tileEl = null;
let refs = null;
let cleanups = [];
let plugins = [];
let activeSet = new Set();
let pluginsLoaded = false;
let query = '';
let category = 'all';
let busy = false;
let section = 'craft';
/** The app in the manifest: `{ kind: 'craft', id }` or `{ kind: 'plugin', id }`. */
let selected = null;
let storePollTimer = null;
let storeBusySeen = new Set();
/** Cascade the stage on the next render (a section opened, first load). */
let revealNext = true;
/** The selection the manifest last animated in, so a repaint does not. */
let shownKey = '';
/** Store icons by app id: a promise of an object URL (or null). */
const iconUrls = new Map();

function on(target, event, handler, opts) {
  target?.addEventListener(event, handler, opts);
  cleanups.push(() => target?.removeEventListener(event, handler, opts));
}

const pad = (n) => String(n).padStart(2, '0');

function niceName(p) {
  return p.name.charAt(0).toUpperCase() + p.name.slice(1);
}

function categoryLabel(cat) {
  return cat === 'Other' ? t('plugins.categoryOther', null, 'Other') : cat;
}

function categoryOf(p) {
  return (p.category || '').trim() || 'Other';
}

function notifyPluginsChanged(opened) {
  localStorage.setItem('plugins.changed', String(Date.now()));
  window.dispatchEvent(new CustomEvent('plugins:changed', { detail: opened ? { opened } : undefined }));
}

function formatSize(bytes) {
  if (!bytes) return '';
  const mb = bytes / (1024 * 1024);
  return `${mb.toFixed(mb < 10 ? 1 : 0)} MB`;
}

function storeApps() {
  return storeSnapshot()?.apps || [];
}

function craftApp(id) {
  return storeApps().find((app) => app.id === id) || null;
}

function needsUpdate(app) {
  return !!app.installed_version && (app.update_available || app.mcp_pending);
}

function pendingUpdates() {
  return storeApps().filter((a) => needsUpdate(a) || (a.installed_version && craftAppBusy(a)));
}

/* ── Data ───────────────────────────────────────────────────── */

async function loadPlugins(force = false) {
  try {
    const [list, active] = await Promise.all([
      refreshPluginCatalog({ force }),
      refreshActivePlugins({ force }),
    ]);
    plugins = list || [];
    activeSet = new Set(active || []);
    pluginsLoaded = true;
  } catch (e) {
    plugins = [];
    activeSet = new Set();
    pluginsLoaded = false;
    toast(e.message || t('plugins.loadFailed', null, "Couldn't load plugins"), { type: 'error' });
  }
  if (refs) render();
}

async function loadStoreAndRender() {
  try { await loadStore(); } catch (_) { /* the stage shows the state */ }
  if (refs) render();
}

async function onPluginAction(action, name) {
  if (!refs || busy) return;
  busy = true;
  refs.layout.classList.add('is-busy');
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
    refs?.layout.classList.remove('is-busy');
  }
}

/* ── One app, whatever it is ────────────────────────────────── */

/**
 * A row's worth of facts about a Craft app or a plugin, so the index and the
 * manifest read both the same way.
 */
function describe(kind, id) {
  if (kind === 'craft') {
    const app = craftApp(id);
    if (!app) return null;
    return {
      kind, id, app,
      name: app.name,
      line: app.tagline || '',
      by: app.publisher || 'ArtCraft',
      color: app.color || null,
      version: app.installed_version || app.latest_version || '',
      size: formatSize(app.asset_size),
      installed: !!app.installed_version,
      community: !!app.community,
      fresh: !!app.discovered && !app.installed_version,
    };
  }
  const p = plugins.find((x) => x.name === id);
  if (!p) return null;
  return {
    kind, id, p,
    name: niceName(p),
    line: p.description || p.summary || '',
    by: p.author || "LiteAura",
    color: null,
    version: p.version,
    size: '',
    installed: true,
    active: activeSet.has(p.name),
    category: categoryLabel(categoryOf(p)),
  };
}

/** What the row says on the right: state first, facts otherwise. */
function metaOf(d) {
  if (d.kind === 'craft') {
    const { app } = d;
    if (craftAppBusy(app)) {
      return app.state === 'downloading'
        ? t('plugins.storeDownloading', { percent: app.progress ?? 0 }, `Downloading ${app.progress ?? 0}%`)
        : t('plugins.storeInstallingNow', null, 'Installing…');
    }
    if (app.state === 'failed') return t('plugins.installFailed', null, 'Install failed');
    if (app.waiting_for_network) return t('plugins.storeWaitingForConnection', null, 'Waiting for connection');
    if (needsUpdate(app) && app.latest_version) return `v${app.installed_version} → v${app.latest_version}`;
    return [d.version && `v${d.version}`, d.size].filter(Boolean).join(' · ');
  }
  return [`v${d.version}`, d.category].join(' · ');
}

/* ── Icons ──────────────────────────────────────────────────── */

function storeIconUrl(id) {
  if (!iconUrls.has(id)) {
    const headers = {};
    const token = getToken();
    if (token) headers.Authorization = `Bearer ${token}`;
    iconUrls.set(id, fetch(`/api/plugins/store/icon?id=${encodeURIComponent(id)}`, { headers })
      .then((res) => (res.ok ? res.blob() : null))
      .then((blob) => (blob ? URL.createObjectURL(blob) : null))
      .catch(() => null));
  }
  return iconUrls.get(id);
}

/** The picture on an icon face: the Craft app's own icon over its initial, or
 *  a plugin's monochrome icon. */
function iconFace(d, size) {
  const face = el('span', 'pw-face');
  if (d.kind !== 'craft') {
    face.appendChild(pluginIconEl(d.id, { size: Math.round(size * 0.44), fallback: 'ui/puzzle' }));
    return face;
  }
  face.appendChild(el('span', 'pw-face-letter', d.name.charAt(0)));
  void storeIconUrl(d.id).then((url) => {
    if (!url) return;
    const img = document.createElement('img');
    img.alt = '';
    img.decoding = 'async';
    img.src = url;
    img.addEventListener('load', () => face.classList.add('has-image'), { once: true });
    face.appendChild(img);
  });
  return face;
}

/** The flat icon of an index row. */
function rowIcon(d) {
  const tile = el('span', `pw-icon${d.kind === 'craft' ? ' is-craft' : ''}`);
  if (d.color) tile.style.setProperty('--app-color', d.color);
  tile.appendChild(iconFace(d, 40));
  return tile;
}

/**
 * The manifest's object: the app's icon as a slab with depth — a stack of
 * layers behind the face in a darker brand colour, a sheen that follows the
 * pointer and a soft shadow under it. Pointer and finger tilt it.
 */
function slab(d) {
  const wrap = el('div', `pw-object${d.kind === 'craft' ? ' is-craft' : ''}`);
  if (d.color) wrap.style.setProperty('--app-color', d.color);
  wrap.setAttribute('aria-hidden', 'true');
  const float = el('div', 'pw-object-float');
  const body = el('div', 'pw-object-body');
  for (let i = 8; i >= 1; i -= 1) {
    const layer = el('span', 'pw-object-layer');
    layer.style.setProperty('--z', String(i));
    body.appendChild(layer);
  }
  body.appendChild(iconFace(d, 168));
  body.appendChild(el('span', 'pw-object-sheen'));
  float.appendChild(body);
  wrap.append(el('span', 'pw-object-shadow'), float);

  const tilt = (x, y) => {
    wrap.style.setProperty('--rx', `${(-y * 22).toFixed(2)}deg`);
    wrap.style.setProperty('--ry', `${(x * 26).toFixed(2)}deg`);
    wrap.style.setProperty('--mx', `${(50 + x * 50).toFixed(1)}%`);
    wrap.style.setProperty('--my', `${(50 + y * 50).toFixed(1)}%`);
  };
  const rest = () => {
    wrap.classList.remove('is-tilting');
    ['--rx', '--ry', '--mx', '--my'].forEach((v) => wrap.style.removeProperty(v));
  };
  wrap.addEventListener('pointermove', (e) => {
    if (reducedMotion()) return;
    const r = wrap.getBoundingClientRect();
    const x = Math.max(-1, Math.min(1, ((e.clientX - r.left) / r.width) * 2 - 1));
    const y = Math.max(-1, Math.min(1, ((e.clientY - r.top) / r.height) * 2 - 1));
    wrap.classList.add('is-tilting');
    tilt(x, y);
  });
  wrap.addEventListener('pointerleave', rest);
  wrap.addEventListener('pointercancel', rest);
  wrap.addEventListener('pointerup', (e) => { if (e.pointerType !== 'mouse') rest(); });
  return wrap;
}

/* ── Actions ────────────────────────────────────────────────── */

function arrowPill(label, onClick, { primary = false, disabled = false, size = 'sm' } = {}) {
  return button({
    label,
    variant: primary ? 'primary' : 'ghost',
    size,
    arrow: true,
    disabled,
    className: 'pw-pill',
    onClick: (e) => { e.stopPropagation(); onClick(); },
  });
}

/** The one action an app offers now; null while it is busy (the row says so). */
function actionOf(d, opts = {}) {
  if (d.kind === 'craft') {
    const { app } = d;
    if (craftAppBusy(app)) return null;
    const waiting = app.waiting_for_network || (storeSnapshot()?.refreshing && !app.latest_version);
    if (!app.installed_version) {
      return arrowPill(t('plugins.get', null, 'Get'), () => void installFromStore(app.id, app.name),
        { primary: true, disabled: waiting || !app.latest_version, ...opts });
    }
    if (needsUpdate(app)) {
      return arrowPill(t('plugins.storeUpdate', null, 'Update'), () => void installFromStore(app.id, app.name), { primary: true, ...opts });
    }
    return arrowPill(t('plugins.open', null, 'Open'), () => void openCraftApp(app), opts);
  }
  const { p } = d;
  if (d.active && (p.surface || p.web_app)) {
    return arrowPill(t('plugins.open', null, 'Open'),
      () => window.dispatchEvent(new CustomEvent('plugin:focus', { detail: { name: p.name } })), opts);
  }
  if (!d.active) return arrowPill(t('ctx.activate', null, 'Turn on'), () => void onPluginAction('activate', p.name), { primary: true, ...opts });
  return null;
}

async function openCraftApp(app) {
  if (!activeSet.has(app.id)) await onPluginAction('activate', app.id);
  window.dispatchEvent(new CustomEvent('plugin:focus', { detail: { name: app.id } }));
}

async function installFromStore(id, name) {
  try {
    await installCraftApp(id);
    toast(t('plugins.storeInstallingToast', { name }, `Installing ${name}…`));
    await loadStoreAndRender();
  } catch (e) {
    toast(e.message || t('plugins.installFailed', null, 'Install failed'), { type: 'error' });
  }
}

async function installAllFromStore({ updatesOnly = false } = {}) {
  const catalogRefreshing = storeSnapshot()?.refreshing === true;
  const apps = storeApps()
    .filter((app) => !craftAppBusy(app) && !app.waiting_for_network
      && (!catalogRefreshing || app.latest_version)
      && (updatesOnly ? needsUpdate(app) : (!app.installed_version || needsUpdate(app))));
  if (!apps.length) {
    toast(storeWaitingForNetwork()
      ? t('plugins.storeWaitingForConnection', null, 'Waiting for connection')
      : t('plugins.storeAllCurrent', null, 'Every Craft app is up to date'));
    return;
  }
  const cliBytes = (app) => (app.mcp_available && (!app.mcp_installed || app.mcp_pending) ? (app.mcp_size || 0) : 0);
  const bytes = apps.reduce((sum, app) => sum + (app.asset_size || 0) + cliBytes(app), 0);
  const size = bytes ? ` (~${formatSize(bytes)})` : '';
  if (!await confirmDialog(t('plugins.storeInstallAllConfirm', { count: apps.length, size },
    `Install ${apps.length} apps${size}?`))) return;
  for (const app of apps) {
    try {
      await installCraftApp(app.id);
    } catch (e) {
      toast(e.message || t('plugins.installFailed', null, 'Install failed'), { type: 'error' });
    }
  }
  toast(t('plugins.storeQueued', { count: apps.length }, `Installing ${apps.length} apps`));
  await loadStoreAndRender();
}

async function setCraftAiControl(app, enable) {
  if (enable && app.installed_version) {
    const size = app.mcp_size ? ` (~${formatSize(app.mcp_size)})` : '';
    const confirmed = await confirmDialog(t('plugins.storeAiControlConfirm', { name: app.name, size },
      `Give the AI control of ${app.name}${size}? It can drive the app while its window is open; focus mode narrows it to the apps you focus.`));
    if (!confirmed) { render(); return; }
  }
  try {
    await setCraftMcp(app.id, enable);
    toast(enable
      ? (app.installed_version
        ? t('plugins.storeAiControlOn', { name: app.name }, `${app.name}: AI control is ready`)
        : t('plugins.storeAiControlWithApp', { name: app.name }, `${app.name}: AI control installs with the app`))
      : t('plugins.storeAiControlOff', { name: app.name }, `${app.name}: AI control removed`));
  } catch (e) {
    toast(e.message || t('plugins.installFailed', null, 'Install failed'), { type: 'error' });
  }
  await loadStoreAndRender();
}

/* ── Rail ───────────────────────────────────────────────────── */

function sectionLabel(id) {
  return {
    craft: t('plugins.tabStore', null, 'Craft apps'),
    installed: t('plugins.tabInstalled', null, 'Installed'),
    updates: t('plugins.tabUpdates', null, 'Updates'),
    activity: t('plugins.installActivity', null, 'Install activity'),
  }[id];
}

function sectionCount(id) {
  if (id === 'craft') return storeApps().length || null;
  if (id === 'installed') return plugins.length || null;
  if (id === 'updates') return pendingUpdates().length || null;
  return null;
}

function renderRail() {
  for (const item of refs.railItems) {
    const id = item.dataset.section;
    const active = id === section && !query.trim();
    item.classList.toggle('is-active', active);
    item.setAttribute('aria-current', active ? 'page' : 'false');
    const count = sectionCount(id);
    const text = count === null ? '' : pad(count);
    if (item.count.textContent !== text) {
      item.count.textContent = text;
      if (text) rollNumber(item.count, { duration: 600 });
    }
    item.classList.toggle('has-news', id === 'updates' && !!count);
  }
}

/* ── Stage ──────────────────────────────────────────────────── */

function stageHead(kicker, title, subtitle) {
  const head = el('header', 'pw-stage-head');
  head.append(eyebrow(kicker, 'pw-kicker'), el('h2', 'pw-title', title));
  if (subtitle) head.appendChild(el('p', 'pw-subtitle', subtitle));
  return head;
}

function indexRow(d, number) {
  const r = el('div', 'pw-row');
  r.dataset.key = `${d.kind}:${d.id}`;
  r.tabIndex = 0;
  r.setAttribute('role', 'button');
  r.setAttribute('aria-label', d.name);
  const key = `${d.kind}:${d.id}`;
  const isSel = selected && `${selected.kind}:${selected.id}` === key;
  r.classList.toggle('is-selected', !!isSel);
  r.classList.toggle('is-dim', d.kind === 'plugin' && !d.active);
  if (d.kind === 'craft' && craftAppBusy(d.app)) {
    r.classList.add('is-busy');
    r.style.setProperty('--p', String(d.app.state === 'downloading' ? (d.app.progress ?? 0) : 100));
  }
  if (d.kind === 'craft' && d.app.state === 'failed') r.classList.add('is-failed');

  const num = el('span', 'pw-row-num', pad(number));
  const text = el('div', 'pw-row-text');
  const name = el('span', 'pw-row-name', d.name);
  if (d.fresh) name.appendChild(el('span', 'pw-row-new', t('plugins.storeNew', null, 'New')));
  if (d.community) name.appendChild(el('span', 'pw-row-new', t('plugins.community', null, 'Community')));
  text.append(name);
  if (d.line) text.appendChild(el('span', 'pw-row-line', d.line));
  const meta = el('span', 'pw-row-meta', metaOf(d));
  const action = actionOf(d) || el('span', 'pw-pill-spacer');
  r.append(num, rowIcon(d), text, meta, action);
  const pick = () => select_({ kind: d.kind, id: d.id }, { open: true });
  r.addEventListener('click', pick);
  r.addEventListener('keydown', (e) => {
    if (e.target === r && (e.key === 'Enter' || e.key === ' ')) { e.preventDefault(); pick(); }
  });
  return r;
}

function indexList(items) {
  const list = el('div', 'pw-index');
  list.setAttribute('role', 'list');
  items.forEach((d, i) => list.appendChild(indexRow(d, i + 1)));
  return list;
}

function stageEmpty(o) {
  const box = el('div', 'pw-empty');
  box.appendChild(emptyState(o));
  return box;
}

function storeMissing() {
  const waiting = storeWaitingForNetwork();
  const failed = storeLoadFailed();
  return stageEmpty({
    icon: waiting ? 'ui/network' : (failed ? 'ui/warning' : 'ui/download'),
    title: waiting
      ? t('plugins.storeWaitingForConnection', null, 'Waiting for connection')
      : failed ? t('plugins.storeUnavailable', null, 'Store unavailable') : t('plugins.storeConnecting', null, 'Connecting…'),
    body: !failed || waiting
      ? t('plugins.storeConnectingBody', null, 'The app catalog will load on its own as soon as the network is ready.')
      : t('plugins.storeUnavailableBody', null, 'The catalog could not be loaded. Check the connection and try again.'),
  });
}

function kicker(id) {
  return `${pad(SECTIONS.indexOf(id) + 1)} / ${pad(SECTIONS.length)}`;
}

function renderStage() {
  const stage = refs.stageBody;
  stage.textContent = '';
  const q = query.trim().toLowerCase();
  if (q) {
    const match = (...f) => f.filter(Boolean).some((s) => String(s).toLowerCase().includes(q));
    const craftIds = new Set(storeApps().map((a) => a.id));
    const items = [
      ...storeApps().filter((a) => match(a.name, a.id, a.tagline, a.publisher)).map((a) => describe('craft', a.id)),
      ...plugins.filter((p) => !craftIds.has(p.name) && match(p.name, p.description, p.summary, categoryOf(p)))
        .map((p) => describe('plugin', p.name)),
    ].filter(Boolean);
    stage.appendChild(stageHead(t('plugins.searchResults', null, 'Results'), `“${query.trim()}”`,
      t('plugins.searchCount', { count: items.length }, `${items.length} found`)));
    stage.appendChild(items.length ? indexList(items) : stageEmpty({
      icon: 'ui/search',
      title: t('plugins.noMatch', null, 'No matching plugins'),
      body: t('plugins.noMatchBody', null, 'Try a different search or category.'),
    }));
    return;
  }

  if (section === 'craft') {
    const data = storeSnapshot();
    stage.appendChild(stageHead(kicker('craft'), t('plugins.tabStore', null, 'Craft apps'),
      t('plugins.subtitleCraft', null, 'The Craft suite, each app in a window of its own — installed in one step, kept up to date, and open to the assistant if you want it.')));
    if (!data) { stage.appendChild(storeMissing()); return; }
    const apps = data.apps || [];
    const missing = apps.filter((a) => !a.installed_version && !craftAppBusy(a) && a.latest_version);
    const facts = el('div', 'pw-facts');
    const fact = (n, label) => {
      const f = el('div', 'pw-fact');
      const num = el('span', 'pw-fact-num', pad(n));
      f.append(num, eyebrow(label));
      facts.appendChild(f);
      if (revealNext) rollNumber(num, { duration: 900 });
    };
    fact(apps.length, t('plugins.factApps', null, 'Apps'));
    fact(apps.filter((a) => a.installed_version).length, t('plugins.factInstalled', null, 'Installed'));
    const fresh = apps.filter((a) => a.discovered).length;
    if (fresh) fact(fresh, t('plugins.factNew', null, 'New'));
    if (missing.length > 1) {
      const all = arrowPill(t('plugins.getAll', null, 'Get all'), () => void installAllFromStore());
      all.classList.add('pw-facts-action');
      facts.appendChild(all);
    }
    stage.appendChild(facts);
    // New apps first, then the suite in its own order.
    const ordered = [...apps.filter((a) => a.discovered), ...apps.filter((a) => !a.discovered)];
    stage.appendChild(indexList(ordered.map((a) => describe('craft', a.id)).filter(Boolean)));
    return;
  }

  if (section === 'installed') {
    stage.appendChild(stageHead(kicker('installed'), t('plugins.tabInstalled', null, 'Installed'),
      t('plugins.metaInstalled', { active: activeSet.size, total: plugins.length },
        `${plugins.length} installed · ${activeSet.size} active`)));
    if (!pluginsLoaded) return;
    if (!plugins.length) {
      stage.appendChild(stageEmpty({
        icon: 'ui/puzzle',
        title: t('plugins.none', null, 'No plugins installed'),
        body: t('plugins.noneBody', null, "LiteAura is running in its bare form. Install a plugin to add apps and new skills for the assistant."),
      }));
      return;
    }
    const cats = ['all', ...new Set(plugins.map(categoryOf).sort((a, b) => a.localeCompare(b)))];
    const chips = el('div', 'pw-chips');
    chips.setAttribute('role', 'toolbar');
    chips.setAttribute('aria-label', t('plugins.categories', null, 'Plugin categories'));
    for (const cat of cats) {
      chips.appendChild(uiChip({
        label: cat === 'all' ? t('plugins.categoryAll', null, 'All') : categoryLabel(cat),
        active: cat === category,
        onClick: () => { category = cat; revealNext = true; render(); },
      }));
    }
    stage.appendChild(chips);
    const list = plugins.filter((p) => category === 'all' || categoryOf(p) === category)
      .map((p) => (craftApp(p.name) ? describe('craft', p.name) : describe('plugin', p.name)))
      .filter(Boolean);
    stage.appendChild(list.length ? indexList(list)
      : stageEmpty({ icon: 'ui/search', title: t('plugins.noMatch', null, 'No matching plugins') }));
    return;
  }

  if (section === 'updates') {
    const pending = pendingUpdates();
    stage.appendChild(stageHead(kicker('updates'), t('plugins.tabUpdates', null, 'Updates'),
      pending.length
        ? t('plugins.metaUpdates', { count: pending.length }, `${pending.length} to update`)
        : t('plugins.allUpToDate', null, 'Everything is up to date')));
    const controls = el('div', 'pw-controls');
    if (pending.some((a) => !craftAppBusy(a))) {
      controls.appendChild(arrowPill(t('plugins.updateAll', null, 'Update all'),
        () => void installAllFromStore({ updatesOnly: true }), { primary: true }));
    }
    const check = button({ label: t('plugins.storeCheck', null, 'Check for updates'), icon: 'ui/refresh', variant: 'ghost', size: 'sm' });
    check.addEventListener('click', async () => {
      check.setLoading(true);
      try { await refreshStore(); } catch (e) {
        toast(storeWaitingForNetwork()
          ? t('plugins.storeWaitingForConnection', null, 'Waiting for connection')
          : (e.message || t('plugins.storeUnavailable', null, 'Store unavailable')), { type: 'error' });
      } finally {
        check.setLoading(false);
        render();
      }
    });
    controls.appendChild(check);
    const auto = el('label', 'pw-auto');
    auto.appendChild(eyebrow(t('plugins.storeAuto', null, 'Update automatically')));
    const autoToggle = toggle({ checked: getStoreAutoUpdate(), onChange: (v) => setStoreAutoUpdate(v) });
    autoToggle.setAttribute('aria-label', t('plugins.storeAuto', null, 'Update automatically'));
    auto.appendChild(autoToggle);
    controls.appendChild(auto);
    stage.appendChild(controls);
    if (!storeSnapshot()) { stage.appendChild(storeMissing()); return; }
    stage.appendChild(pending.length ? indexList(pending.map((a) => describe('craft', a.id)).filter(Boolean)) : stageEmpty({
      icon: 'ui/check',
      title: t('plugins.allUpToDate', null, 'Everything is up to date'),
      body: getStoreAutoUpdate()
        ? t('plugins.allUpToDateAuto', null, 'New versions install on their own.')
        : t('plugins.allUpToDateManual', null, 'New versions will show here.'),
    }));
    return;
  }

  // Activity: the install log as mono lines on hairlines.
  stage.appendChild(stageHead(kicker('activity'), t('plugins.installActivity', null, 'Install activity'),
    t('plugins.subtitleActivity', null, 'Every install, update and removal, newest last.')));
  const log = el('div', 'pw-log');
  log.appendChild(el('div', 'pw-log-note', t('plugins.activityLoading', null, 'Loading…')));
  stage.appendChild(log);
  void refreshActivity(log);
}

/* ── Manifest ───────────────────────────────────────────────── */

function kv(label, value) {
  if (value === null || value === undefined || value === '') return null;
  const r = el('div', 'pw-kv');
  r.append(el('span', 'pw-kv-key', label), el('span', 'pw-kv-val', String(value)));
  return r;
}

function settingRow(label, control, hint = '') {
  const r = el('div', 'pw-setting');
  const text = el('div', 'pw-setting-text');
  text.appendChild(el('span', 'pw-setting-label', label));
  if (hint) text.appendChild(el('span', 'pw-setting-hint', hint));
  r.append(text, control);
  return r;
}

function windowMode(id, label) {
  const mode = select({
    options: [
      { value: 'tile', label: t('plugins.layoutTile', null, 'Tile') },
      { value: 'full', label: t('ctx.fullscreen', null, 'Full screen') },
    ],
    value: getPluginLayout(id),
    onChange: (value) => {
      setPluginLayout(id, value);
      toast(value === 'full'
        ? t('plugins.opensAsFull', { name: label }, `${label}: opens full screen`)
        : t('plugins.opensAsTile', { name: label }, `${label}: opens as a tile`), { type: 'info' });
    },
  });
  mode.querySelector('select')?.setAttribute('aria-label', t('plugins.windowMode', null, 'Opens as'));
  return settingRow(t('plugins.windowMode', null, 'Opens as'), mode);
}

function removeControl(name, onConfirm) {
  const wrap = el('div', 'pw-remove');
  const ask = button({ label: t('plugins.remove', { name }, `Remove ${name}`), icon: 'ui/trash', variant: 'ghost', size: 'sm' });
  const confirmRow = el('div', 'pw-remove-confirm hidden');
  confirmRow.append(
    el('span', 'pw-remove-text', t('plugins.uninstallConfirm', { name }, `Remove plugin “${name}”? Its saved data will be kept.`)),
    button({
      label: t('common.cancel', null, 'Cancel'), variant: 'ghost', size: 'sm',
      onClick: () => { confirmRow.classList.add('hidden'); ask.classList.remove('hidden'); },
    }),
    button({ label: t('plugins.removeShort', null, 'Remove'), variant: 'danger', size: 'sm', onClick: onConfirm }),
  );
  ask.addEventListener('click', () => { ask.classList.add('hidden'); confirmRow.classList.remove('hidden'); });
  wrap.append(ask, confirmRow);
  return wrap;
}

function link(label, href) {
  const a = document.createElement('a');
  a.className = 'pw-link';
  a.href = href;
  a.target = '_blank';
  a.rel = 'noopener noreferrer';
  a.append(el('span', '', label), el('span', 'pw-link-arrow', '↗'));
  return a;
}

/** The selection, or the first thing worth showing in this section. */
function currentSelection() {
  if (selected && describe(selected.kind, selected.id)) return selected;
  if (section === 'installed') {
    const first = plugins.filter((p) => category === 'all' || categoryOf(p) === category)[0];
    return first ? { kind: craftApp(first.name) ? 'craft' : 'plugin', id: first.name } : null;
  }
  if (section === 'updates') {
    const first = pendingUpdates()[0];
    return first ? { kind: 'craft', id: first.id } : null;
  }
  const apps = storeApps();
  const first = apps.find((a) => a.discovered && !a.installed_version)
    || apps.find((a) => !a.installed_version) || apps[0];
  return first ? { kind: 'craft', id: first.id } : null;
}

function renderManifest() {
  const box = refs.manifestBody;
  box.textContent = '';
  const sel = currentSelection();
  const d = sel && describe(sel.kind, sel.id);
  if (!d) {
    box.appendChild(el('p', 'pw-manifest-note', t('plugins.selectApp', null, 'Pick an app to see it here.')));
    shownKey = '';
    return;
  }
  const key = `${d.kind}:${d.id}`;
  refs.manifestName.textContent = d.name;

  const object = slab(d);
  const head = el('div', 'pw-manifest-head');
  head.append(
    eyebrow(d.kind === 'craft' ? (d.fresh ? t('plugins.featuredNew', null, 'New in the store') : 'Craft') : d.category || '', 'pw-kicker'),
    el('h3', 'pw-manifest-title', d.name),
  );
  if (d.line) head.appendChild(el('p', 'pw-manifest-line', d.line));
  const actions = el('div', 'pw-manifest-actions');
  const action = actionOf(d, { size: 'md' });
  if (action) actions.appendChild(action);
  if (d.kind === 'craft' && craftAppBusy(d.app)) {
    const bar = el('div', 'pw-progress');
    bar.style.setProperty('--p', String(d.app.state === 'downloading' ? (d.app.progress ?? 0) : 100));
    bar.appendChild(el('span', 'pw-progress-text', metaOf(d)));
    actions.appendChild(bar);
  }
  head.appendChild(actions);

  const rows = el('div', 'pw-kvs');
  const status = d.kind === 'craft'
    ? (craftAppBusy(d.app) ? metaOf(d)
      : d.app.state === 'failed' ? (d.app.error || t('plugins.installFailed', null, 'Install failed'))
        : needsUpdate(d.app) ? t('plugins.storeUpdateReady', null, 'Update available')
          : d.installed ? t('plugins.storeUpToDate', null, 'Up to date')
            : t('plugins.statusNotInstalled', null, 'Not installed'))
    : (d.active ? t('plugins.activeBadge', null, 'Active') : t('plugins.statusOff', null, 'Off'));
  for (const r of [
    kv(t('plugins.infoVersion', null, 'Version'), d.version && `v${d.version}`),
    d.kind === 'craft' && needsUpdate(d.app) ? kv(t('plugins.infoLatest', null, 'Latest'), `v${d.app.latest_version}`) : null,
    kv(t('plugins.infoSize', null, 'Size'), d.size),
    kv(t('plugins.infoPublisher', null, 'Publisher'), d.by + (d.community ? ` · ${t('plugins.community', null, 'Community')}` : '')),
    d.kind === 'plugin' ? kv(t('plugins.infoApi', null, 'API'), d.p.api_level) : null,
    kv(t('plugins.infoStatus', null, 'Status'), status),
  ]) if (r) rows.appendChild(r);

  const settings = el('div', 'pw-settings');
  if (d.kind === 'craft' && d.app.mcp_available) {
    const app = d.app;
    const ai = toggle({ checked: app.mcp_enabled !== false, onChange: (v) => void setCraftAiControl(app, v) });
    ai.setAttribute('aria-label', t('plugins.storeAiControlName', { name: app.name }, `AI control for ${app.name}`));
    ai.disabled = craftAppBusy(app);
    settings.appendChild(settingRow(t('plugins.storeAiControl', null, 'AI control'), ai,
      app.mcp_installed && app.mcp_version
        ? t('plugins.storeAiControlVersion', { version: app.mcp_version }, `AI control v${app.mcp_version}`)
        : t('plugins.storeAiControlIncluded', null, 'Installs with the app')));
  }
  if (d.kind === 'plugin') {
    const sw = toggle({ checked: d.active, onChange: (v) => void onPluginAction(v ? 'activate' : 'deactivate', d.id) });
    sw.setAttribute('aria-label', t('plugins.turnedOn', null, 'Turned on'));
    settings.appendChild(settingRow(t('plugins.turnedOn', null, 'Turned on'), sw));
  }
  const hasWindow = d.kind === 'craft' ? d.installed : (d.p.surface || d.p.web_app) && d.id !== 'keyboard';
  if (hasWindow && activeSet.has(d.id)) settings.appendChild(windowMode(d.id, d.name));

  const links = el('div', 'pw-links');
  if (d.kind === 'craft') {
    if (d.app.website) links.appendChild(link(t('plugins.website', null, 'Website'), d.app.website));
    if (d.app.repo) links.appendChild(link(t('plugins.source', null, 'Source code'), d.app.repo));
  }

  box.append(object, head, rows);
  if (settings.childElementCount) box.appendChild(settings);
  if (links.childElementCount) box.appendChild(links);
  if (d.installed && !(d.kind === 'craft' && craftAppBusy(d.app))) {
    box.appendChild(removeControl(d.name, () => {
      selected = null;
      void onPluginAction('uninstall', d.id);
    }));
  }

  // A new selection arrives in the motion language; a repaint stays still.
  if (key !== shownKey) {
    shownKey = key;
    void cardEnter(object);
    void listReveal([head, ...box.children].filter((n) => n !== object));
  }
}

/** Show an app in the manifest; on a narrow window, open it as a pane
 *  over the stage (the shared pane stack, ui/components/panes.js). */
function select_(next, { open = false } = {}) {
  selected = next;
  render();
  if (open) refs.nav?.show('manifest');
}

function closeManifest() {
  return !!refs?.nav?.reset();
}

/* ── Render ─────────────────────────────────────────────────── */

/**
 * What the page shows, minus the download percentages: when only those
 * moved (the store is polled while something downloads), the rows and the
 * manifest are patched in place instead of rebuilt — the slab keeps its
 * tilt, hovers stay, nothing re-animates.
 */
function signature() {
  return JSON.stringify([
    section, query, category, selected,
    storeSnapshot() ? storeApps().map((a) => [a.id, a.state, a.installed_version, a.latest_version,
      a.update_available, a.mcp_pending, a.mcp_enabled, a.mcp_installed, a.waiting_for_network, a.error])
      : storeLoadFailed(),
    pluginsLoaded, plugins.map((p) => `${p.name}@${p.version}`), [...activeSet].sort(),
    refs?.nav?.top() || '',
  ]);
}
let lastSignature = '';

/** Patch the download progress where it is shown. */
function patchProgress() {
  for (const app of storeApps().filter(craftAppBusy)) {
    const pct = String(app.state === 'downloading' ? (app.progress ?? 0) : 100);
    const d = describe('craft', app.id);
    refs.root.querySelectorAll(`.pw-row[data-key="craft:${CSS.escape(app.id)}"]`).forEach((r) => {
      r.style.setProperty('--p', pct);
      const meta = r.querySelector('.pw-row-meta');
      if (meta && d) meta.textContent = metaOf(d);
    });
    if (shownKey === `craft:${app.id}`) {
      const bar = refs.manifestBody.querySelector('.pw-progress');
      bar?.style.setProperty('--p', pct);
      const text = bar?.querySelector('.pw-progress-text');
      if (text && d) text.textContent = metaOf(d);
    }
  }
}

function render() {
  if (!refs) return;
  const sig = signature();
  if (sig === lastSignature) {
    patchProgress();
    scheduleStorePoll();
    return;
  }
  lastSignature = sig;
  renderRail();
  renderStage();
  renderManifest();
  const pending = pendingUpdates().length;
  refs.status.textContent = storeWaitingForNetwork()
    ? t('plugins.storeWaitingForConnection', null, 'Waiting for connection')
    : pending
      ? t('plugins.metaUpdates', { count: pending }, `${pending} to update`)
      : t('plugins.allUpToDate', null, 'Everything is up to date');
  refs.statusDot.classList.toggle('is-news', !!pending);
  refs.installBtn.classList.toggle('hidden', section !== 'installed');
  if (revealNext) {
    revealNext = false;
    void listReveal([...refs.stageBody.querySelectorAll('.pw-stage-head > *, .pw-facts, .pw-chips, .pw-controls, .pw-row')]);
  }
  reportFinishedInstalls(storeApps());
  scheduleStorePoll();
}

function reportFinishedInstalls(apps) {
  const busyNow = new Set(apps.filter(craftAppBusy).map((app) => app.id));
  const finished = [...storeBusySeen].filter((id) => !busyNow.has(id));
  storeBusySeen = busyNow;
  if (finished.some((id) => apps.find((app) => app.id === id)?.installed_version)) notifyPluginsChanged();
}

function scheduleStorePoll() {
  if (storePollTimer) {
    clearTimeout(storePollTimer);
    storePollTimer = null;
  }
  if (!refs) return;
  if (!(storeApps().some(craftAppBusy) || storeSnapshot()?.refreshing)) return;
  storePollTimer = setTimeout(() => {
    storePollTimer = null;
    void loadStoreAndRender();
  }, 900);
}

/* ── Install from a file, activity ──────────────────────────── */

async function uploadArchive(file) {
  if (busy || !file || !refs) return;
  if (!/\.(zip|tar\.gz|tgz)$/i.test(file.name)) {
    toast(t('plugins.badArchive', null, 'File must be .zip or .tar.gz'), { type: 'error' });
    return;
  }
  busy = true;
  try {
    const form = new FormData();
    form.append('file', file);
    const res = await apiFetch('/api/plugins/install', { method: 'POST', body: form });
    toast(t('plugins.installed', { name: res?.data?.installed || 'plugin' }, `Installed: ${res?.data?.installed || 'plugin'}`));
    notifyPluginsChanged(res?.data?.installed);
  } catch (e) {
    toast(e.message || t('plugins.installFailed', null, 'Install failed'), { type: 'error' });
  } finally {
    busy = false;
  }
}

async function refreshActivity(log) {
  const note = (text) => { log.textContent = ''; log.appendChild(el('div', 'pw-log-note', text)); };
  try {
    const headers = {};
    const token = getToken();
    if (token) headers.Authorization = `Bearer ${token}`;
    const res = await fetch('/api/plugins/install.log', { headers });
    if (!res.ok) { note(t('plugins.activityUnavailable', null, 'Activity unavailable.')); return; }
    const text = (await res.text()).trim();
    if (!text) { note(t('plugins.activityEmpty', null, 'No install events yet.')); return; }
    log.textContent = '';
    for (const line of text.split('\n').slice(-80)) {
      let cls = 'ok';
      if (/reject|failed|missing/i.test(line)) cls = 'error';
      else if (/deactivate|uninstall/i.test(line)) cls = 'warn';
      log.appendChild(el('div', `pw-log-line is-${cls}`, line));
    }
  } catch (_) {
    note(t('plugins.activityUnavailable', null, 'Activity unavailable.'));
  }
}

/* ── Surface ────────────────────────────────────────────────── */

function setSection(next) {
  if (!SECTIONS.includes(next)) return;
  if (next !== section || query.trim()) revealNext = true;
  section = next;
  selected = null;
  if (query.trim()) {
    query = '';
    const input = refs.search.querySelector('input');
    if (input) input.value = '';
  }
  closeManifest();
  refs.stage.scrollTop = 0;
  render();
  if (next === 'installed') void loadPlugins();
  else if (next !== 'activity') void loadStoreAndRender();
}

function mountPlugins() {
  if (tileEl) return tileEl;

  tileEl = el('section', 'tile plugins-tile');
  tileEl.dataset.plugin = PLUGINS_WINDOW;

  // The window bar (merged into the core title row): a mono window name, the
  // search, install-from-a-file and one mono status — Settings' grammar.
  const bar = el('div', 'pw-chrome');
  bar.dataset.windowBar = '';
  const barName = eyebrow(t('hud.plugins', null, 'Plugins'), 'pw-chrome-title');
  const spacer = el('span', 'pw-chrome-spacer');
  const search = searchBar({
    placeholder: t('plugins.searchStore', null, 'Search apps and plugins'),
    onInput: (value) => { query = value; revealNext = true; selected = null; render(); },
  });
  search.classList.add('pw-search');
  const installBtn = iconButton({ icon: 'ui/upload', variant: 'ghost', size: 'sm', label: t('plugins.installFromFile', null, 'Install from a file') });
  on(installBtn, 'click', async () => {
    const [file] = await pickFiles({ accept: '.zip,.tar.gz,.tgz,application/zip,application/gzip' });
    if (file) void uploadArchive(file);
  });
  const statusWrap = el('span', 'pw-chrome-status');
  const statusDot = el('span', 'pw-status-dot');
  const status = eyebrow('', 'pw-status-text');
  statusWrap.append(statusDot, status);
  bar.append(barName, spacer, search, installBtn, statusWrap);

  // Rail: the numbered sections.
  const rail = el('nav', 'pw-rail');
  rail.setAttribute('aria-label', t('hud.plugins', null, 'Plugins'));
  const railHead = el('div', 'pw-rail-head');
  railHead.append(eyebrow("LiteAura"), el('h2', 'pw-rail-title', t('hud.plugins', null, 'Plugins')));
  const railList = el('div', 'pw-rail-list');
  const railItems = SECTIONS.map((id, i) => {
    const item = document.createElement('button');
    item.type = 'button';
    item.className = 'pw-rail-item';
    item.dataset.section = id;
    item.append(el('span', 'pw-rail-num', pad(i + 1)), el('span', 'pw-rail-label', sectionLabel(id)));
    const count = el('span', 'pw-rail-count');
    item.appendChild(count);
    item.count = count;
    item.addEventListener('click', () => setSection(id));
    railList.appendChild(item);
    return item;
  });
  rail.append(railHead, railList);

  // Stage.
  const stage = el('main', 'pw-stage');
  const stageBody = el('div', 'pw-stage-body');
  stage.appendChild(stageBody);

  // Manifest.
  const manifest = el('aside', 'pw-manifest');
  manifest.setAttribute('aria-label', t('plugins.atAGlance', null, 'At a glance'));
  // Its top row is the pane bar: while the manifest is a pane over the
  // stage, the shared Back lands at its start.
  const manifestTop = el('div', 'pw-manifest-top');
  manifestTop.dataset.paneBar = '';
  const manifestName = el('span', 'pw-manifest-name');
  manifestTop.append(eyebrow(t('plugins.atAGlance', null, 'At a glance'), 'pw-manifest-eyebrow'), manifestName);
  const manifestBody = el('div', 'pw-manifest-body');
  manifest.append(manifestTop, manifestBody);

  const layout = el('div', 'pw-layout');
  layout.append(rail, stage, manifest);
  const root = el('div', 'pw-window');
  root.appendChild(layout);
  tileEl.append(bar, root);

  // Rail · stage · manifest side by side while wide; under 1080px (or on a
  // phone) the manifest is a pane that slides over the stage when an app is
  // picked and closes with Back / Escape, the rail a strip above the stage.
  const nav = paneStack(layout, {
    panes: [{ id: 'stage', el: stage }, { id: 'manifest', el: manifest }],
    compactWhen: ({ width, phone }) => phone || width <= 1080,
    onChange: () => { if (refs) render(); },
  });
  cleanups.push(() => nav.destroy());
  // A finger pulls the stage down to look again (the store, the plugins).
  cleanups.push(pullRefresh(stage, () => Promise.all([loadPlugins(true), loadStoreAndRender()])));

  refs = {
    root, layout, stage, stageBody, manifest, manifestBody, manifestName, search, installBtn,
    status, statusDot, railItems, nav,
  };

  // Drag a plugin archive anywhere over the window to install it.
  ['dragenter', 'dragover'].forEach((evt) => on(tileEl, evt, (e) => {
    if (![...(e.dataTransfer?.types || [])].includes('Files')) return;
    e.preventDefault();
    e.stopPropagation();
    root.classList.add('is-dragging');
  }));
  ['dragleave', 'drop'].forEach((evt) => on(tileEl, evt, (e) => {
    e.preventDefault();
    e.stopPropagation();
    root.classList.remove('is-dragging');
  }));
  on(tileEl, 'drop', (e) => {
    const f = e.dataTransfer?.files?.[0];
    if (f) void uploadArchive(f);
  });
  on(window, 'plugins:changed', () => { void loadPlugins(true); void loadStoreAndRender(); });
  on(window, 'craft-store:changed', () => render());
  cleanups.push(() => {
    if (storePollTimer) clearTimeout(storePollTimer);
    storePollTimer = null;
  });

  storeBusySeen = new Set(storeApps().filter(craftAppBusy).map((app) => app.id));
  revealNext = true;
  shownKey = '';
  render();
  void loadPlugins();
  void loadStoreAndRender();
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
  selected = null;
  lastSignature = '';
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
      onClick: () => { void loadPlugins(true); void refreshStore().catch(() => {}).then(() => render()); },
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
