/**
 * browser.js — the Browser plugin's window.
 *
 * The window's chrome is HTML (this file): a tab strip, back / forward /
 * reload / home, and an address bar that doubles as a search box. The **page**
 * is not rendered here — the shell renders it in a native WebKit child webview
 * (`crates/peakd`'s `browse` module) at the page's real origin, so anti-bot
 * challenges (Cloudflare) pass and cookies/TLS are the page's own. This file
 * drives that view over the shell's IPC bridge and mirrors its URL/title back
 * into the toolbar.
 *
 * The home surface is the one thing rendered locally: a sandboxed `srcdoc`
 * iframe holding the related-news shelf, built from what the user searches for.
 *
 * Every control is a core UI component — `iconButton` for the toolbar,
 * `searchBar` for the address field, `icon` for the glyphs — so the window
 * inherits the theme's button metrics, focus rings and accent for free. The
 * plugin's own stylesheet carries layout only (PLUGINS.md §19).
 *
 * Server state lives on the plugin side (`/api/browser/*`), so this file owns
 * presentation and local navigation history only.
 */
import { apiFetch } from '../../js/api.js';
import { openWithPlugin, pluginForFile, revealInFiles } from '../../js/files.js';
import { icon, iconButton, searchBar } from '../../ui/index.js';

export const BROWSER_PLUGIN = 'browser';

/** How many tabs may be open. Each is a live web view, so this is bounded. */
const MAX_TABS = 8;

/**
 * The native-view bridge.
 *
 * The shell exposes `window.ipc` (see `crates/peakd`). Every tab is rendered by
 * a real WebKit child webview at the page's true origin — no iframe, no HTML
 * rewriting — so anti-bot systems such as Cloudflare see an ordinary browser.
 */
const nativeAvailable =
  typeof window !== 'undefined' &&
  window.ipc &&
  typeof window.ipc.postMessage === 'function';
const NATIVE_PREFIX = 'peakd:view:';

/** Send one native-view command. Returns false when there is no bridge. */
function nativeSend(payload) {
  if (!nativeAvailable) return false;
  try {
    window.ipc.postMessage(NATIVE_PREFIX + JSON.stringify(payload));
    return true;
  } catch (_) {
    return false;
  }
}

/**
 * Send a shell message with a non-view prefix (`peakd:settings:`,
 * `peakd:filter:`, `peakd:download:`). The app view is the only one whose IPC
 * bridge forwards these; a browsing page's bridge accepts only `peakd:exit`.
 */
function shellSend(prefix, payload) {
  if (!nativeAvailable) return false;
  try {
    window.ipc.postMessage(prefix + JSON.stringify(payload));
    return true;
  } catch (_) {
    return false;
  }
}

/** Send a raw (non-JSON) shell message, e.g. `peakd:downloads:list`. */
function shellRaw(message) {
  if (!nativeAvailable) return false;
  try {
    window.ipc.postMessage(message);
    return true;
  } catch (_) {
    return false;
  }
}

/**
 * The viewport rectangle in CSS pixels, plus the device ratio so the shell can
 * map it to native pixels without assuming the kiosk's page zoom.
 */
function viewportRect() {
  const el = frameWrapEl || viewportEl;
  const r = el && el.getBoundingClientRect ? el.getBoundingClientRect() : null;
  if (!r || !r.width || !r.height) return null;
  return {
    x: r.left,
    y: r.top,
    w: r.width,
    h: r.height,
    dpr: window.devicePixelRatio || 1,
  };
}

/**
 * Whether the active native view should currently be on screen.
 *
 * A native child view is a real window stacked above the page, so it ignores
 * `display: none` and every HTML layer. When its Browser window is hidden (a
 * workspace switch), covered by the overview/launcher, or underneath a higher
 * floating window, it would otherwise keep floating over them; the view is
 * hidden explicitly whenever it is not what the user is looking at.
 */
function nativeShouldShow(tab) {
  if (!tab || !tab.native || !tileEl) return false;
  if (tileEl.classList.contains('hidden')) return false;
  const body = document.body;
  if (body && body.classList) {
    // The overview and launcher are full-screen HTML layers over the desktop;
    // the native view cannot be composited beneath them.
    if (body.classList.contains('overview-active')) return false;
    if (body.classList.contains('launcher-active')) return false;
  }
  return !isNativeOccluded();
}

/** True when a higher floating window overlaps this view's own rectangle. */
function isNativeOccluded() {
  if (!tileEl) return false;
  const grid = document.getElementById('tile-grid');
  // Tiled layouts never overlap; only the free "Windows" layout does.
  if (!grid || grid.dataset.layout !== 'windows') return false;
  const rect = viewportRect();
  if (!rect) return false;
  const z = Number(tileEl.style.zIndex) || 0;
  const tiles = grid.querySelectorAll(':scope > .tile[data-plugin]');
  for (const el of tiles) {
    if (el === tileEl || el.classList.contains('hidden')) continue;
    if ((Number(el.style.zIndex) || 0) <= z) continue;
    const r = el.getBoundingClientRect();
    if (rect.x < r.right && r.left < rect.x + rect.w
      && rect.y < r.bottom && r.top < rect.y + rect.h) {
      return true;
    }
  }
  return false;
}

/** Push bounds and visibility for one tab, skipping no-op updates. */
function syncNativeTab(tab) {
  if (!nativeAvailable || !tab || !tab.native) return;
  const show = nativeShouldShow(tab);
  if (show !== lastNativeVisible) {
    lastNativeVisible = show;
    nativeSend({ op: 'setVisible', id: tab.id, visible: show });
  }
  if (!show) {
    lastNativeRect = '';
    return;
  }
  const rect = viewportRect();
  if (!rect) return;
  const key = `${Math.round(rect.x * rect.dpr)},${Math.round(rect.y * rect.dpr)},`
    + `${Math.round(rect.w * rect.dpr)},${Math.round(rect.h * rect.dpr)}`;
  if (key === lastNativeRect) return;
  lastNativeRect = key;
  nativeSend({ op: 'setBounds', id: tab.id, rect });
}

/** Keep the shell's native view aligned with the viewport element. */
function syncActiveNative() {
  syncNativeTab(activeTab());
}

let tileEl = null;
let tabstripEl = null;
let frameWrapEl = null;
let viewportEl = null;
let addressEl = null;
let backBtn = null;
let forwardBtn = null;
let statusEl = null;
let shieldBtn = null;
let downloadsBtn = null;
let incognitoBtn = null;
let downloadsPanel = null;
let downloadsListEl = null;

/**
 * The window's server-backed settings. `adblock` drives the shield toggle and
 * `downloadsDir` is the absolute directory the shell saves downloads into (the
 * user's `Downloads` folder, resolved server-side per account).
 */
