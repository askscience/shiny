/**
 * launcher.js — the plugin launcher (three-finger swipe down).
 *
 * Every installed plugin, plus the recent files and (opt-in) the open
 * windows, in one of four layouts the user picks in Settings → Appearance
 * (`getLauncher().style`, mirrored on `html[data-launcher-style]`):
 *
 *   grid    — the full-screen icon grid on the theme's frosted glass, with a
 *             search field and a strip of recent files above it;
 *   sheet   — a window-like sheet hanging from the top bar: recent files on
 *             the left, the plugins by category on the right;
 *   command — a dark palette born from the bar, keyboard first: one filtered
 *             list of recent files and plugins;
 *   mosaic  — a bento: recent files as tiles with their previews, then every
 *             plugin in a compact band.
 *
 * Recent files come from the Files plugin (`GET /api/files/recent`, the same
 * list as its "Recent" place) and are drawn with Files' own thumbnails
 * (`renderFileThumb`), so the launcher shows nothing Files would not. Window
 * previews are off by default (the overview already shows every window);
 * when on, each open window is cloned into a scaled, inert copy.
 *
 * The whole launcher is one progress value (0 = hidden, 1 = open): a swipe
 * pulls it in with the fingers and a release completes or springs back
 * (`launcherTransition`). The CSS reads `--launcher-p` / `--launcher-over` and
 * `html[data-launcher="opening|open|closing"]`; every revealing element (.rv)
 * carries a delay `--d` and derives its own progress from `--launcher-p`, so
 * a cascade is a pure function of the finger position. The motion language
 * (`html[data-motion]`) decides how things arrive (css/launcher.css).
 *
 * The gesture is recognised natively (crates/peakd/src/gestures.rs) and mapped
 * by gestures.js; Escape, an upward swipe and a click on the frosted desktop
 * all dismiss. Typing searches; arrows + Enter pick.
 */
import { apiFetch } from './api.js';
import { createTransition, clamp01 } from './gestureTransition.js';
import { refreshActivePlugins, refreshPluginCatalog } from './activePlugins.js';
import { pluginIconEl } from './pluginIcon.js';
import { effectiveLauncher as getLauncher } from './preferences.js';
import { pluginForFile, openWithPlugin } from './files.js';
import { openCoreWindow } from './tiles.js';
import { setIcon } from '../ui/index.js';
import { t } from './i18n.js';

/** How many recent files each layout shows. */
const RECENT_COUNT = { grid: 6, sheet: 5, command: 5, mosaic: 4 };
/** Recent files are re-read at most this often while the launcher is used. */
const RECENT_TTL = 15_000;

let panel = null;
let body = null;
let open = false;
/** On screen (opening, open or closing). */
let staged = false;
let running = null;
let active = new Set();
let plugins = [];
let recents = [];
let recentsAt = 0;
let query = '';
/** The last rendered data, so a refresh that changed nothing keeps the DOM. */
let signature = '';
/** The Files module (for its thumbnails), once loaded. */
let filesModule = null;
let launching = false;
/** The catalog has been read at least once (no "no plugins" flash before). */
let loaded = false;

/* ── Small helpers ───────────────────────────────────────────── */

function label(name) {
  return name.charAt(0).toUpperCase() + name.slice(1);
}

function h(tag, className = '', text = null) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text != null) node.textContent = text;
  return node;
}

function glyph(name, size = 16, className = 'launcher-glyph') {
  const span = h('span', className);
  span.setAttribute('aria-hidden', 'true');
  void setIcon(span, name, { size });
  return span;
}

/** Mark `node` as revealing at delay `d` (0..1) of the entrance. */
function rv(node, d) {
  node.classList.add('rv');
  node.style.setProperty('--d', Math.min(1, Math.max(0, d)).toFixed(3));
  return node;
}

const norm = (s) => String(s || '').normalize('NFD').replace(/[̀-ͯ]/g, '').toLowerCase();