let browserSettings = { adblock: true, incognito: false, downloadsDir: '' };

/** Live downloads, newest first, keyed by the shell's id. */
let downloads = [];

// Last state pushed for the active native view, so the per-frame sync can skip
// updates the shell already has. `null` means "unknown, send it".
let lastNativeVisible = null;
let lastNativeRect = '';

/**
 * The window's tabs. Each owns a native child webview in the shell (and a local
 * iframe only for its home surface), a server session id and its own navigation
 * history; only the active one is visible. `activeTabId` always names one of
 * them while the window is mounted.
 */
let tabs = [];
let activeTabId = null;
let tabSeq = 0;

let wired = false;

/**
 * Load this window's stylesheet once.
 *
 * Core builds each plugin window's `.tile` chrome and hands the inside to the
 * plugin; it does not auto-inject a plugin stylesheet. A browser window's
 * layout is genuinely its own (an address bar over a web viewport), so rather
 * than reaching into core's `tiles.css` the window loads its own file — the
 * one scoped exception documented at the top of `plugin.css`.
 */
function ensureStylesheet() {
  const id = 'browser-plugin-css';
  if (document.getElementById(id)) return;
  const link = document.createElement('link');
  link.id = id;
  link.rel = 'stylesheet';
  link.href = '/plugins/browser/plugin.css';
  document.head.appendChild(link);
}

/* ── Tabs ─────────────────────────────────────────────────────── */

function activeTab() {
  return tabs.find((t) => t.id === activeTabId) || null;
}

function tabTitle(tab) {
  if (tab.title) return tab.title;
  if (tab.url) {
    try {
      return new URL(tab.url).hostname.replace(/^www\./, '');
    } catch (_) {
      /* fall through to the placeholder */
    }
  }
  return 'New tab';
}

/**
 * How long a native view may take before the "Loading…" hint is dropped.
 *
 * Native loads are the real web at its real origin: slow pages are normal, and
 * forcing a reload (as the proxy watchdog does) interrupts Cloudflare
 * challenges. This timer is therefore long and never reports an error — the
 * child webview shows its own error page if a load genuinely fails.
 */
const NATIVE_LOAD_TIMEOUT_MS = 45000;

function clearWatchdog(tab) {
  if (tab?.watchdog) clearTimeout(tab.watchdog);
  if (tab) tab.watchdog = null;
}

/** Create a tab, its iframe, and (unless a URL is given) its home surface. */
function createTab({ url = '', activate = true, incognito = false } = {}) {
  const tab = {
    id: `t${++tabSeq}`,
    sessionId: null,
    title: '',
    url: '',
    home: true,
    /** Whether this tab is rendered by an off-the-record profile. */
    incognito: !!incognito,
    /** Whether this tab is currently rendered by a native child webview. */
    native: false,
    history: [],
    historyIndex: -1,
    loaded: false,
    watchdog: null,
    frameEl: null,
  };
  tab.frameEl = buildFrame(tab);
  tabs.push(tab);
  frameWrapEl.appendChild(tab.frameEl);

  // Bound tabs keep the tab model honest even if a very long session opens
  // more than the cap allows: close the oldest inactive one.
  if (tabs.length > MAX_TABS) {
    const victim = tabs.find((t) => t.id !== tab.id && t.id !== activeTabId);
    if (victim) closeTab(victim.id);
  }

  if (activate) setActiveTab(tab.id);
  renderTabs();
  if (url) void navigateTo(url, { tab });
  else void showHome(tab);
  return tab;
}

/** Tear down one tab's frame and state. */
function destroyTab(tab) {
  clearWatchdog(tab);
  // Release the native child webview, if this tab had one.
  if (nativeAvailable) nativeSend({ op: 'close', id: tab.id });
  const frame = tab.frameEl;
  if (!frame) return;
  frame.src = 'about:blank';
  frame.remove();
  tab.frameEl = null;
}

function closeTab(id) {
  const idx = tabs.findIndex((t) => t.id === id);
  if (idx < 0) return;
  const [tab] = tabs.splice(idx, 1);
  // Best-effort: tell the server its session is done. A failure here cannot
  // stop the tab from closing.
  if (tab.sessionId) {
    void apiFetch('/api/browser/session/close', {
      method: 'POST',
      body: JSON.stringify({ id: tab.sessionId }),
    }).catch(() => {});
  }
  const wasActive = activeTabId === id;
  destroyTab(tab);

  if (wasActive) {
    const next = tabs[idx] || tabs[idx - 1] || null;
    if (next) {
      setActiveTab(next.id);
    } else {
      activeTabId = null;
      renderTabs();
      createTab({});
      return;
    }
  }
  renderTabs();
  updateNavButtons();
}

function setActiveTab(id) {
  if (!tabs.some((t) => t.id === id)) return;
  activeTabId = id;
  const tab = activeTab();
  for (const t of tabs) {
    t.frameEl.classList.toggle('is-active', t.id === id);
    // Only the active tab's native view is shown; the rest stay alive (their
    // history and cookies) but hidden. The active one goes through the shared
    // sync so window occlusion/overlays are honoured too.
    if (nativeAvailable && t.id !== id) {
      nativeSend({ op: 'setVisible', id: t.id, visible: false });
    }
  }
  if (addressEl && document.activeElement !== addressEl.input) {
    addressEl.input.value = tab?.url || '';
  }
  lastNativeVisible = null;
  lastNativeRect = '';
  syncActiveNative();
  if (nativeAvailable && tab?.native) nativeSend({ op: 'focus', id: tab.id });
  renderTabs();
  updateNavButtons();
}

/** The tab strip: one button per tab, plus a new-tab action. */
function renderTabs() {
  if (!tabstripEl) return;
  const nodes = [];
  for (const tab of tabs) {
    const el = document.createElement('div');
    el.className = 'browser-tab' + (tab.id === activeTabId ? ' is-active' : '')
      + (tab.incognito ? ' is-incognito' : '');
    el.setAttribute('role', 'tab');
    el.setAttribute('aria-selected', String(tab.id === activeTabId));

    const label = document.createElement('button');
    label.type = 'button';
    label.className = 'browser-tab-label';
    if (tab.incognito) {
      const mask = icon('ui/incognito', { size: 12 });
      mask.classList.add('browser-tab-mask');
      label.appendChild(mask);
    }
    const text = document.createElement('span');
    text.className = 'browser-tab-text';
    text.textContent = tabTitle(tab);
    label.appendChild(text);
    label.title = tab.incognito ? `Incognito — ${tab.url || 'New tab'}` : (tab.url || 'New tab');
    label.addEventListener('click', () => setActiveTab(tab.id));

    const close = document.createElement('button');
    close.type = 'button';
    close.className = 'browser-tab-close';
    close.setAttribute('aria-label', `Close ${tabTitle(tab)}`);
    close.appendChild(icon('ui/close', { size: 12 }));
    close.addEventListener('click', (e) => {
      e.stopPropagation();
      closeTab(tab.id);
    });

    el.append(label, close);
    nodes.push(el);
  }
  const add = iconButton({
    icon: 'ui/plus',
    size: 'sm',
    label: 'New tab',
    onClick: () => createTab({}),
  });
  add.classList.add('browser-newtab');
  nodes.push(add);
  tabstripEl.replaceChildren(...nodes);
}

/**
 * The local iframe. It now carries only the home surface (a sandboxed
 * `srcdoc` shelf); real pages are rendered by a native child webview, so this
 * element is parked and hidden whenever a tab has one.
 */
function buildFrame(tab) {
  const frame = document.createElement('iframe');
  frame.className = 'browser-frame';
  // The home shelf is built from server data, so it stays sandboxed without
  // `allow-same-origin`: it must not be able to reach the app shell.
  frame.setAttribute('referrerpolicy', 'no-referrer');
  return frame;
}

/**
 * Render a real URL in a native child webview.
 *
 * The local iframe is parked on `about:blank` and hidden so it does not fight
 * the native view for the viewport; going Home restores it (see `showHome`).
 * The shell owns the view's history, so back/forward/reload are IPC commands
 * rather than the injected page shim the proxy path uses.
 */
function loadNative(tab, url) {
  if (!tab) return;
  tab.native = true;
  tab.loaded = false;
  clearWatchdog(tab);
  // Non-destructive: drop the "Loading…" hint, never reload or claim the proxy
  // is down. See NATIVE_LOAD_TIMEOUT_MS.
  tab.watchdog = setTimeout(() => {
    tab.watchdog = null;
    if (!tab.loaded && tab.id === activeTabId) setStatus('');
  }, NATIVE_LOAD_TIMEOUT_MS);
  if (tab.frameEl) {
    tab.frameEl.src = 'about:blank';
    tab.frameEl.classList.remove('is-active');
    tab.frameEl.classList.add('is-native-hidden');
  }
  const rect = viewportRect();
  nativeSend({
    op: 'open',
    id: tab.id,
    url,
    rect,
    visible: tab.id === activeTabId,
    incognito: !!tab.incognito,
  });
  lastNativeVisible = null;
  lastNativeRect = '';
  if (tab.id === activeTabId) {
    nativeSend({ op: 'focus', id: tab.id });
    // Let occlusion/overlay state decide whether it may actually stay visible.
    syncActiveNative();
  }
}

/** Park this tab's native view and hand the viewport back to the iframe. */
function hideNative(tab) {
  if (!nativeAvailable || !tab) return;
  tab.native = false;
  nativeSend({ op: 'setVisible', id: tab.id, visible: false });
  if (tab.id === activeTabId) {
    lastNativeVisible = false;
    lastNativeRect = '';
  }
  if (tab.frameEl) {
    tab.frameEl.classList.remove('is-native-hidden');
    tab.frameEl.classList.toggle('is-active', tab.id === activeTabId);
  }
}

async function navigateTo(input, { tab = activeTab() } = {}) {
  const value = String(input || '').trim();
  if (!value || !tab) return;

  if (tab.id === activeTabId) setStatus('Loading…');
  let data;
  try {
    const res = await apiFetch('/api/browser/navigate', {
      method: 'POST',
      body: JSON.stringify({
        session_id: tab.sessionId,
        input: value,
        incognito: !!tab.incognito,
      }),
    });
    data = res?.data;
  } catch (_) {
    if (tab.id === activeTabId) setStatus('Could not open that page');
    return;
  }
  if (!data?.url) {
    if (tab.id === activeTabId) setStatus('Could not open that page');
    return;
  }

  tab.sessionId = data.session?.id || tab.sessionId;
  tab.home = false;
  recordLocation(tab, data.url || value);
  if (tab.id === activeTabId) setStatus('');
  loadNative(tab, data.url || value);
}

/* ── Chrome ───────────────────────────────────────────────────── */

function setStatus(text) {
  if (!statusEl) return;
  statusEl.textContent = text || '';
  statusEl.classList.toggle('hidden', !text);
}

function updateNavButtons() {
  const tab = activeTab();
  const onPage = !!tab && tab.url !== '';
  if (backBtn) backBtn.disabled = !onPage || tab.historyIndex <= 0;
  if (forwardBtn) {
    forwardBtn.disabled = !onPage || tab.historyIndex >= tab.history.length - 1;
  }
}

function buildTabStrip() {
  const strip = document.createElement('div');
  strip.className = 'browser-tabstrip';
  strip.setAttribute('role', 'tablist');
  strip.setAttribute('aria-label', 'Tabs');
  return strip;
}

function buildToolbar() {
  const bar = document.createElement('div');
  bar.className = 'browser-bar';

  // Back and forward are the same glyph, mirrored. The theme ships
  // `ui/arrow-left` but its `ui/forward` is a curved "share" arrow, so the pair
  // looked like two unrelated features; `browser-nav--forward` flips the one
  // arrow in CSS so both are literally the same drawing (PLUGINS.md §19: window
  // icons come from the theme, and a plugin ships no icon assets).
  backBtn = iconButton({
    icon: 'ui/arrow-left',
    size: 'sm',
    label: 'Back',
    onClick: () => go(-1),
  });
  forwardBtn = iconButton({
    icon: 'ui/arrow-left',
    size: 'sm',
    label: 'Forward',
    onClick: () => go(1),
  });
  backBtn.classList.add('browser-nav', 'browser-nav--back');
  forwardBtn.classList.add('browser-nav', 'browser-nav--forward');

  const reloadBtn = iconButton({
    icon: 'ui/refresh',
    size: 'sm',
    label: 'Reload',
    onClick: () => reload(),
  });
  const homeBtn = iconButton({
    icon: 'ui/launcher',
    size: 'sm',
    label: 'Home',
    onClick: () => goHome(),
  });

  // The address bar is the library's search field — the same element the
  // YouTube and Radio windows use — because that is what this control is: a
  // search box that also accepts a URL.
  addressEl = document.createElement('form');
  addressEl.className = 'browser-address';
  const search = searchBar({ placeholder: 'Search or enter an address' });
  const input = search.input;
  // `searchBar` is a div, so the browser's own submit gesture needs wiring:
  // Enter in a form input fires `submit`, which we intercept.
  input.setAttribute('autocomplete', 'off');
  input.setAttribute('spellcheck', 'false');
  input.setAttribute('aria-label', 'Search or enter an address');
  input.setAttribute('enterkeyhint', 'go');
  addressEl.appendChild(search);
  addressEl.addEventListener('submit', (e) => {
    e.preventDefault();
    input.blur();
    void navigateTo(input.value);
  });
  addressEl.input = input;
  addressEl.search = search;

  // The right cluster: the ad-block shield (the requested toggle), the
  // downloads manager, and a new incognito tab. Appended after the flexible
  // address field, so they sit on the right.
  shieldBtn = iconButton({
    icon: 'ui/shield',
    size: 'sm',
    label: 'Ad blocking',
    onClick: () => toggleShield(),
  });
  shieldBtn.classList.add('browser-shield');

  downloadsBtn = iconButton({
    icon: 'ui/download',
    size: 'sm',
    label: 'Downloads',
    onClick: () => toggleDownloadsPanel(),
  });
  downloadsBtn.classList.add('browser-downloads-btn');

  incognitoBtn = iconButton({
    icon: 'ui/incognito',
    size: 'sm',
    label: 'New incognito tab',
    onClick: () => createTab({ incognito: true }),
  });
  incognitoBtn.classList.add('browser-incognito');

  bar.append(backBtn, forwardBtn, reloadBtn, homeBtn, addressEl, shieldBtn, downloadsBtn, incognitoBtn);
  return bar;
}