function relativeTime(seconds) {
  if (!seconds) return '';
  const date = new Date(seconds * 1000);
  const now = new Date();
  const day = 86_400_000;
  const startOf = (d) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  const days = Math.round((startOf(now) - startOf(date)) / day);
  const time = date.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' });
  if (days <= 0) return t('launcher.today', { time }, `Today · ${time}`);
  if (days === 1) return t('launcher.yesterday', null, 'Yesterday');
  return date.toLocaleDateString(undefined, { day: 'numeric', month: 'short' });
}

/** The app a recent file opens in: its own app, else Files (quick look). */
function openerOf(entry) {
  return pluginForFile(entry.name) || 'files';
}

/* ── Data ────────────────────────────────────────────────────── */

function hasFiles() {
  return plugins.some((p) => p.name === 'files');
}

async function loadFilesModule() {
  if (filesModule) return filesModule;
  const meta = plugins.find((p) => p.name === 'files');
  if (!meta) return null;
  try {
    // The very URL tiles.js imports (version included), so this is the same
    // module instance as the Files window and shares its thumbnail caches.
    const url = new URL('../plugins/files/plugin.js', import.meta.url);
    if (meta.version) url.searchParams.set('v', meta.version);
    filesModule = await import(url.href);
  } catch (_) {
    filesModule = null;
  }
  return filesModule;
}

async function loadRecents(force = false) {
  const prefs = getLauncher();
  if (!prefs.recents || !hasFiles()) {
    recents = [];
    return;
  }
  if (!force && Date.now() - recentsAt < RECENT_TTL) return;
  try {
    const res = await apiFetch('/api/files/recent?limit=8', { authRedirect: false });
    recents = res?.data?.entries || [];
    recentsAt = Date.now();
  } catch (_) {
    // Keep the last list on a transient error.
  }
}

async function refresh(force = false) {
  try {
    const [list, activeNames] = await Promise.all([
      refreshPluginCatalog({ force }),
      refreshActivePlugins({ force }),
    ]);
    plugins = list || [];
    active = new Set(activeNames || []);
    loaded = true;
  } catch (_) {
    // Keep the last known list on a transient error.
  }
  await Promise.all([loadRecents(force), getLauncher().recents ? loadFilesModule() : null]);
  if (staged) render();
}

/* ── Picking ─────────────────────────────────────────────────── */

async function choosePlugin(name) {
  if (active.has(name)) {
    window.dispatchEvent(new CustomEvent('plugin:focus', { detail: { name } }));
    return;
  }
  try {
    await apiFetch('/api/plugins/activate', {
      method: 'POST',
      body: JSON.stringify({ name }),
    });
  } catch (err) {
    window.dispatchEvent(new CustomEvent('app:toast', {
      detail: {
        message: err.message || t('launcher.activateFailed', { name: label(name) },
          `Could not activate ${label(name)}`),
        type: 'error',
      },
    }));
    return;
  }
  // The same signal the tray and the Plugins page send, so the desktop mounts
  // the new window; `opened` makes it come to the front.
  localStorage.setItem('plugins.changed', String(Date.now()));
  window.dispatchEvent(new CustomEvent('plugins:changed', { detail: { opened: name } }));
}

function chooseRecent(entry) {
  void openWithPlugin({ plugin: openerOf(entry), path: entry.path, name: entry.name });
}

/** Run the picked item: it swells towards the user, then the launcher goes. */
function pick(item) {
  if (!item || launching) return;
  const { plugin, path, window: win } = item.dataset;
  const entry = path ? recents.find((r) => r.path === path) : null;
  const swell = !(plugin && active.has(plugin)) && !win;
  launching = true;
  if (swell) {
    panel.classList.add('is-launching');
    item.classList.add('is-launch');
  }
  const go = () => {
    if (win) window.dispatchEvent(new CustomEvent('plugin:focus', { detail: { name: win } }));
    else if (entry) chooseRecent(entry);
    else if (plugin) void choosePlugin(plugin);
    closeLauncher();
  };
  if (swell) setTimeout(go, 180);
  else go();
}

/* ── Building blocks ─────────────────────────────────────────── */