function buildViewport() {
  const wrap = document.createElement('div');
  wrap.className = 'browser-viewport';

  frameWrapEl = document.createElement('div');
  frameWrapEl.className = 'browser-frames';
  wrap.appendChild(frameWrapEl);

  statusEl = document.createElement('div');
  statusEl.className = 'browser-status hidden';
  wrap.appendChild(statusEl);

  return wrap;
}

/* ── Navigation ───────────────────────────────────────────────── */

/**
 * Reconcile one tab's history with a URL the shell reports as current.
 *
 * The frame owns the real history (its back/forward is browser-native), so
 * this is a mirror for the address bar and for enabling the buttons: an
 * adjacent URL means the user went back or forward, anything else is a new
 * navigation and truncates the forward entries.
 */
function recordLocation(tab, url) {
  if (!url || !tab) return;
  tab.url = url;
  if (tab.id === activeTabId && addressEl && document.activeElement !== addressEl.input) {
    addressEl.input.value = url;
  }
  if (tab.history[tab.historyIndex] !== url) {
    if (tab.history[tab.historyIndex + 1] === url) {
      tab.historyIndex += 1;
    } else if (tab.history[tab.historyIndex - 1] === url) {
      tab.historyIndex -= 1;
    } else {
      const existing = tab.history.lastIndexOf(url);
      if (existing >= 0) {
        tab.historyIndex = existing;
      } else {
        tab.history = tab.history.slice(0, tab.historyIndex + 1);
        tab.history.push(url);
        tab.historyIndex = tab.history.length - 1;
      }
    }
  }
  renderTabs();
  if (tab.id === activeTabId) updateNavButtons();
}

/** Ask the active view to move through *its* history — no server round-trip. */
function go(delta) {
  const tab = activeTab();
  if (!tab || !tab.url || !tab.native) return;
  nativeSend({ op: delta < 0 ? 'back' : 'forward', id: tab.id });
}

function reload() {
  const tab = activeTab();
  if (!tab) return;
  if (!tab.url) {
    void showHome(tab);
    return;
  }
  if (tab.native) nativeSend({ op: 'reload', id: tab.id });
}

function goHome() {
  void showHome(activeTab());
}

/* ── Settings, the shield toggle, and downloads ───────────────────
 *
 * Three things live on the right of the toolbar:
 *  - the **shield**, which toggles the shell's ad blocker live and persists the
 *    choice per user (`/api/browser/settings`);
 *  - the **downloads** button + panel, fed by shell events
 *    (`window.__peakdViewEvent`, type `download`) and persisted server-side;
 *  - a **new incognito tab**.
 */

/** Fetch the user's settings and push them to the shell. */
async function loadSettings() {
  try {
    const res = await apiFetch('/api/browser/settings');
    const d = res?.data || {};
    browserSettings.adblock = d.adblock !== false;
    browserSettings.incognito = !!d.incognito;
    browserSettings.downloadsDir = d.downloads_dir || '';
  } catch (_) {
    /* defaults stay in place */
  }
  pushSettings();
  updateShield();
}

/** Tell the shell the current settings (adblock + where downloads go). */
function pushSettings() {
  shellSend('peakd:settings:', {
    adblock: browserSettings.adblock,
    downloadsDir: browserSettings.downloadsDir,
  });
}

/** Flip ad blocking, optimistically, and persist it. */
function toggleShield() {
  const on = !browserSettings.adblock;
  browserSettings.adblock = on;
  updateShield();
  shellSend('peakd:filter:', { enabled: on });
  void apiFetch('/api/browser/settings', {
    method: 'POST',
    body: JSON.stringify({ adblock: on }),
  }).catch(() => {});
}

/** Reflect the shield state on its button (on = accent, off = muted). */
function updateShield() {
  if (!shieldBtn) return;
  const on = browserSettings.adblock;
  shieldBtn.classList.toggle('is-on', on);
  shieldBtn.classList.toggle('is-off', !on);
  shieldBtn.setAttribute('aria-pressed', String(on));
  const blocked = browserSettings.blocked;
  shieldBtn.title = on
    ? `Ad blocking: on${typeof blocked === 'number' ? ` — ${blocked} blocked` : ''}`
    : 'Ad blocking: off';
  shieldBtn.setAttribute('aria-label', shieldBtn.title);
}

/* ── Downloads panel ─────────────────────────────────────────── */

/** Build the (hidden) downloads popover. */
function buildDownloadsPanel() {
  const panel = document.createElement('div');
  panel.className = 'browser-downloads-panel hidden';

  const head = document.createElement('div');
  head.className = 'browser-downloads-head';
  const title = document.createElement('span');
  title.className = 'browser-downloads-title';
  title.textContent = 'Downloads';
  const clear = document.createElement('button');
  clear.type = 'button';
  clear.className = 'browser-downloads-clear';
  clear.textContent = 'Clear finished';
  clear.addEventListener('click', () => downloadAction('', 'clear'));
  head.append(title, clear);

  const list = document.createElement('div');
  list.className = 'browser-downloads-list';

  panel.append(head, list);
  downloadsListEl = list;
  return panel;
}

function toggleDownloadsPanel() {
  if (!downloadsPanel) return;
  const hidden = downloadsPanel.classList.toggle('hidden');
  if (!hidden) void loadDownloads();
}

function closeDownloadsPanel() {
  if (downloadsPanel) downloadsPanel.classList.add('hidden');
}

/** Fetch the persisted history and ask the shell for its live list. */
async function loadDownloads() {
  try {
    const res = await apiFetch('/api/browser/downloads?limit=50');
    const rows = res?.data?.downloads;
    if (Array.isArray(rows)) {
      // Merge rather than replace: the shell may already have reported live
      // downloads this session, and opening the panel must not drop them.
      const byId = new Map(rows.map((r) => [r.id, { ...r }]));
      for (const d of downloads) {
        const existing = byId.get(d.id);
        if (!existing) byId.set(d.id, d);
        else if (isActiveDownload(d.state)) byId.set(d.id, { ...existing, ...d });
      }
      downloads = Array.from(byId.values());
    }
  } catch (_) {
    /* history is optional */
  }
  shellRaw('peakd:downloads:list');
  renderDownloads();
}

/** Merge one shell event into the live list and persist it. */
function applyDownload(event) {
  if (!event || event.type !== 'download') return;
  const d = event.download;
  if (event.kind === 'removed' || !d || !d.id) {
    if (d && d.id) {
      downloads = downloads.filter((x) => x.id !== d.id);
      renderDownloads();
    }
    return;
  }
  const idx = downloads.findIndex((x) => x.id === d.id);
  if (idx >= 0) downloads[idx] = { ...downloads[idx], ...d };
  else downloads.unshift({ ...d });
  renderDownloads();
  void apiFetch('/api/browser/downloads/event', {
    method: 'POST',
    body: JSON.stringify(d),
  }).catch(() => {});
}

/** Send a control command for one download and mirror it locally. */
function downloadAction(id, action) {
  if (action === 'clear') {
    shellSend('peakd:download:', { id: '', action: 'clear' });
    downloads = downloads.filter(
      (d) => !['completed', 'cancelled', 'interrupted'].includes(d.state),
    );
    renderDownloads();
    void apiFetch('/api/browser/downloads/clear', { method: 'POST' }).catch(() => {});
    return;
  }
  if (!id) return;
  shellSend('peakd:download:', { id, action });
  if (action === 'remove') {
    downloads = downloads.filter((d) => d.id !== id);
    renderDownloads();
    void apiFetch('/api/browser/downloads/remove', {
      method: 'POST',
      body: JSON.stringify({ id }),
    }).catch(() => {});
  }
}

/** Open a finished download in the owning app (or show Downloads in Files). */
function openDownload(d) {
  if (!d.file) return;
  const dir = d.incognito ? '.incognito' : 'Downloads';
  const rel = `${dir}/${d.file}`;
  const plugin = pluginForFile ? pluginForFile(d.file) : null;
  if (plugin && openWithPlugin) {
    void openWithPlugin({ plugin, path: rel, name: d.file });
  } else if (revealInFiles) {
    revealInFiles(dir);
  }
}