function searchField(placeholder) {
  const wrap = h('label', 'launcher-search');
  const input = h('input', 'launcher-search-input');
  input.type = 'search';
  input.autocomplete = 'off';
  input.spellcheck = false;
  input.placeholder = placeholder;
  input.value = query;
  input.setAttribute('aria-label', placeholder);
  input.addEventListener('input', () => {
    query = input.value;
    filter();
  });
  const count = h('span', 'launcher-search-count launcher-mono');
  wrap.append(glyph('ui/search', 16), input, count);
  return wrap;
}

function sectionHead(text, count = null, extra = null) {
  const head = h('div', 'launcher-section launcher-mono');
  head.append(h('span', '', text));
  if (extra) head.append(extra);
  if (count != null) head.append(h('span', 'launcher-section-count', String(count).padStart(2, '0')));
  return head;
}

function runDot() {
  const dot = h('i', 'launcher-run');
  dot.setAttribute('aria-hidden', 'true');
  return dot;
}

/** A plugin, as a big icon (`tile`) or a row with its description (`row`). */
function pluginItem(p, variant) {
  const name = label(p.name);
  const isActive = active.has(p.name);
  const btn = h('button', `launcher-pick launcher-plugin launcher-plugin--${variant}`);
  btn.type = 'button';
  btn.dataset.plugin = p.name;
  btn.dataset.search = norm(`${p.name} ${p.description || ''} ${p.category || ''}`);
  btn.classList.toggle('is-active', isActive);
  btn.title = p.description || name;
  btn.setAttribute('aria-label', isActive
    ? t('launcher.openItem', { name }, `${name} (open)`)
    : name);
  const icon = h('span', 'launcher-plugin-icon');
  icon.appendChild(pluginIconEl(p.name, { size: variant === 'tile' ? 44 : 26, fallback: 'ui/puzzle' }));
  if (variant === 'tile') {
    btn.append(icon, h('span', 'launcher-plugin-name', name), runDot());
  } else {
    const text = h('span', 'launcher-text');
    text.append(h('b', '', name), h('span', '', p.description || ''));
    btn.append(icon, text);
    if (isActive) btn.append(h('span', 'launcher-hint launcher-mono', t('launcher.running', null, 'Open')));
    btn.append(runDot());
  }
  return btn;
}

/** A recent file: its Files thumbnail, its name and where it opens. */
function recentItem(entry, variant) {
  const app = openerOf(entry);
  const btn = h('button', `launcher-pick launcher-recent launcher-recent--${variant}`);
  btn.type = 'button';
  btn.dataset.path = entry.path;
  btn.dataset.search = norm(`${entry.name} ${entry.path}`);
  btn.title = entry.path;
  const thumb = h('span', 'launcher-thumb');
  // A 44 px row cannot show a page: there, only photos and posters keep their
  // picture and everything else wears the app it opens in.
  const tiny = variant === 'row' && !entry.is_image && !entry.is_video;
  if (tiny) {
    thumb.appendChild(pluginIconEl(app, { size: 24, fallback: 'ui/file' }));
  } else if (filesModule?.renderFileThumb) {
    try { filesModule.renderFileThumb(thumb, entry); } catch (_) { thumb.appendChild(glyph('ui/file', 22)); }
  } else {
    thumb.appendChild(glyph('ui/file', 22));
  }
  const text = h('span', 'launcher-text');
  text.append(h('b', '', entry.name));
  const meta = h('span', 'launcher-recent-meta launcher-mono');
  meta.append(pluginIconEl(app, { size: 12, fallback: 'ui/folder' }), h('span', '', `${label(app)} · ${relativeTime(entry.modified)}`));
  text.append(meta);
  btn.append(thumb, text);
  return btn;
}

/**
 * An open window as an inert, scaled copy of its tile (`width` px wide). The
 * copy is the real DOM cloned once — canvases are repainted from the
 * originals, embedded pages (iframes, video) become a quiet placeholder.
 */
function windowItem(tile, width) {
  const name = tile.dataset.plugin;
  const btn = h('button', 'launcher-pick launcher-window');
  btn.type = 'button';
  btn.dataset.window = name;
  btn.dataset.search = norm(name);
  const frame = h('span', 'launcher-winprev');
  const W = tile.offsetWidth;
  const H = tile.offsetHeight;
  if (W > 0 && H > 0) {
    const k = width / W;
    frame.style.height = `${Math.round(Math.min(H * k, width * 0.75))}px`;
    const copy = tile.cloneNode(true);
    copy.classList.remove('hidden', 'tile--overview');
    copy.style.cssText = `position:absolute;left:0;top:0;margin:0;width:${W}px;height:${H}px;transform:scale(${k});transform-origin:0 0;pointer-events:none;`;
    copy.inert = true;
    copy.setAttribute('aria-hidden', 'true');
    const from = tile.querySelectorAll('canvas');
    copy.querySelectorAll('canvas').forEach((c, i) => {
      try { c.getContext('2d')?.drawImage(from[i], 0, 0); } catch (_) { /* tainted or lost */ }
    });
    const live = tile.querySelectorAll('iframe, video, embed, object');
    copy.querySelectorAll('iframe, video, embed, object').forEach((node, i) => {
      const ph = h('span', 'launcher-winprev-ph');
      ph.style.width = `${live[i]?.offsetWidth || 0}px`;
      ph.style.height = `${live[i]?.offsetHeight || 0}px`;
      node.replaceWith(ph);
    });
    frame.appendChild(copy);
  } else {
    frame.classList.add('is-empty');
    frame.style.height = `${Math.round(width * 0.6)}px`;
    frame.appendChild(pluginIconEl(name, { size: 36, fallback: 'ui/puzzle' }));
  }
  const cap = h('span', 'launcher-window-cap');
  cap.append(pluginIconEl(name, { size: 14, fallback: 'ui/puzzle' }), h('span', '', label(name)));
  btn.append(frame, cap);
  return btn;
}

function openTiles() {
  return [...document.querySelectorAll('#tile-grid > .tile[data-plugin]')]
    .filter((tile) => tile.dataset.plugin);
}

function manageButton() {
  const btn = h('button', 'launcher-manage');
  btn.type = 'button';
  btn.append(h('span', '', t('launcher.manage', null, 'Manage')), glyph('ui/arrow-up', 14, 'launcher-manage-arrow'));
  btn.addEventListener('click', () => {
    closeLauncher();
    openCoreWindow('plugins');
  });
  return btn;
}

/* ── The four layouts ────────────────────────────────────────── */

function categoryOf(p) {
  return (p.category || '').trim() || t('plugins.categoryOther', null, 'Other');
}

function buildGrid(ctx) {
  const wrap = h('div', 'lg-wrap');
  wrap.append(rv(searchField(t('launcher.search', null, 'Search')), 0));
  if (ctx.windows.length) {
    const strip = h('div', 'launcher-strip lg-windows');
    ctx.windows.slice(0, 5).forEach((tile, i) => strip.append(rv(windowItem(tile, 200), 0.05 + i * 0.04)));
    wrap.append(rv(sectionHead(t('launcher.windows', null, 'Open windows'), ctx.windows.length), 0.04), strip);
  }
  if (ctx.recents.length) {
    const strip = h('div', 'launcher-strip lg-recents');
    ctx.recents.forEach((entry, i) => strip.append(rv(recentItem(entry, 'card'), 0.08 + i * 0.04)));
    wrap.append(rv(sectionHead(t('launcher.recents', null, 'Recent files')), 0.06), strip);
  }
  const grid = h('div', 'lg-grid');
  const cols = 7;
  const rows = Math.max(1, Math.ceil(plugins.length / cols));
  plugins.forEach((p, i) => {
    const r = Math.floor(i / cols);
    const c = i % cols;
    // A cascade from the top centre, where the fingers pull it from.
    const d = 0.2 + (r / Math.max(1, rows - 1)) * 0.45 + (Math.abs(c - (cols - 1) / 2) / ((cols - 1) / 2)) * 0.35;
    grid.append(rv(pluginItem(p, 'tile'), d * 0.8));
  });
  wrap.append(grid, rv(h('p', 'lg-foot launcher-mono', t('launcher.dismissHint', null, 'Swipe up with three fingers to close · Esc')), 1));
  return wrap;
}