function formatBytes(n) {
  const value = Number(n) || 0;
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(0)} KB`;
  if (value < 1024 * 1024 * 1024) return `${(value / (1024 * 1024)).toFixed(1)} MB`;
  return `${(value / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

function isActiveDownload(state) {
  return state === 'downloading' || state === 'requested' || state === 'paused';
}

/** One row: name, host/state, a progress bar while active, and actions. */
function downloadRow(d) {
  const row = document.createElement('div');
  row.className = `browser-download is-${d.state || 'download'}`;

  const info = document.createElement('div');
  info.className = 'browser-download-info';
  const name = document.createElement('div');
  name.className = 'browser-download-name';
  name.textContent = d.file || d.url || 'download';
  name.title = d.path || d.url || '';
  const meta = document.createElement('div');
  meta.className = 'browser-download-meta';
  const total = Number(d.total) || 0;
  const received = Number(d.received) || 0;
  const sizeText = total ? `${formatBytes(received)} / ${formatBytes(total)}` : formatBytes(received);
  meta.textContent = [d.host, d.state, sizeText].filter(Boolean).join(' · ');
  info.append(name, meta);

  if (isActiveDownload(d.state)) {
    const bar = document.createElement('div');
    bar.className = 'browser-download-progress';
    const fill = document.createElement('div');
    fill.className = 'browser-download-fill';
    const pct = total ? Math.min(100, Math.round((received / total) * 100)) : 0;
    fill.style.width = `${pct}%`;
    bar.appendChild(fill);
    info.appendChild(bar);
  }

  const actions = document.createElement('div');
  actions.className = 'browser-download-actions';

  if (d.state === 'downloading' || d.state === 'requested' || d.state === 'paused') {
    actions.appendChild(iconButton({
      icon: d.state === 'paused' ? 'ui/play' : 'ui/pause',
      size: 'sm',
      label: d.state === 'paused' ? 'Resume' : 'Pause',
      onClick: () => downloadAction(d.id, d.state === 'paused' ? 'resume' : 'pause'),
    }));
    actions.appendChild(iconButton({
      icon: 'ui/close',
      size: 'sm',
      label: 'Cancel',
      onClick: () => downloadAction(d.id, 'cancel'),
    }));
  } else if (d.state === 'completed') {
    actions.appendChild(iconButton({
      icon: 'ui/folder-open',
      size: 'sm',
      label: 'Open',
      onClick: () => openDownload(d),
    }));
    actions.appendChild(iconButton({
      icon: 'ui/trash',
      size: 'sm',
      label: 'Remove',
      onClick: () => downloadAction(d.id, 'remove'),
    }));
  } else {
    if (d.url) {
      actions.appendChild(iconButton({
        icon: 'ui/refresh',
        size: 'sm',
        label: 'Retry',
        onClick: () => navigateTo(d.url),
      }));
    }
    actions.appendChild(iconButton({
      icon: 'ui/trash',
      size: 'sm',
      label: 'Remove',
      onClick: () => downloadAction(d.id, 'remove'),
    }));
  }

  row.append(info, actions);
  return row;
}

function renderDownloads() {
  if (!downloadsListEl) return;
  if (!downloads.length) {
    const empty = document.createElement('div');
    empty.className = 'browser-downloads-empty';
    empty.textContent = 'No downloads yet';
    downloadsListEl.replaceChildren(empty);
  } else {
    downloadsListEl.replaceChildren(...downloads.map(downloadRow));
  }
  updateDownloadsBadge();
}

/** Show how many downloads are active on the toolbar button. */
function updateDownloadsBadge() {
  if (!downloadsBtn) return;
  const active = downloads.filter((d) => isActiveDownload(d.state)).length;
  let badge = downloadsBtn.querySelector
    ? downloadsBtn.querySelector('.browser-downloads-badge')
    : null;
  if (!badge) {
    badge = document.createElement('span');
    badge.className = 'browser-downloads-badge';
    downloadsBtn.appendChild(badge);
  }
  badge.textContent = active ? String(active) : '';
  badge.classList.toggle('hidden', active === 0);
}

/* Shell → window: shield state and the live download list. */
window.__peakdShield = function (state) {
  if (!state || typeof state !== 'object') return;
  if (typeof state.enabled === 'boolean') browserSettings.adblock = state.enabled;
  if (typeof state.blocked === 'number') browserSettings.blocked = state.blocked;
  if (typeof state.rules === 'number') browserSettings.rules = state.rules;
  updateShield();
};

window.__peakdDownloads = function (list) {
  if (!Array.isArray(list)) return;
  // The shell's list is chronological (oldest first); show newest first and
  // keep any persisted rows the shell no longer tracks.
  const live = list.slice().reverse();
  const seen = new Set(live.map((d) => d.id));
  downloads = live.concat(downloads.filter((d) => !seen.has(d.id)));
  renderDownloads();
};

/* ── Home surface: related-news cards ─────────────────────────────
 *
 * `about:home` used to be a bare "type an address" panel. It is now the
 * browser's start page: a shelf of news chosen from what the user actually
 * searches for (`/api/browser/news`), with an explicit card click feeding the
 * ranking back (`/api/browser/news/click`).
 *
 * The surface is rendered inside a sandboxed `srcdoc` iframe rather than in the
 * window's own DOM. That is deliberate: the sandbox (no `allow-same-origin`)
 * means the start page cannot reach the app shell even though it is built from
 * data the server sent.
 */

/** Fetch the shelf. Never throws: an empty shelf with a reason is the fallback. */
async function fetchNews({ refresh = false } = {}) {
  const query = `?limit=12${refresh ? '&refresh=1' : ''}`;
  const res = await apiFetch(`/api/browser/news${query}`);
  return res?.data || { cards: [], topics: [], personalized: false };
}

/** The home page markup, with the shelf's data embedded. */
function homeDocument(news) {
  const cards = Array.isArray(news?.cards) ? news.cards : [];
  const topics = Array.isArray(news?.topics) ? news.topics : [];
  const payload = JSON.stringify({ cards, topics }).replace(/<\//g, '<\\/');
  const personal = news?.personalized && cards.length;

  const shelf = personal
    ? `<div class="chips">${topics
        .map((t) => `<span class="chip">${escapeHtml(t)}</span>`)
        .join('')}</div>
       <div class="head"><span class="eyebrow">Because of your recent searches</span>
         <button type="button" class="again" data-refresh>Refresh</button></div>
       <div class="cards">${cards.map(cardHtml).join('')}</div>`
    : emptyShelf(news);

  return `<!doctype html><html><head><meta charset="utf-8">
<style>
  :root { color-scheme: dark light; }
  html,body{height:100%;margin:0}
  body{background:var(--bg,#101014);color:var(--text,#ececf1);
       font-family:var(--font-body,system-ui,-apple-system,sans-serif);
       -webkit-font-smoothing:antialiased}
  .home{max-width:60rem;margin:0 auto;padding:2.2rem 1.5rem 3rem}
  .brand{display:flex;align-items:baseline;gap:.6rem;margin-bottom:.35rem}
  .brand b{font-size:1.15rem;font-weight:650;letter-spacing:-.01em}
  .brand span{font-size:.8rem;opacity:.6}
  .hint{font-size:.85rem;opacity:.65;margin:0 0 1.6rem}
  .chips{display:flex;flex-wrap:wrap;gap:.4rem;margin-bottom:1.1rem}
  .chip{font-size:.72rem;opacity:.8;padding:.22rem .6rem;border-radius:999px;
        border:1px solid color-mix(in srgb, currentColor 22%, transparent)}
  .head{display:flex;align-items:center;justify-content:space-between;gap:1rem;margin-bottom:.75rem}
  .eyebrow{font-size:.7rem;letter-spacing:.08em;text-transform:uppercase;opacity:.55}
  .again{cursor:pointer;font:inherit;font-size:.72rem;padding:.25rem .6rem;border-radius:8px;
         border:1px solid color-mix(in srgb, currentColor 22%, transparent);
         background:transparent;color:inherit;opacity:.8}
  .again:hover{opacity:1}
  .cards{display:grid;gap:.6rem;grid-template-columns:repeat(auto-fill,minmax(17rem,1fr))}
  a.card{display:flex;flex-direction:row;gap:.7rem;padding:.7rem;border-radius:14px;
         text-decoration:none;color:inherit;
         border:1px solid color-mix(in srgb, currentColor 14%, transparent);
         background:color-mix(in srgb, currentColor 4%, transparent);
         transition:border-color .15s ease, background .15s ease, transform .15s ease}
  a.card:hover{border-color:var(--accent,#7aa2ff);
               background:color-mix(in srgb, currentColor 8%, transparent);transform:translateY(-1px)}
  .thumb{flex:none;width:88px;height:88px;border-radius:10px;object-fit:cover;
         background:color-mix(in srgb, currentColor 8%, transparent)}
  .body{display:flex;flex-direction:column;gap:.4rem;min-width:0;flex:1 1 auto}
  .meta{display:flex;align-items:center;gap:.4rem;font-size:.68rem;opacity:.6;min-width:0}
  .meta b{font-weight:600;opacity:.95;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
  .meta i{font-style:normal;opacity:.7;white-space:nowrap}
  .topic{margin-left:auto;opacity:.75;white-space:nowrap}
  .title{font-size:.88rem;font-weight:600;line-height:1.3;
         display:-webkit-box;-webkit-line-clamp:3;-webkit-box-orient:vertical;overflow:hidden}
  .snip{font-size:.78rem;line-height:1.45;opacity:.7;
        display:-webkit-box;-webkit-line-clamp:3;-webkit-box-orient:vertical;overflow:hidden}
  .empty{max-width:30rem;margin:3rem auto 0;text-align:center}
  .empty .t{font-size:1.05rem;font-weight:600;margin-bottom:.4rem}
  .empty .s{font-size:.85rem;line-height:1.55;opacity:.7}
</style></head><body>
<div class="home">
  <div class="brand"><b>Browser</b><span>filtered by the ad blocker</span></div>
  <p class="hint">Type an address or a search above. Pages load through the ad blocker.</p>
  <div id="shelf">${shelf}</div>
</div>
<scr${'ipt'} id="home-data" type="application/json">${payload}</scr${'ipt'}>
<scr${'ipt'}>
(function () {
  var data = { cards: [], topics: [] };
  try { data = JSON.parse(document.getElementById('home-data').textContent || '{}'); } catch (e) {}
  if (!data.cards.length) data.cards = [];

  // A card navigates by asking the parent, which is the only frame allowed to
  // touch the proxy/session state. The sandbox gives this document no
  // same-origin access, so postMessage is the only channel — and it is enough.
  document.addEventListener('click', function (event) {
    var a = event.target && event.target.closest ? event.target.closest('a.card') : null;
    if (a) {
      event.preventDefault();
      var index = parseInt(a.getAttribute('data-index'), 10);
      var card = data.cards[index];
      if (!card) return;
      try {
        parent.postMessage({ type: 'browser:open-card', url: card.url, topic: card.topic }, '*');
      } catch (e) {}
      return;
    }
    var again = event.target && event.target.closest ? event.target.closest('[data-refresh]') : null;
    if (again) {
      again.disabled = true;
      again.textContent = 'Refreshing…';
      try { parent.postMessage({ type: 'browser:refresh-news' }, '*'); } catch (e) {}
    }
  });
})();
</scr${'ipt'}>
</body></html>`;
}

/** One card. Kept as a string because it is injected into the sandboxed frame. */
function cardHtml(card, index) {
  const meta = [
    `<b>${escapeHtml(card.source || '')}</b>`,
    card.age ? `<i>${escapeHtml(card.age)}</i>` : '',
    card.topic ? `<span class="topic">${escapeHtml(card.topic)}</span>` : '',
  ]
    .filter(Boolean)
    .join('');
  const snippet = card.snippet ? `<div class="snip">${escapeHtml(card.snippet)}</div>` : '';
  // The engine's thumbnail is loaded straight from its CDN. `no-referrer` keeps
  // the app's origin out of the request; the image is decorative (the title
  // carries the meaning), so a failed load must not show a broken glyph.
  const thumb = card.image
    ? `<img class="thumb" src="${escapeHtml(card.image)}" alt="" loading="lazy" referrerpolicy="no-referrer">`
    : '';
  return `<a class="card" href="#" data-index="${index}" title="${escapeHtml(card.title || '')}">
    ${thumb}
    <span class="body">
      <span class="meta">${meta}</span>
      <span class="title">${escapeHtml(card.title || '')}</span>
      ${snippet}
    </span>
  </a>`;
}

/** What to show when there is nothing to personalise from. */
function emptyShelf(news) {
  const reason = news?.error ? escapeHtml(news.error) : '';
  const body = news?.personalized === false && !reason
    ? 'Search for a few things and this page fills up with news about them.'
    : reason || 'Nothing to show yet — try refreshing.';
  return `<div class="empty">
    <div class="t">No stories yet</div>
    <div class="s">${body}</div>
  </div>`;
}

function escapeHtml(value) {
  return String(value ?? '')
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

/** The idle surface for one tab: the start page, with related news. */
async function showHome(tab) {
  if (!tab) return;
  hideNative(tab);
  tab.home = true;
  tab.url = '';
  tab.history = [];
  tab.historyIndex = -1;
  if (tab.id === activeTabId) {
    if (addressEl) addressEl.input.value = '';
    setStatus('Loading recommendations…');
  }
  // The idle surface must not be same-origin with the app shell (it is
  // rendered from server data), so it is sandboxed without allow-same-origin.
  tab.frameEl.removeAttribute('allow');
  tab.frameEl.setAttribute('sandbox', 'allow-scripts');
  tab.frameEl.removeAttribute('srcdoc');
  // Clear the previous page *before* awaiting the shelf, so going Home is
  // instant instead of leaving the last site on screen while it loads.
  tab.frameEl.src = 'about:blank';

  let news = { cards: [], topics: [], personalized: false };
  try {
    news = await fetchNews();
  } catch (_) {
    news = { cards: [], topics: [], personalized: false, error: 'Could not load recommendations' };
  }
  // Re-check: the user may have navigated this tab while the shelf was in
  // flight.
  if (!tab.frameEl || !tab.home) return;
  if (tab.id === activeTabId) setStatus('');
  tab.frameEl.src = 'about:blank';
  tab.frameEl.srcdoc = homeDocument(news);
  renderTabs();
  if (tab.id === activeTabId) updateNavButtons();
}

/** A card in the home shelf was clicked. */
function onHomeCard(tab, url, topic) {
  if (!url || !tab) return;
  // Best-effort: the click is a ranking signal, not a precondition for
  // navigating. A failed POST must not stop the page from opening.
  void apiFetch('/api/browser/news/click', {
    method: 'POST',
    body: JSON.stringify({ url, topic }),
  }).catch(() => {});
  void navigateTo(url, { tab });
}

/** Open a URL in a fresh tab. */
function openInNewTab(_from, url) {
  if (!url) return;
  if (tabs.length >= MAX_TABS) {
    // At the cap, reuse the active tab rather than silently dropping the click.
    void navigateTo(url, { tab: activeTab() });
    return;
  }
  createTab({ url });
}

/* ── Link preview ─────────────────────────────────────────────── */

/**
 * The native view owns link interaction.
 *
 * The old proxied design injected a shim into every page so this window could
 * draw its own link context menu and hover preview. A native child webview runs
 * the page's own document, so those are the page's/WebKit's to handle — there
 * is nothing to forward here.
 */

/**
 * Messages from the framed surfaces.
 *
 * Only the home shelf talks back now: `event.source` is its `WindowProxy`, not
 * the `<iframe>` element, so the guard matches it against each tab's
 * `contentWindow` — with more than one tab open, the message must update the tab
 * that sent it, never the active one.
 */
function onFrameMessage(event) {
  const data = event.data;
  if (!data || typeof data !== 'object') return;
  const tab = tabs.find((t) => t.frameEl && t.frameEl.contentWindow === event.source);
  if (!tab) return;

  if (data.type === 'browser:open-card') {
    void onHomeCard(tab, String(data.url || ''), data.topic ? String(data.topic) : undefined);
  } else if (data.type === 'browser:refresh-news') {
    void showHome(tab);
  }
}

/**
 * Events from the shell about a native view: the child webview's URL, title and
 * load phase. The shell calls this on the main webview, naming the tab by id.
 *
 * Defined at module scope because the shell may call it before `wireEvents`
 * runs (a navigation can complete while the window is still mounting).
 */
window.__peakdViewEvent = function (event) {
  if (!event || typeof event !== 'object') return;
  // Download events are not tab-scoped; handle them before the tab lookup.
  if (event.type === 'download') {
    applyDownload(event);
    return;
  }
  const tab = tabs.find((t) => t.id === event.id);
  if (!tab) return;
  if (event.type === 'title' && typeof event.title === 'string' && event.title) {
    tab.title = event.title;
    renderTabs();
  } else if (event.type === 'url' && typeof event.url === 'string' && event.url) {
    recordLocation(tab, event.url);
  } else if (event.type === 'load') {
    if (event.phase === 'finished') {
      tab.loaded = true;
      clearWatchdog(tab);
      if (tab.id === activeTabId) setStatus('');
    } else if (event.phase === 'started' && tab.id === activeTabId) {
      setStatus('Loading…');
    }
  } else if (event.type === 'new-window' && typeof event.url === 'string' && event.url) {
    // A `target="_blank"` link in a native view: the shell refused to hand a
    // popup to the OS browser, so open it as a tab here.
    openInNewTab(tab, event.url);
  }
};

/* ── Native-view bounds sync ──────────────────────────────────── */

let boundsTimer = null;
let boundsObserver = null;
let tileObserver = null;
let bodyObserver = null;
let syncQueued = false;

/**
 * Coalesce a burst of layout changes into one sync per frame.
 *
 * Dragging or resizing a window writes inline styles many times per frame; the
 * native view only needs the latest value, and reading it once per frame keeps
 * the page from chasing the shell.
 */
function requestSync() {
  if (syncQueued) return;
  if (typeof requestAnimationFrame !== 'function') {
    syncActiveNative();
    return;
  }
  syncQueued = true;
  requestAnimationFrame(() => {
    syncQueued = false;
    syncActiveNative();
  });
}

function startBoundsSync() {
  stopBoundsSync();
  // A native child window is positioned in native pixels, so it must follow the
  // viewport element every frame it moves. A `ResizeObserver` covers resizes and
  // a `MutationObserver` covers the inline style the drag/resize code writes;
  // the slow interval is a fallback for anything else (a CSS animation, say).
  // Only the active tab is visible, so only it is tracked.
  boundsTimer = setInterval(syncActiveNative, 250);
  if (typeof ResizeObserver !== 'undefined' && frameWrapEl) {
    boundsObserver = new ResizeObserver(requestSync);
    boundsObserver.observe(frameWrapEl);
  }
  if (typeof MutationObserver !== 'undefined') {
    const scope = document.getElementById('tile-grid') || tileEl;
    if (scope) {
      // Watch the whole grid: another window moving over—or away from—this one
      // changes whether the native page may stay on top.
      tileObserver = new MutationObserver(requestSync);
      tileObserver.observe(scope, {
        attributes: true,
        attributeFilter: ['class', 'style'],
        subtree: true,
      });
    }
    if (document.body) {
      bodyObserver = new MutationObserver(requestSync);
      bodyObserver.observe(document.body, { attributes: true, attributeFilter: ['class'] });
    }
  }
  window.addEventListener('resize', requestSync);
  window.addEventListener('desktop:changed', requestSync);
  window.addEventListener('overlay:open', requestSync);
  window.addEventListener('fullscreen:change', requestSync);
}

function stopBoundsSync() {
  if (boundsTimer) clearInterval(boundsTimer);
  boundsTimer = null;
  if (boundsObserver) boundsObserver.disconnect();
  boundsObserver = null;
  if (tileObserver) tileObserver.disconnect();
  tileObserver = null;
  if (bodyObserver) bodyObserver.disconnect();
  bodyObserver = null;
  window.removeEventListener('resize', requestSync);
  window.removeEventListener('desktop:changed', requestSync);
  window.removeEventListener('overlay:open', requestSync);
  window.removeEventListener('fullscreen:change', requestSync);
}

/* ── AI-driven navigation ─────────────────────────────────────── */

/**
 * The AI asked for a page: load it in the active tab.
 *
 * `narrative` is where the browser tools put the URL (see `page_artifact`).
 * The other two sources are kept because cards saved before that change exist:
 * `payload.url` was never written by anything, and a card's `title` *is* the
 * URL for a browser page — which is exactly the kind of coincidence a fix
 * should not quietly start depending on.
 */
function onArtifactSaved(event) {
  const art = event?.detail;
  const type = art?.artifact_type || art?.type;
  if (!art || type !== 'browser_page') return;
  const url =
    art.narrative || art?.payload?.url || art?.params?.url || art?.payload?.view_url || art?.title;
  if (typeof url === 'string' && /^https?:\/\//.test(url)) {
    const tab = activeTab();
    if (tab) {
      tab.home = false;
      void navigateTo(url, { tab });
    } else {
      createTab({ url });
    }
  }
}

/** Entries core splices into this window's right-click menu (PLUGINS.md §19). */
export function browserContextMenu() {
  return [
    {
      type: 'item',
      label: 'New tab',
      icon: 'ui/plus',
      onClick: () => createTab({}),
    },
    {
      type: 'item',
      label: 'New incognito tab',
      icon: 'ui/incognito',
      onClick: () => createTab({ incognito: true }),
    },
    {
      type: 'item',
      label: 'Downloads',
      icon: 'ui/download',
      onClick: () => toggleDownloadsPanel(),
    },
    {
      type: 'item',
      label: 'Focus address bar',
      icon: 'ui/search',
      disabled: !addressEl,
      onClick: () => addressEl?.input?.focus(),
    },
    {
      type: 'item',
      label: 'Reload',
      icon: 'ui/loop',
      disabled: !frameWrapEl,
      onClick: () => reload(),
    },
    {
      type: 'item',
      label: 'Home',
      icon: 'ui/launcher',
      disabled: !tileEl,
      onClick: () => void showHome(activeTab()),
    },
  ];
}

/* ── Tile lifecycle ───────────────────────────────────────────── */

export function mountBrowserTile() {
  if (tileEl) return tileEl;
  ensureStylesheet();

  tileEl = document.createElement('section');
  tileEl.className = 'tile browser-tile';
  tileEl.dataset.plugin = BROWSER_PLUGIN;

  tabstripEl = buildTabStrip();
  viewportEl = buildViewport();
  downloadsPanel = buildDownloadsPanel();
  tileEl.append(tabstripEl, buildToolbar(), viewportEl, downloadsPanel);

  updateNavButtons();
  updateShield();
  startBoundsSync();
  // One tab, on its home surface.
  createTab({});
  // Settings (the shield + where downloads go) and the persisted history.
  void loadSettings();
  void loadDownloads();
  return tileEl;
}

export function unmountBrowserTile() {
  stopBoundsSync();
  window.removeEventListener('message', onFrameMessage);
  wired = false;
  for (const tab of tabs) destroyTab(tab);
  tabs = [];
  activeTabId = null;
  downloads = [];
  tileEl?.remove();
  tileEl = null;
  tabstripEl = null;
  frameWrapEl = null;
  viewportEl = null;
  addressEl = null;
  backBtn = null;
  forwardBtn = null;
  statusEl = null;
  shieldBtn = null;
  downloadsBtn = null;
  incognitoBtn = null;
  downloadsPanel = null;
  downloadsListEl = null;
}

export function getBrowserTileElement() {
  return tileEl;
}

export function wireBrowserEvents() {
  if (wired) return;
  wired = true;
  window.addEventListener('artifact:saved', onArtifactSaved);
  // The home shelf talks back through postMessage (card clicks, refresh).
  window.addEventListener('message', onFrameMessage);
}

export default {
  name: 'browser',
  icon: 'ui/launcher',
  mount: mountBrowserTile,
  unmount: unmountBrowserTile,
  getElement: getBrowserTileElement,
  wireEvents: wireBrowserEvents,
  contextMenu: browserContextMenu,
};