function buildSheet(ctx) {
  const sheet = h('section', 'ls-sheet launcher-surface');
  const head = h('header', 'ls-head');
  const title = h('div', 'ls-title');
  title.append(
    rv(h('span', 'launcher-mono ls-eyebrow', t('launcher.installed', { count: plugins.length }, `Plugins · ${plugins.length} installed`)), 0),
    rv(h('h1', '', null), 0.04),
  );
  title.lastChild.append(h('span', '', t('launcher.sheetTitle', null, 'What shall we open?')));
  head.append(title, rv(searchField(t('launcher.search', null, 'Search')), 0.08), rv(manageButton(), 0.1));

  const main = h('div', 'ls-body');
  let k = 0;
  if (ctx.windows.length || ctx.recents.length) {
    const aside = h('aside', 'ls-aside');
    if (ctx.windows.length) {
      aside.append(rv(sectionHead(t('launcher.windows', null, 'Open windows'), ctx.windows.length), 0.04));
      ctx.windows.slice(0, 3).forEach((tile, i) => aside.append(rv(windowItem(tile, 232), 0.08 + i * 0.06)));
    }
    if (ctx.recents.length) {
      aside.append(rv(sectionHead(t('launcher.recents', null, 'Recent files')), 0.1));
      ctx.recents.forEach((entry, i) => aside.append(rv(recentItem(entry, 'row'), 0.14 + i * 0.05)));
    }
    main.append(aside);
  } else {
    sheet.classList.add('is-wide');
  }
  const cats = h('div', 'ls-cats');
  const groups = new Map();
  for (const p of plugins) {
    const cat = categoryOf(p);
    if (!groups.has(cat)) groups.set(cat, []);
    groups.get(cat).push(p);
  }
  for (const [cat, list] of groups) {
    const section = h('section', 'ls-cat');
    section.append(rv(sectionHead(cat, list.length), Math.min(1, 0.1 + k * 0.035)));
    const rows = h('div', 'ls-list');
    for (const p of list) {
      k += 1;
      rows.append(rv(pluginItem(p, 'row'), Math.min(1, 0.12 + k * 0.035)));
    }
    section.append(rows);
    cats.append(section);
  }
  main.append(cats);
  const grab = h('div', 'ls-grab launcher-mono');
  grab.append(h('i'), h('span', '', t('launcher.pullUp', null, 'Pull up to close')));
  sheet.append(head, main, grab);
  return sheet;
}

function buildCommand(ctx) {
  const pal = h('section', 'lc-pal launcher-surface');
  const input = rv(searchField(t('launcher.commandPlaceholder', null, 'Open a plugin or a file…')), 0.2);
  input.append(h('kbd', 'launcher-kbd launcher-mono', 'esc'));
  pal.append(input);
  if (ctx.windows.length) {
    const row = h('div', 'launcher-strip lc-windows');
    ctx.windows.slice(0, 4).forEach((tile, i) => row.append(rv(windowItem(tile, 132), 0.26 + i * 0.03)));
    pal.append(row);
  }
  const list = h('div', 'lc-list');
  let k = 0;
  const d = () => Math.min(1, 0.3 + (k++) * 0.04);
  if (ctx.recents.length) {
    list.append(rv(sectionHead(t('launcher.recents', null, 'Recent files')), d()));
    ctx.recents.forEach((entry) => list.append(rv(recentItem(entry, 'row'), d())));
  }
  list.append(rv(sectionHead(t('launcher.plugins', null, 'Plugins'), plugins.length), d()));
  plugins.forEach((p) => list.append(rv(pluginItem(p, 'row'), d())));
  list.append(h('p', 'launcher-empty lc-empty hidden', t('launcher.noResults', null, 'Nothing matches.')));
  const foot = h('footer', 'lc-foot launcher-mono');
  foot.append(
    h('span', '', t('launcher.keysPick', null, '↑↓ choose')),
    h('span', '', t('launcher.keysOpen', null, '↵ open')),
    h('span', 'lc-sp'),
    h('span', '', t('launcher.count', { count: plugins.length }, `${plugins.length} plugins`)),
  );
  pal.append(list, rv(foot, 0.6));
  return pal;
}

function buildMosaic(ctx) {
  const wrap = h('div', 'lm-wrap');
  const head = h('header', 'lm-head');
  const date = new Date().toLocaleDateString(undefined, { weekday: 'long', day: 'numeric', month: 'long' });
  head.append(
    h('h1', '', t('launcher.greeting', null, 'Welcome back.')),
    h('span', 'launcher-mono lm-date', date),
    searchField(t('launcher.search', null, 'Search')),
  );
  wrap.append(rv(head, 0));
  const grid = h('div', 'lm-grid');
  if (ctx.windows.length) {
    grid.append(rv(h('div', 'launcher-section launcher-mono lm-label', t('launcher.windows', null, 'Open windows')), 0.06));
    const row = h('div', 'lm-windows');
    ctx.windows.slice(0, 4).forEach((tile, i) => row.append(rv(windowItem(tile, 280), 0.1 + i * 0.06)));
    grid.append(row);
  }
  if (ctx.recents.length) {
    grid.append(rv(h('div', 'launcher-section launcher-mono lm-label', t('launcher.recents', null, 'Recent files')), 0.14));
    const row = h('div', 'lm-recents');
    ctx.recents.forEach((entry, i) => {
      const tile = recentItem(entry, 'tile');
      // Each tile grows out of its own corner.
      tile.style.setProperty('--ox', `${(i / Math.max(1, ctx.recents.length - 1)) * 100}%`);
      row.append(rv(tile, 0.18 + i * 0.07));
    });
    grid.append(row);
  }
  const all = h('div', 'lm-all');
  const allHead = sectionHead(t('launcher.allPlugins', { count: plugins.length }, `All plugins · ${plugins.length}`));
  allHead.append(manageButton());
  const apps = h('div', 'lm-apps');
  plugins.forEach((p, i) => apps.append(rv(pluginItem(p, 'tile'), Math.min(1, 0.45 + i * 0.02))));
  all.append(allHead, apps);
  grid.append(rv(all, 0.4));
  wrap.append(grid);
  return wrap;
}

const BUILD = { grid: buildGrid, sheet: buildSheet, command: buildCommand, mosaic: buildMosaic };

function render() {
  if (!body) return;
  const prefs = getLauncher();
  const windows = prefs.windows ? openTiles() : [];
  const recentList = prefs.recents ? recents.slice(0, RECENT_COUNT[prefs.style]) : [];
  const sig = JSON.stringify([
    prefs, plugins.map((p) => [p.name, p.description, p.category]), [...active],
    recentList.map((r) => [r.path, r.modified]), windows.map((w) => w.dataset.plugin), !!filesModule,
  ]);
  if (sig === signature && body.childElementCount) return;
  signature = sig;
  const focusWasIn = body.contains(document.activeElement);
  body.textContent = '';
  panel.dataset.style = prefs.style;
  if (!plugins.length) {
    if (!loaded) return;
    body.append(h('p', 'launcher-empty', t('launcher.noPlugins', null, 'No plugins installed')));
    return;
  }
  body.append(BUILD[prefs.style]({ windows, recents: recentList }));
  filter();
  if (focusWasIn) body.querySelector('.launcher-search-input')?.focus();
}

/** Apply the query: non-matching items step back (the layout never jumps). */
function filter() {
  if (!body) return;
  const q = norm(query.trim());
  const picks = body.querySelectorAll('.launcher-pick');
  let hits = 0;
  for (const item of picks) {
    const match = !q || (item.dataset.search || '').includes(q);
    item.classList.toggle('is-off', !match);
    if (match) hits += 1;
  }
  // The palette lists only the matches, and hides a section left empty.
  if (panel.dataset.style === 'command') {
    for (const head of body.querySelectorAll('.lc-list > .launcher-section')) {
      let next = head.nextElementSibling;
      let any = false;
      while (next && !next.classList.contains('launcher-section')) {
        if (next.classList.contains('launcher-pick') && !next.classList.contains('is-off')) any = true;
        next = next.nextElementSibling;
      }
      head.classList.toggle('hidden', !any);
    }
    const none = !body.querySelector('.lc-list > .launcher-pick:not(.is-off)');
    body.querySelector('.lc-empty')?.classList.toggle('hidden', !none);
  }
  for (const count of body.querySelectorAll('.launcher-search-count')) {
    count.textContent = q ? `${hits} / ${picks.length}` : '';
  }
  const sel = body.querySelector('.launcher-pick.is-sel');
  if (!sel || sel.classList.contains('is-off')) select(q ? visiblePicks()[0] : null);
}

/* ── Keyboard ────────────────────────────────────────────────── */

function visiblePicks() {
  return body ? [...body.querySelectorAll('.launcher-pick:not(.is-off)')] : [];
}

function select(item) {
  for (const el of body?.querySelectorAll('.launcher-pick.is-sel') || []) el.classList.remove('is-sel');
  if (!item) return;
  item.classList.add('is-sel');
  item.scrollIntoView({ block: 'nearest', inline: 'nearest' });
}

/** Arrow navigation by geometry, so it works in every layout. */
function move(key) {
  const items = visiblePicks();
  if (!items.length) return;
  const current = body.querySelector('.launcher-pick.is-sel:not(.is-off)');
  const i = items.indexOf(current);
  if (i < 0) { select(items[0]); return; }
  if (key === 'ArrowRight' || key === 'ArrowLeft') {
    select(items[(i + (key === 'ArrowRight' ? 1 : -1) + items.length) % items.length]);
    return;
  }
  const down = key === 'ArrowDown';
  const r = current.getBoundingClientRect();
  const cx = r.left + r.width / 2;
  let best = null;
  let bestScore = Infinity;
  for (const item of items) {
    if (item === current) continue;
    const b = item.getBoundingClientRect();
    const dy = down ? b.top - (r.bottom - 4) : (r.top + 4) - b.bottom;
    if (dy < 0) continue;
    const score = dy + Math.abs(b.left + b.width / 2 - cx) * 0.7;
    if (score < bestScore) { bestScore = score; best = item; }
  }
  if (best) select(best);
}

/** The launcher is the phone's home screen (js/phoneShell.js): it stays up
 *  until a window takes the screen, so nothing but a pick dismisses it. */
function isHome() {
  return document.documentElement.dataset.launcherHome === '1';
}

function onKey(e) {
  if (!staged) return;
  if (e.key === 'Escape') {
    if (isHome()) return;
    e.preventDefault();
    e.stopImmediatePropagation();
    closeLauncher();
    return;
  }
  if (!open || launching) return;
  if (['ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight'].includes(e.key)) {
    // Left/right move the caret while typing; up/down always pick.
    const typing = e.target?.classList?.contains('launcher-search-input') && query;
    if (typing && (e.key === 'ArrowLeft' || e.key === 'ArrowRight')) return;
    e.preventDefault();
    move(e.key);
    return;
  }
  if (e.key === 'Enter') {
    const item = body.querySelector('.launcher-pick.is-sel:not(.is-off)') || (query ? visiblePicks()[0] : null);
    if (!item) return;
    e.preventDefault();
    pick(item);
    return;
  }
  // Typing anywhere searches: send the key to the field.
  const input = body.querySelector('.launcher-search-input');
  if (input && document.activeElement !== input && e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey) {
    input.focus();
  }
}

/* ── Progress & lifecycle ────────────────────────────────────── */

export function isLauncherOpen() {
  return open;
}

/** True from `beginLauncher()` until the launcher is fully hidden again. */
export function isLauncherActive() {
  return staged;
}

function setPhase(phase) {
  const root = document.documentElement;
  if (phase) root.dataset.launcher = phase;
  else delete root.dataset.launcher;
}

/** Put the launcher on screen at progress 0 (or mark an open one closing). */
export function beginLauncher() {
  if (!panel) return false;
  if (staged) {
    setPhase(open ? 'closing' : 'opening');
    return true;
  }
  staged = true;
  launching = false;
  window.dispatchEvent(new CustomEvent('overlay:open', { detail: { which: 'launcher' } }));
  setLauncherProgress(0);
  setPhase('opening');
  // Picking happens here now: nothing behind may keep the keyboard.
  document.activeElement?.blur?.();
  panel.classList.remove('hidden', 'is-launching');
  // Set before the panel paints, so native pages hide as the panel appears.
  document.body.classList.add('launcher-active');
  render();
  void refresh();
  return true;
}

/** Render the launcher at `p` (0 = hidden, 1 = open). */
export function setLauncherProgress(p) {
  document.documentElement.style.setProperty('--launcher-p', clamp01(p).toFixed(4));
  // Beyond the ends (a rubber band) only the panel's travel stretches.
  const over = Number(p) > 1 ? Number(p) - 1 : Number(p) < 0 ? Number(p) : 0;
  document.documentElement.style.setProperty('--launcher-over', over.toFixed(4));
}

/** Settle: `true` leaves the launcher open, `false` hides it. */
export function endLauncher(openAtEnd) {
  if (!staged) return;
  const root = document.documentElement;
  if (openAtEnd) {
    open = true;
    setLauncherProgress(1);
    setPhase('open');
    return;
  }
  open = false;
  staged = false;
  launching = false;
  query = '';
  panel?.classList.add('hidden');
  panel?.classList.remove('is-launching');
  document.body.classList.remove('launcher-active');
  setPhase(null);
  root.style.removeProperty('--launcher-p');
  root.style.removeProperty('--launcher-over');
  // Drop the DOM (and any window copies) while hidden.
  if (body) body.textContent = '';
  signature = '';
  window.dispatchEvent(new Event('launcher:closed'));
}

/**
 * An interactive open (`wantOpen`) or close of the launcher, for a gesture to
 * drive or a discrete caller to `play()`. Null when there is nothing to do.
 */
export function launcherTransition(wantOpen) {
  running?.finishNow(true);
  if (wantOpen ? open : !staged) return null;
  if (!beginLauncher()) return null;
  setPhase(wantOpen ? 'opening' : 'closing');
  const transition = createTransition({
    apply: (k) => setLauncherProgress(wantOpen ? k : 1 - k),
    finish: (committed) => {
      if (running === transition) running = null;
      endLauncher(committed ? wantOpen : !wantOpen);
    },
  });
  running = transition;
  return transition;
}

export function openLauncher() {
  launcherTransition(true)?.play();
}

export function closeLauncher() {
  launcherTransition(false)?.play();
}

export function initLauncher() {
  if (panel) return;
  panel = document.getElementById('plugin-launcher');
  body = document.getElementById('plugin-launcher-body');
  if (!panel || !body) return;

  panel.addEventListener('click', (e) => {
    const item = e.target.closest('.launcher-pick');
    if (item && panel.contains(item)) {
      pick(item);
      return;
    }
    // A click on the frosted desktop (outside every surface) leaves.
    if (!isHome() && !e.target.closest('.launcher-surface, .launcher-search, .launcher-manage, .launcher-section, .launcher-strip, .lg-grid, .lm-grid, .lm-head')) {
      closeLauncher();
    }
  });

  window.addEventListener('keydown', onKey, true);

  window.addEventListener('overlay:open', (e) => {
    if (staged && e.detail?.which !== 'launcher') closeLauncher();
  });
  window.addEventListener('plugins:changed', () => {
    if (staged) void refresh(true);
  });
  window.addEventListener('launcher:change', () => {
    recentsAt = 0;
    if (staged) void refresh();
  });
  // A file saved anywhere is a new recent file.
  window.addEventListener('files:changed', () => { recentsAt = 0; });
}
