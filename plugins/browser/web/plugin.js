import {
  toolbar, controlButton, icon, iconButton, searchBar, themeMode, listReveal,
} from '../../ui/index.js';
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
import { copyText, readClipboardText } from '../../js/clipboard.js';
import { openContextMenu } from '../../js/contextMenu.js';
import { openWithPlugin, pluginForFile, revealInFiles } from '../../js/files.js';
import { t, tp } from '../../js/i18n.js';
import { onOpenUrl } from '../../js/linkRouter.js';
import { nativeOccluderHoles, watchNativeOccluders } from '../../js/nativeOccluders.js';
import { createPasswordsPanel, fillScript, readScript } from './passwords.js';
import { openCoreWindow } from '../../js/tiles.js';

export const BROWSER_PLUGIN = 'browser';

/** How many tabs may be open. Each is a live web view, so this is bounded. */
const MAX_TABS = 8;

/**
 * The search engines the window offers. The id is what the server stores and
 * maps to a URL (see `plugins/browser/src/settings.rs`); DuckDuckGo is the
 * default. The list is duplicated here only for the menu's labels — the server
 * is the single source of truth for where a search actually goes.
 */
const SEARCH_ENGINES = [
  { id: 'duckduckgo', label: 'DuckDuckGo' },
  { id: 'brave', label: 'Brave Search' },
  { id: 'google', label: 'Google' },
  { id: 'bing', label: 'Bing' },
];

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
 * Whether the active page should currently be on screen.
 *
 * The page is an item in the shell's own scene (see `crates/peakd`), composited
 * by the same renderer as the app's DOM: it follows its window frame for frame
 * and never has to be dropped while it moves. Only the states the app draws
 * over everything — a hidden tile (a workspace switch), the overview, the
 * launcher, a workspace slide in flight — hide it.
 *
 * Overlapping HTML (open menus, a higher floating window in the "Windows"
 * layout) is not a reason to hide it either: the shell cuts the page where the
 * overlay is (see `occluderHoles`), so the page stays visible and live
 * everywhere the overlay does not cover it.
 */
function nativeShouldShow(tab) {
  if (!tab || !tab.native || !tileEl) return false;
  if (tileEl.classList.contains('hidden')) return false;
  const body = document.body;
  if (body && body.classList) {
    // The overview and launcher are full-screen HTML layers over the desktop;
    // the page cannot be composited beneath them. A workspace slide moves the
    // tiles with DOM transforms the native item cannot follow.
    if (body.classList.contains('overview-active')) return false;
    if (body.classList.contains('launcher-active')) return false;
    if (body.classList.contains('workspace-sliding')) return false;
  }
  return true;
}

/**
 * The CSS rects the page must show *through* — everything the HTML draws over
 * an otherwise always-on-top native view. Core chrome comes from
 * `nativeOccluderHoles` (chat panel, voice bar/dock, top bar, keyboard,
 * navigation, travel panel, tile overlay and shared popups); this window adds
 * its own popovers and, in the free "Windows" layout, any higher floating
 * window that overlaps it.
 *
 * Each rect is in the same client-pixel space as `viewportRect`, so the shell
 * can subtract them directly from the view's geometry.
 */
function occluderHoles() {
  const rect = viewportRect();
  if (!rect) return [];
  const holes = nativeOccluderHoles(rect);
  const add = (r) => {
    if (!r || r.width < 1 || r.height < 1) return;
    // Only a rect that actually covers part of the page is a hole.
    if (r.right <= rect.x || r.left >= rect.x + rect.w) return;
    if (r.bottom <= rect.y || r.top >= rect.y + rect.h) return;
    holes.push({ x: r.left, y: r.top, w: r.width, h: r.height, dpr: rect.dpr });
  };

  // This window's own popovers (downloads / menu / history / bookmarks / tabs
  // / passwords). They live inside the tile, but the native view still stacks
  // above them.
  for (const el of browserPopovers()) {
    if (!el.classList.contains('hidden')) add(el.getBoundingClientRect());
  }
  // A higher floating window overlapping this one, in the "Windows" layout.
  // Compare by z-index, falling back to DOM order when equal (windows that
  // have never been raised share a z, and the later sibling is painted on top).
  const grid = document.getElementById('tile-grid');
  if (grid && grid.dataset.layout === 'windows') {
    const z = Number(tileEl.style.zIndex) || 0;
    const tiles = [...grid.querySelectorAll(':scope > .tile[data-plugin]')];
    const myIndex = tiles.indexOf(tileEl);
    tiles.forEach((el, index) => {
      if (el === tileEl || el.classList.contains('hidden')) return;
      const ez = Number(el.style.zIndex) || 0;
      const above = ez > z || (ez === z && index > myIndex);
      if (above) add(el.getBoundingClientRect());
    });
  }
  return holes;
}

/** Round a rect to whole logical pixels, the way the shell keys updates. */
function rectKey(r) {
  return `${Math.round(r.x * r.dpr)},${Math.round(r.y * r.dpr)},`
    + `${Math.round(r.w * r.dpr)},${Math.round(r.h * r.dpr)}`;
}

function holeKey(h) {
  return `${Math.round(h.x * h.dpr)},${Math.round(h.y * h.dpr)},`
    + `${Math.round(h.w * h.dpr)},${Math.round(h.h * h.dpr)}`;
}

/** Push bounds, clip and visibility for one tab, skipping no-op updates. */
function syncNativeTab(tab) {
  if (!nativeAvailable || !tab || !tab.native) return;
  const show = nativeShouldShow(tab);
  if (show !== lastNativeVisible) {
    lastNativeVisible = show;
    nativeSend({ op: 'setVisible', id: tab.id, visible: show });
  }
  if (!show) {
    lastNativeRect = '';
    lastNativeHoles = null;
    return;
  }
  const rect = viewportRect();
  if (!rect) return;
  const key = rectKey(rect);
  if (key !== lastNativeRect) {
    lastNativeRect = key;
    nativeSend({ op: 'setBounds', id: tab.id, rect });
  }
  const holes = occluderHoles();
  const holesKey = holes.map(holeKey).join(';');
  if (holesKey !== lastNativeHoles) {
    lastNativeHoles = holesKey;
    nativeSend({ op: 'setMask', id: tab.id, holes });
  }
}

/** Keep the shell's native view aligned with the viewport element. */
function syncActiveNative() {
  if (document.hidden) return;
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
let bookmarkBtn = null;
let passwordsBtn = null;
let menuBtn = null;
let downloadsPanel = null;
let downloadsListEl = null;
let browserMenu = null;
let historyPanel = null;
let historyListEl = null;
let bookmarksPanel = null;
let bookmarksListEl = null;
let tabsPanel = null;
let tabsListEl = null;
let passwordsPanel = null;
/** The password manager behind `passwordsPanel` (see passwords.js). */
let passwords = null;

/** Every popover this window owns, open or not. */
function browserPopovers() {
  return [downloadsPanel, browserMenu, historyPanel, bookmarksPanel, tabsPanel, passwordsPanel]
    .filter(Boolean);
}

/** The persisted history the panels show (newest first). */
let history = [];
/** The user's saved pages, newest first. */
let bookmarks = [];

/**
 * The window's server-backed settings. `adblock` drives the shield toggle and
 * `downloadsDir` is the absolute directory the shell saves downloads into (the
 * user's `Downloads` folder, resolved server-side per account).
 */
let browserSettings = {
  adblock: true,
  incognito: false,
  downloadsDir: '',
  searchEngine: 'duckduckgo',
};

/** Live downloads, newest first, keyed by the shell's id. */
let downloads = [];

// Last state pushed for the active native view, so the per-frame sync can skip
// updates the shell already has. `null`/`''` means "unknown, send it".
let lastNativeVisible = null;
let lastNativeRect = '';
// `null` means "unknown" and `''` means "no holes". The shell keeps the last
// mask it was given, so after the view is hidden (a drag, a tab switch) the
// mask must be re-sent even when it is now empty — otherwise a stale hole from
// before the hide clips the page once the view moves.
let lastNativeHoles = null;

/**
 * The window's tabs. Each owns a native child webview in the shell (and a local
 * iframe only for its home surface), a server session id and its own navigation
 * history; only the active one is visible. `activeTabId` always names one of
 * them while the window is mounted.
 */
let tabs = [];
let activeTabId = null;
let tabSeq = 0;
/** A URL handed over by the core link router before the window mounted.
 *  The module is imported (and wired) a moment before the tile exists, and
 *  `createTab` needs the frame wrapper — so the open waits for mount instead
 *  of being dropped. */
let pendingCoreOpen = null;

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
  return t('plugin.browser.tabs.newTab', null, 'New tab');
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
    // history and cookies) but hidden — and keep playing when they are playing
    // audio, because the shell freezes a hidden page only where the engine
    // says it may. The active one goes through the shared sync so window
    // occlusion/overlays are honoured too.
    if (nativeAvailable && t.id !== id) {
      nativeSend({ op: 'setVisible', id: t.id, visible: false });
    }
  }
  if (addressEl && document.activeElement !== addressEl.input) {
    addressEl.input.value = tab?.url || '';
  }
  lastNativeVisible = null;
  lastNativeRect = '';
  lastNativeHoles = null;
  syncActiveNative();
  if (nativeAvailable && tab?.native) nativeSend({ op: 'focus', id: tab.id });
  renderTabs();
  updateNavButtons();
  updateBookmarkButton();
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

    const label = controlButton();
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
    const tabName = tab.url || t('plugin.browser.tabs.newTab', null, 'New tab');
    label.title = tab.incognito
      ? t('plugin.browser.tabs.incognito', { url: tabName }, `Incognito — ${tabName}`)
      : tabName;
    label.addEventListener('click', () => setActiveTab(tab.id));

    const close = controlButton();
    close.type = 'button';
    close.className = 'browser-tab-close';
    close.setAttribute(
      'aria-label',
      t('plugin.browser.tabs.close', { title: tabTitle(tab) }, `Close ${tabTitle(tab)}`),
    );
    close.appendChild(icon('ui/close', { size: 12 }));
    close.addEventListener('click', (e) => {
      e.stopPropagation();
      closeTab(tab.id);
    });

    el.append(label, close);
    nodes.push(el);
  }
  // The tab switcher's trigger: the open-tab count, as phone browsers show it.
  // CSS only reveals it in a narrow window, where the strip keeps just the
  // active tab (see `.browser-tabcount` in plugin.css).
  const count = controlButton();
  count.type = 'button';
  count.className = 'browser-tabcount';
  count.textContent = String(tabs.length);
  const countLabel = tp('plugin.browser.tabs.count', tabs.length, null, `${tabs.length} open tabs`);
  count.title = countLabel;
  count.setAttribute('aria-label', countLabel);
  count.setAttribute('aria-haspopup', 'dialog');
  count.addEventListener('click', () => toggleTabsPanel());
  nodes.push(count);

  const add = iconButton({
    icon: 'ui/plus',
    size: 'sm',
    label: t('plugin.browser.tabs.newTab', null, 'New tab'),
    onClick: () => createTab({}),
  });
  add.classList.add('browser-newtab');
  nodes.push(add);
  tabstripEl.replaceChildren(...nodes);
  // A strip wider than the window scrolls; keep the active tab on screen.
  tabstripEl.querySelector('.browser-tab.is-active')
    ?.scrollIntoView?.({ block: 'nearest', inline: 'nearest' });
  if (tabsPanel && !tabsPanel.classList.contains('hidden')) renderTabsPanel();
}

/* ── Tab switcher ─────────────────────────────────────────────── */

/**
 * The full list of tabs as large rows: the narrow-window (phone) way to see,
 * switch and close tabs, since the strip then shows only the active one.
 */
function buildTabsPanel() {
  const panel = popover('browser-tabs-panel');
  panel.setAttribute('role', 'dialog');
  panel.setAttribute('aria-label', t('plugin.browser.tabs.label', null, 'Tabs'));

  const head = document.createElement('div');
  head.className = 'browser-popover-head';
  head.append(labelSpan(t('plugin.browser.tabs.label', null, 'Tabs')));
  const incognito = controlButton();
  incognito.type = 'button';
  incognito.className = 'browser-popover-action';
  incognito.textContent = t('plugin.browser.tabs.newIncognito', null, 'New incognito tab');
  incognito.addEventListener('click', () => {
    closeAllBrowserPanels();
    createTab({ incognito: true });
  });
  const fresh = controlButton();
  fresh.type = 'button';
  fresh.className = 'browser-popover-action';
  fresh.textContent = t('plugin.browser.tabs.newTab', null, 'New tab');
  fresh.addEventListener('click', () => {
    closeAllBrowserPanels();
    createTab({});
  });
  head.append(incognito, fresh);

  const list = document.createElement('div');
  list.className = 'browser-popover-list';
  list.setAttribute('role', 'listbox');
  tabsListEl = list;

  panel.append(head, list);
  return panel;
}

function renderTabsPanel() {
  if (!tabsListEl) return;
  tabsListEl.replaceChildren(...tabs.map((tab) => {
    const row = document.createElement('div');
    row.className = 'browser-tabs-row'
      + (tab.id === activeTabId ? ' is-active' : '')
      + (tab.incognito ? ' is-incognito' : '');
    row.setAttribute('role', 'option');
    row.setAttribute('aria-selected', String(tab.id === activeTabId));

    const pick = controlButton();
    pick.type = 'button';
    pick.className = 'browser-popover-row';
    if (tab.incognito) {
      const mask = icon('ui/incognito', { size: 14 });
      mask.classList.add('browser-tab-mask');
      pick.appendChild(mask);
    }
    const main = document.createElement('div');
    main.className = 'browser-popover-main';
    const title = document.createElement('div');
    title.className = 'browser-popover-title';
    title.textContent = tabTitle(tab);
    const sub = document.createElement('div');
    sub.className = 'browser-popover-sub';
    sub.textContent = tab.url || t('plugin.browser.tabs.newTab', null, 'New tab');
    main.append(title, sub);
    pick.appendChild(main);
    pick.addEventListener('click', () => {
      closeAllBrowserPanels();
      setActiveTab(tab.id);
    });

    const close = controlButton();
    close.type = 'button';
    close.className = 'browser-tab-close';
    close.setAttribute(
      'aria-label',
      t('plugin.browser.tabs.close', { title: tabTitle(tab) }, `Close ${tabTitle(tab)}`),
    );
    close.appendChild(icon('ui/close', { size: 14 }));
    close.addEventListener('click', (e) => {
      e.stopPropagation();
      closeTab(tab.id);
    });

    row.append(pick, close);
    return row;
  }));
}

function toggleTabsPanel() {
  if (!tabsPanel) return;
  const willOpen = tabsPanel.classList.contains('hidden');
  closeAllBrowserPanels();
  if (willOpen) {
    renderTabsPanel();
    tabsPanel.classList.remove('hidden');
    requestSync();
  }
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
    delete tab.frameEl.dataset.home;
    tab.frameEl.style.colorScheme = '';
  }
  tab.homeData = null;
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
  lastNativeHoles = null;
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
    lastNativeHoles = null;
  }
  if (tab.frameEl) {
    tab.frameEl.classList.remove('is-native-hidden');
    tab.frameEl.classList.toggle('is-active', tab.id === activeTabId);
  }
}

async function navigateTo(input, { tab = activeTab() } = {}) {
  const value = String(input || '').trim();
  if (!value || !tab) return;

  if (tab.id === activeTabId) setStatus(t('plugin.browser.status.loading', null, 'Loading…'));
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
    if (tab.id === activeTabId) {
      setStatus(t('plugin.browser.status.openFailed', null, 'Could not open that page'));
    }
    return;
  }
  if (!data?.url) {
    if (tab.id === activeTabId) {
      setStatus(t('plugin.browser.status.openFailed', null, 'Could not open that page'));
    }
    return;
  }

  tab.sessionId = data.session?.id || tab.sessionId;
  tab.home = false;
  recordLocation(tab, data.url || value);
  // The server already recorded this navigation (with the search phrase); mark
  // it so the following load does not log a duplicate "visit".
  tab.lastHistoryUrl = data.url || value;
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
  strip.setAttribute('aria-label', t('plugin.browser.tabs.label', null, 'Tabs'));
  // Nominate the strip as the window's top bar: the shared chrome folds it into
  // the header row next to the close/fullscreen controls (see `data-window-bar`
  // in web/js/tiles.js), so the tabs and the new-tab `+` live in the title bar
  // and the centered "Browser" label is dropped.
  strip.dataset.windowBar = '';
  return strip;
}

function buildToolbar() {
  const bar = toolbar([], { className: 'browser-bar' });

  // Back and forward are the same glyph, mirrored. The theme ships
  // `ui/arrow-left` but its `ui/forward` is a curved "share" arrow, so the pair
  // looked like two unrelated features; `browser-nav--forward` flips the one
  // arrow in CSS so both are literally the same drawing (PLUGINS.md §19: window
  // icons come from the theme, and a plugin ships no icon assets).
  backBtn = iconButton({
    icon: 'ui/arrow-left',
    size: 'sm',
    label: t('plugin.browser.toolbar.back', null, 'Back'),
    onClick: () => go(-1),
  });
  forwardBtn = iconButton({
    icon: 'ui/arrow-left',
    size: 'sm',
    label: t('plugin.browser.toolbar.forward', null, 'Forward'),
    onClick: () => go(1),
  });
  backBtn.classList.add('browser-nav', 'browser-nav--back');
  forwardBtn.classList.add('browser-nav', 'browser-nav--forward');

  const reloadBtn = iconButton({
    icon: 'ui/refresh',
    size: 'sm',
    label: t('plugin.browser.toolbar.reload', null, 'Reload'),
    onClick: () => reload(),
  });
  const homeBtn = iconButton({
    icon: 'ui/launcher',
    size: 'sm',
    label: t('plugin.browser.toolbar.home', null, 'Home'),
    onClick: () => goHome(),
  });

  // The address bar is the library's search field — the same element the
  // YouTube and Radio windows use — because that is what this control is: a
  // search box that also accepts a URL.
  addressEl = document.createElement('form');
  addressEl.className = 'browser-address';
  const search = searchBar({
    placeholder: t('plugin.browser.toolbar.address', null, 'Search or enter an address'),
  });
  const input = search.input;
  // `searchBar` is a div, so the browser's own submit gesture needs wiring:
  // Enter in a form input fires `submit`, which we intercept.
  input.setAttribute('autocomplete', 'off');
  input.setAttribute('spellcheck', 'false');
  input.setAttribute(
    'aria-label',
    t('plugin.browser.toolbar.address', null, 'Search or enter an address'),
  );
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
    label: t('plugin.browser.shield.label', null, 'Ad blocking'),
    onClick: () => toggleShield(),
  });
  shieldBtn.classList.add('browser-shield');

  downloadsBtn = iconButton({
    icon: 'ui/download',
    size: 'sm',
    label: t('plugin.browser.downloads.title', null, 'Downloads'),
    onClick: () => toggleDownloadsPanel(),
  });
  downloadsBtn.classList.add('browser-downloads-btn');

  incognitoBtn = iconButton({
    icon: 'ui/incognito',
    size: 'sm',
    label: t('plugin.browser.tabs.newIncognito', null, 'New incognito tab'),
    onClick: () => createTab({ incognito: true }),
  });
  incognitoBtn.classList.add('browser-incognito');

  // Bookmark the page in the active tab, and the overflow menu (search engine,
  // history, bookmarks, settings).
  bookmarkBtn = iconButton({
    icon: 'ui/star',
    size: 'sm',
    label: t('plugin.browser.bookmarks.add', null, 'Bookmark this page'),
    onClick: () => toggleBookmark(),
  });
  bookmarkBtn.classList.add('browser-bookmark');

  // The password manager: saved logins for this page, filled on a click.
  passwordsBtn = iconButton({
    icon: 'ui/lock',
    size: 'sm',
    label: t('plugin.browser.passwords.title', null, 'Passwords'),
    onClick: () => togglePasswordsPanel(),
  });
  passwordsBtn.classList.add('browser-passwords-btn');

  menuBtn = iconButton({
    icon: 'ui/more',
    size: 'sm',
    label: t('plugin.browser.menu.title', null, 'Browser menu'),
    onClick: () => toggleBrowserMenu(),
  });
  menuBtn.classList.add('browser-menu-btn');

  bar.append(
    backBtn, forwardBtn, reloadBtn, homeBtn, addressEl,
    bookmarkBtn, passwordsBtn, shieldBtn, downloadsBtn, incognitoBtn, menuBtn,
  );
  return bar;
}

function buildViewport() {
  const wrap = document.createElement('div');
  wrap.className = 'browser-viewport';

  frameWrapEl = document.createElement('div');
  frameWrapEl.className = 'browser-frames';
  wrap.appendChild(frameWrapEl);

  statusEl = document.createElement('div');
  statusEl.className = 'browser-status hud-menu hidden';
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
  if (tab.id === activeTabId) {
    updateNavButtons();
    updateBookmarkButton();
  }
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
    browserSettings.searchEngine = d.search_engine || 'duckduckgo';
  } catch (_) {
    /* defaults stay in place */
  }
  pushSettings();
  updateShield();
  updateEngineChecks();
}

/**
 * Tell the shell the current settings: ad blocking, where downloads go, and
 * which colour scheme pages should render for. The last one is what makes a
 * site's own dark/light styling follow the app's Ink/Dusk theme.
 */
function pushSettings() {
  shellSend('peakd:settings:', {
    adblock: browserSettings.adblock,
    downloadsDir: browserSettings.downloadsDir,
    colorScheme: themeMode() === 'light' ? 'light' : 'dark',
  });
}

/** Flip ad blocking, optimistically, and persist it. */
function toggleShield() {
  const on = !browserSettings.adblock;
  browserSettings.adblock = on;
  updateShield();
  shellSend('peakd:filter:', { enabled: on });
  // Half the filter is a document-start script (it joins or leaves a page when
  // that page loads), so the tab in front of the user has to reload for "off"
  // to really let ads through — and for "on" to hide them again.
  reloadActivePage();
  void apiFetch('/api/browser/settings', {
    method: 'POST',
    body: JSON.stringify({ adblock: on }),
  }).catch(() => {});
}

/** Reload the active tab's page, when it has one loaded. */
function reloadActivePage() {
  const tab = activeTab();
  if (tab && tab.native && tab.url) reload();
}

/**
 * Core's Settings window saved browser settings: follow them live.
 *
 * The ad-block switch there writes the same preference the shield button does,
 * and without this the shell would keep its old state until this window is
 * mounted again.
 */
function onSettingsChanged(event) {
  const section = event?.detail?.section;
  if (section && section !== 'browser') return;
  if (!tileEl) return;
  const before = browserSettings.adblock;
  void loadSettings().then(() => {
    // Same rule as the shield button: the scriptlet only lands on a fresh
    // document, so an open page has to reload for the change to show.
    if (browserSettings.adblock !== before) reloadActivePage();
  });
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
    ? (typeof blocked === 'number'
      ? tp('plugin.browser.shield.onBlocked', blocked, null,
        `Ad blocking: on — ${blocked} blocked`)
      : t('plugin.browser.shield.on', null, 'Ad blocking: on'))
    : t('plugin.browser.shield.off', null, 'Ad blocking: off');
  shieldBtn.setAttribute('aria-label', shieldBtn.title);
}

/* ── Downloads panel ─────────────────────────────────────────── */

/** Build the (hidden) downloads popover. */
function buildDownloadsPanel() {
  const panel = document.createElement('div');
  panel.className = 'browser-downloads-panel hud-menu hidden';

  const head = document.createElement('div');
  head.className = 'browser-downloads-head';
  const title = document.createElement('span');
  title.className = 'browser-downloads-title';
  title.textContent = t('plugin.browser.downloads.title', null, 'Downloads');
  const clear = controlButton();
  clear.type = 'button';
  clear.className = 'browser-downloads-clear';
  clear.textContent = t('plugin.browser.downloads.clearFinished', null, 'Clear finished');
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
  const willOpen = downloadsPanel.classList.contains('hidden');
  closeAllBrowserPanels();
  if (willOpen) {
    downloadsPanel.classList.remove('hidden');
    requestSync();
    void loadDownloads().then(() => revealRows(downloadsPanel, downloadsListEl));
  }
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
  if (value < 1024) return t('plugin.browser.downloads.size.b', { value }, `${value} B`);
  if (value < 1024 * 1024) {
    const kb = (value / 1024).toFixed(0);
    return t('plugin.browser.downloads.size.kb', { value: kb }, `${kb} KB`);
  }
  if (value < 1024 * 1024 * 1024) {
    const mb = (value / (1024 * 1024)).toFixed(1);
    return t('plugin.browser.downloads.size.mb', { value: mb }, `${mb} MB`);
  }
  const gb = (value / (1024 * 1024 * 1024)).toFixed(2);
  return t('plugin.browser.downloads.size.gb', { value: gb }, `${gb} GB`);
}

/** Catalog keys for the download-state tokens the shell reports. */
const DOWNLOAD_STATE_KEYS = {
  requested: 'plugin.browser.downloads.state.requested',
  downloading: 'plugin.browser.downloads.state.downloading',
  paused: 'plugin.browser.downloads.state.paused',
  completed: 'plugin.browser.downloads.state.completed',
  cancelled: 'plugin.browser.downloads.state.cancelled',
  interrupted: 'plugin.browser.downloads.state.interrupted',
};

/** The label for a download's state token (the token itself when unknown). */
function downloadStateLabel(state) {
  return DOWNLOAD_STATE_KEYS[state] ? t(DOWNLOAD_STATE_KEYS[state], null, state) : (state || '');
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
  name.textContent = d.file || d.url || t('plugin.browser.downloads.untitled', null, 'download');
  name.title = d.path || d.url || '';
  const meta = document.createElement('div');
  meta.className = 'browser-download-meta';
  const total = Number(d.total) || 0;
  const received = Number(d.received) || 0;
  const sizeText = total ? `${formatBytes(received)} / ${formatBytes(total)}` : formatBytes(received);
  meta.textContent = [d.host, downloadStateLabel(d.state), sizeText].filter(Boolean).join(' · ');
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
      label: d.state === 'paused'
        ? t('plugin.browser.downloads.resume', null, 'Resume')
        : t('plugin.browser.downloads.pause', null, 'Pause'),
      onClick: () => downloadAction(d.id, d.state === 'paused' ? 'resume' : 'pause'),
    }));
    actions.appendChild(iconButton({
      icon: 'ui/close',
      size: 'sm',
      label: t('plugin.browser.downloads.cancel', null, 'Cancel'),
      onClick: () => downloadAction(d.id, 'cancel'),
    }));
  } else if (d.state === 'completed') {
    actions.appendChild(iconButton({
      icon: 'ui/folder-open',
      size: 'sm',
      label: t('plugin.browser.downloads.open', null, 'Open'),
      onClick: () => openDownload(d),
    }));
    actions.appendChild(iconButton({
      icon: 'ui/trash',
      size: 'sm',
      label: t('plugin.browser.downloads.remove', null, 'Remove'),
      onClick: () => downloadAction(d.id, 'remove'),
    }));
  } else {
    if (d.url) {
      actions.appendChild(iconButton({
        icon: 'ui/refresh',
        size: 'sm',
        label: t('plugin.browser.downloads.retry', null, 'Retry'),
        onClick: () => navigateTo(d.url),
      }));
    }
    actions.appendChild(iconButton({
      icon: 'ui/trash',
      size: 'sm',
      label: t('plugin.browser.downloads.remove', null, 'Remove'),
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
    empty.textContent = t('plugin.browser.downloads.empty', null, 'No downloads yet');
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

/* ── Overflow menu, history and bookmarks ─────────────────────────
 *
 * The toolbar's ⋯ button opens a popover holding the search-engine choice,
 * the history and bookmark panels, and a shortcut to Settings. The panels are
 * popovers inside the tile, so they need the same treatment as a core menu:
 * they are subtracted from the native page's mask (see `occluderHoles`) so the
 * always-on-top page cannot cover them.
 */

/** A popover shell, hidden until its opener shows it. */
/** A panel's rows have loaded: cascade them in (only while it is open). */
function revealRows(panel, list) {
  if (!panel || !list || panel.classList.contains('hidden')) return;
  void listReveal(list.children);
}

function popover(className) {
  const el = document.createElement('div');
  // `hud-menu` borrows the theme's dark-island token scope (the top-bar status
  // menus'), so every popover reads as one family on both themes.
  el.className = `browser-popover hud-menu ${className} hidden`;
  return el;
}

function popoverSeparator() {
  const el = document.createElement('div');
  el.className = 'browser-popover-sep';
  return el;
}

/** One actionable row in the overflow menu. */
function menuItem(iconName, label, onClick) {
  const item = controlButton();
  item.type = 'button';
  item.className = 'browser-menu-item';
  item.setAttribute('role', 'menuitem');
  item.append(icon(iconName, { size: 15 }), labelSpan(label));
  item.addEventListener('click', onClick);
  return item;
}

function labelSpan(text) {
  const el = document.createElement('span');
  el.textContent = text;
  return el;
}

function buildBrowserMenu() {
  const menu = popover('browser-menu');
  menu.setAttribute('role', 'menu');
  menu.setAttribute('aria-label', t('plugin.browser.menu.title', null, 'Browser menu'));

  const heading = document.createElement('div');
  heading.className = 'browser-popover-heading';
  heading.textContent = t('plugin.browser.menu.searchEngine', null, 'Search engine');
  menu.appendChild(heading);

  const engines = document.createElement('div');
  engines.className = 'browser-engine-list';
  for (const engine of SEARCH_ENGINES) {
    const item = controlButton();
    item.type = 'button';
    item.className = 'browser-menu-item browser-engine';
    item.dataset.engine = engine.id;
    item.setAttribute('role', 'menuitemradio');
    const check = icon('ui/check', { size: 14, className: 'browser-engine-check' });
    item.append(check, labelSpan(engine.label));
    item.addEventListener('click', () => setSearchEngine(engine.id));
    engines.appendChild(item);
  }
  menu.appendChild(engines);

  menu.append(
    popoverSeparator(),
    menuItem('ui/history', t('plugin.browser.history.title', null, 'History'), () => {
      closeBrowserMenu();
      openHistoryPanel();
    }),
    menuItem('ui/star', t('plugin.browser.bookmarks.title', null, 'Bookmarks'), () => {
      closeBrowserMenu();
      openBookmarksPanel();
    }),
    menuItem('ui/lock', t('plugin.browser.passwords.title', null, 'Passwords'), () => {
      closeBrowserMenu();
      void openPasswordsPanel();
    }),
    popoverSeparator(),
    menuItem('ui/settings', t('plugin.browser.menu.settings', null, 'Settings'), () => {
      closeBrowserMenu();
      openCoreWindow('settings');
    }),
  );
  return menu;
}

function updateEngineChecks() {
  if (!browserMenu) return;
  for (const el of browserMenu.querySelectorAll('.browser-engine')) {
    const on = el.dataset.engine === browserSettings.searchEngine;
    el.classList.toggle('is-active', on);
    el.setAttribute('aria-checked', String(on));
  }
}

function setSearchEngine(id) {
  browserSettings.searchEngine = id;
  updateEngineChecks();
  void apiFetch('/api/browser/settings', {
    method: 'POST',
    body: JSON.stringify({ search_engine: id }),
  }).catch(() => {});
}

function toggleBrowserMenu() {
  if (!browserMenu) return;
  const willOpen = browserMenu.classList.contains('hidden');
  closeAllBrowserPanels();
  if (willOpen) {
    updateEngineChecks();
    browserMenu.classList.remove('hidden');
    requestSync();
  }
}

function closeBrowserMenu() {
  if (browserMenu) browserMenu.classList.add('hidden');
  requestSync();
}

/** Close every popover this window owns. */
function closeAllBrowserPanels() {
  let changed = false;
  for (const el of browserPopovers()) {
    if (!el.classList.contains('hidden')) {
      el.classList.add('hidden');
      changed = true;
    }
  }
  if (changed) requestSync();
}

/* ── Page scripts ──────────────────────────────────────────────── */

/** Answers to `runJs`, by token (see the `page-js` view event). */
const pageJsWaiters = new Map();
let pageJsSeq = 0;
/** A page script that has not answered by then never will (a navigation). */
const PAGE_JS_TIMEOUT_MS = 4000;

/**
 * Run `script` in a tab's native page and resolve to its result as a string,
 * or '' when there is no page, no answer, or the shell is absent.
 */
function runPageJs(tab, script) {
  if (!nativeAvailable || !tab?.native) return Promise.resolve('');
  pageJsSeq += 1;
  const token = `browser-${pageJsSeq}`;
  return new Promise((resolve) => {
    const timer = setTimeout(() => {
      pageJsWaiters.delete(token);
      resolve('');
    }, PAGE_JS_TIMEOUT_MS);
    pageJsWaiters.set(token, (result) => {
      clearTimeout(timer);
      resolve(result);
    });
    nativeSend({ op: 'runJs', id: tab.id, token, script });
  });
}

/* ── Passwords ─────────────────────────────────────────────────── */

/** The active tab's address when it shows a web page, else ''. */
function activePageUrl() {
  const tab = activeTab();
  return tab?.native && /^https?:/i.test(tab.url || '') ? tab.url : '';
}

/** Fill a login into the active page (see `fillScript` for the answers). */
async function fillActivePage(username, password) {
  const tab = activeTab();
  const url = activePageUrl();
  if (!tab || !url) return 'nofield';
  let origin;
  try {
    origin = new URL(url).origin;
  } catch (_) {
    return 'nofield';
  }
  const result = await runPageJs(tab, fillScript(origin, username, password));
  // Hand the keyboard to the page, so Enter submits the filled form.
  if (result === 'ok' || result === 'user') nativeSend({ op: 'focus', id: tab.id });
  return result;
}

/** What the user typed into the active page's login form, or null. */
async function readActivePage() {
  const tab = activeTab();
  if (!tab || !activePageUrl()) return null;
  try {
    return JSON.parse(await runPageJs(tab, readScript()) || 'null');
  } catch (_) {
    return null;
  }
}

function buildPasswords() {
  passwords = createPasswordsPanel({
    popover,
    pageUrl: activePageUrl,
    fillPage: fillActivePage,
    readPage: readActivePage,
    onLayout: () => requestSync(),
    closePanels: () => closeAllBrowserPanels(),
  });
  return passwords.element;
}

async function openPasswordsPanel() {
  closeAllBrowserPanels();
  if (!passwordsPanel || !passwords) return;
  passwordsPanel.classList.remove('hidden');
  requestSync();
  await passwords.open();
}

function togglePasswordsPanel() {
  if (passwordsPanel && !passwordsPanel.classList.contains('hidden')) {
    closeAllBrowserPanels();
    return;
  }
  void openPasswordsPanel();
}

/* ── History panel ─────────────────────────────────────────────── */

function buildHistoryPanel() {
  const panel = popover('browser-history');
  panel.setAttribute('role', 'dialog');
  panel.setAttribute('aria-label', t('plugin.browser.history.title', null, 'History'));

  const head = document.createElement('div');
  head.className = 'browser-popover-head';
  head.append(labelSpan(t('plugin.browser.history.title', null, 'History')));
  const clear = controlButton();
  clear.type = 'button';
  clear.className = 'browser-popover-action';
  clear.textContent = t('plugin.browser.history.clear', null, 'Clear');
  clear.addEventListener('click', () => void clearHistory());
  head.appendChild(clear);

  const list = document.createElement('div');
  list.className = 'browser-popover-list';
  historyListEl = list;

  panel.append(head, list);
  return panel;
}

async function openHistoryPanel() {
  closeAllBrowserPanels();
  if (historyPanel) {
    historyPanel.classList.remove('hidden');
    requestSync();
  }
  await loadHistory();
  revealRows(historyPanel, historyListEl);
}

async function loadHistory() {
  try {
    const res = await apiFetch('/api/browser/history?limit=100');
    const rows = res?.data?.history;
    if (Array.isArray(rows)) history = rows;
  } catch (_) {
    /* history is best-effort */
  }
  renderHistory();
}

async function clearHistory() {
  history = [];
  renderHistory();
  try {
    await apiFetch('/api/browser/history/clear', { method: 'POST' });
  } catch (_) {
    /* best-effort */
  }
}

function historyLabel(entry) {
  if (entry.query) {
    return t('plugin.browser.history.search', { query: entry.query }, `Search — ${entry.query}`);
  }
  try {
    return new URL(entry.url).hostname.replace(/^www\./, '') || entry.url;
  } catch (_) {
    return entry.url;
  }
}

function renderHistory() {
  if (!historyListEl) return;
  if (!history.length) {
    const empty = document.createElement('div');
    empty.className = 'browser-popover-empty';
    empty.textContent = t('plugin.browser.history.empty', null, 'No history yet');
    historyListEl.replaceChildren(empty);
    return;
  }
  historyListEl.replaceChildren(...history.map((entry) => {
    const row = controlButton();
    row.type = 'button';
    row.className = 'browser-popover-row';
    const main = document.createElement('div');
    main.className = 'browser-popover-main';
    const title = document.createElement('div');
    title.className = 'browser-popover-title';
    title.textContent = historyLabel(entry);
    const sub = document.createElement('div');
    sub.className = 'browser-popover-sub';
    sub.textContent = entry.url;
    main.append(title, sub);
    row.appendChild(main);
    row.addEventListener('click', () => {
      closeAllBrowserPanels();
      void navigateTo(entry.url);
    });
    return row;
  }));
}

/* ── Bookmarks panel ───────────────────────────────────────────── */

function buildBookmarksPanel() {
  const panel = popover('browser-bookmarks');
  panel.setAttribute('role', 'dialog');
  panel.setAttribute('aria-label', t('plugin.browser.bookmarks.title', null, 'Bookmarks'));

  const head = document.createElement('div');
  head.className = 'browser-popover-head';
  head.append(labelSpan(t('plugin.browser.bookmarks.title', null, 'Bookmarks')));

  const list = document.createElement('div');
  list.className = 'browser-popover-list';
  bookmarksListEl = list;

  panel.append(head, list);
  return panel;
}

async function openBookmarksPanel() {
  closeAllBrowserPanels();
  if (bookmarksPanel) {
    bookmarksPanel.classList.remove('hidden');
    requestSync();
  }
  await loadBookmarks();
  revealRows(bookmarksPanel, bookmarksListEl);
}

async function fetchBookmarks() {
  try {
    const res = await apiFetch('/api/browser/bookmarks?limit=200');
    const rows = res?.data?.bookmarks;
    if (Array.isArray(rows)) bookmarks = rows;
  } catch (_) {
    /* best-effort */
  }
  return bookmarks;
}

async function loadBookmarks() {
  await fetchBookmarks();
  renderBookmarks();
  updateBookmarkButton();
}

function renderBookmarks() {
  if (!bookmarksListEl) return;
  if (!bookmarks.length) {
    const empty = document.createElement('div');
    empty.className = 'browser-popover-empty';
    empty.textContent = t(
      'plugin.browser.bookmarks.empty',
      null,
      'No bookmarks yet — use the ☆ in the toolbar',
    );
    bookmarksListEl.replaceChildren(empty);
    return;
  }
  bookmarksListEl.replaceChildren(...bookmarks.map((bookmark) => {
    const row = document.createElement('div');
    row.className = 'browser-popover-row';
    const main = controlButton();
    main.type = 'button';
    main.className = 'browser-popover-main';
    const title = document.createElement('div');
    title.className = 'browser-popover-title';
    title.textContent = bookmark.title || bookmark.url;
    const sub = document.createElement('div');
    sub.className = 'browser-popover-sub';
    sub.textContent = bookmark.url;
    main.append(title, sub);
    main.addEventListener('click', () => {
      closeAllBrowserPanels();
      openInNewTab(null, bookmark.url);
    });
    const remove = iconButton({
      icon: 'ui/trash',
      size: 'sm',
      label: t('plugin.browser.bookmarks.remove', null, 'Remove bookmark'),
      onClick: () => void removeBookmark(bookmark),
    });
    remove.classList.add('browser-popover-remove');
    row.append(main, remove);
    return row;
  }));
}

function isBookmarked(url) {
  return !!url && bookmarks.some((b) => b.url === url);
}

function updateBookmarkButton() {
  if (!bookmarkBtn) return;
  const tab = activeTab();
  const on = !!tab && isBookmarked(tab.url);
  bookmarkBtn.classList.toggle('is-on', on);
  const label = on
    ? t('plugin.browser.bookmarks.remove', null, 'Remove bookmark')
    : t('plugin.browser.bookmarks.add', null, 'Bookmark this page');
  bookmarkBtn.title = label;
  bookmarkBtn.setAttribute('aria-label', label);
}

async function toggleBookmark() {
  const tab = activeTab();
  if (!tab || !tab.url || !/^https?:/.test(tab.url)) return;
  const existing = bookmarks.find((b) => b.url === tab.url);
  if (existing) {
    await removeBookmark(existing);
    return;
  }
  try {
    const res = await apiFetch('/api/browser/bookmarks/add', {
      method: 'POST',
      body: JSON.stringify({ url: tab.url, title: tab.title || tab.url }),
    });
    const bookmark = res?.data?.bookmark;
    if (bookmark) bookmarks.unshift(bookmark);
  } catch (_) {
    /* best-effort */
  }
  updateBookmarkButton();
  renderBookmarks();
}

async function removeBookmark(bookmark) {
  bookmarks = bookmarks.filter((b) => b.id !== bookmark.id);
  updateBookmarkButton();
  renderBookmarks();
  try {
    await apiFetch('/api/browser/bookmarks/remove', {
      method: 'POST',
      body: JSON.stringify({ id: bookmark.id }),
    });
  } catch (_) {
    /* best-effort */
  }
}

/**
 * Record a finished page load as a visit.
 *
 * Typed navigations are already recorded server-side (with the search phrase,
 * which the recommender weighs). This adds the pages reached *inside* a site —
 * a link followed on a page never round-trips through `/navigate`, so without
 * this the history would only ever hold what was typed.
 */
function recordVisit(tab) {
  if (!tab || tab.incognito || !tab.url || !/^https?:/.test(tab.url)) return;
  if (tab.lastHistoryUrl === tab.url) return;
  tab.lastHistoryUrl = tab.url;
  void apiFetch('/api/browser/history/record', {
    method: 'POST',
    body: JSON.stringify({ url: tab.url, mode: 'visit' }),
  }).catch(() => {});
}

/** Dismiss the popovers when the pointer lands outside them. */
function onBrowserPointerDown(event) {
  const target = event.target;
  if (target?.closest?.('.browser-popover')) return;
  if (target?.closest?.('.browser-menu-btn, .browser-bookmark, .browser-downloads-btn, .browser-tabcount, .browser-passwords-btn')) return;
  // A confirmation the window asked for (deleting a saved password) is not a
  // click away from it.
  if (target?.closest?.('.ui-modal')) return;
  closeAllBrowserPanels();
}

function onBrowserKeydown(event) {
  if (event.key === 'Escape') closeAllBrowserPanels();
}

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

/**
 * The theme tokens the start page needs, resolved now.
 *
 * The sandboxed frame is a separate document: it inherits none of the app's
 * custom properties, so the active theme's values are read from the shell's
 * root and written into the page as its own `:root`. Re-rendered on
 * `theme:change` / `appearance:change` (see `refreshHomeTheme`).
 */
const HOME_TOKENS = [
  '--bg', '--bg-elevated', '--text', '--text-secondary', '--muted', '--accent',
  '--accent-contrast', '--ed-pill', '--ed-pill-hover', '--ed-pill-dark',
  '--ed-pill-dark-ink', '--font-display', '--font-body', '--font-mono',
];

function homePalette() {
  const vars = {};
  try {
    const style = getComputedStyle(document.documentElement);
    for (const name of HOME_TOKENS) {
      // Only what can be a CSS value: a token must not be able to close the
      // style block it is written into.
      const value = style.getPropertyValue(name).trim().replace(/[<>{};]/g, '');
      if (value) vars[name] = value;
    }
  } catch (_) {
    /* no computed style (tests): the page falls back to its own defaults */
  }
  let mode = 'dark';
  try {
    mode = themeMode() === 'light' ? 'light' : 'dark';
  } catch (_) {
    /* theme manifest not loaded yet */
  }
  const motion = (typeof document !== 'undefined' && document.documentElement?.dataset?.motion) || 'kinetic';
  return { vars, mode, motion: /^[a-z]+$/.test(motion) ? motion : 'kinetic' };
}

/** A palette's identity, to skip re-rendering when nothing changed. */
function paletteKey(palette) {
  return JSON.stringify(palette);
}

/** The large title's date eyebrow, in the UI language. */
function homeDateLabel() {
  try {
    const lang = document.documentElement.lang || undefined;
    return new Intl.DateTimeFormat(lang, { weekday: 'short', day: 'numeric', month: 'short' }).format(new Date());
  } catch (_) {
    return '';
  }
}

/** The home page markup, with the shelf and bookmarks' data embedded. */
function homeDocument(news, saved = [], { palette = homePalette(), animate = true } = {}) {
  const cards = Array.isArray(news?.cards) ? news.cards : [];
  const topics = Array.isArray(news?.topics) ? news.topics : [];
  const personal = news?.personalized && cards.length;
  const recent = t('plugin.browser.home.recentSearches', null, 'Because of your recent searches');
  const refresh = t('plugin.browser.home.refresh', null, 'Refresh');
  const refreshing = t('plugin.browser.home.refreshing', null, 'Refreshing…');
  const bookmarkHeading = t('plugin.browser.bookmarks.title', null, 'Bookmarks');
  const title = t('plugin.browser.home.title', null, 'Home');
  const bookmarkList = (Array.isArray(saved) ? saved : []).filter((b) => b?.url);
  const meta = [
    bookmarkList.length
      ? tp('plugin.browser.home.metaBookmarks', bookmarkList.length, null, `${bookmarkList.length} bookmarks`)
      : '',
    personal ? tp('plugin.browser.home.metaStories', cards.length, null, `${cards.length} stories`) : '',
  ].filter(Boolean);
  // Each revealed block carries its place in the cascade (`--i`).
  let step = 0;
  const rv = () => `rv" style="--i:${step++}`;
  const head = `<header class="hero">
      <div class="eyebrow ${rv()}">${escapeHtml(homeDateLabel())}</div>
      <h1 class="title ${rv()}">${escapeHtml(title)}</h1>
      ${meta.length ? `<div class="eyebrow meta-line ${rv()}">${meta.map(escapeHtml).join(' · ')}</div>` : ''}
    </header>`;
  const savedGrid = bookmarkList.length
    ? `<section class="saved">
         <div class="section-head ${rv()}"><span class="eyebrow">${escapeHtml(bookmarkHeading)}</span></div>
         <div class="saved-grid">${bookmarkList.map((b) => bookmarkCardHtml(b, rv)).join('')}</div>
       </section>`
    : '';
  // The shelf's data block. Also where every *translated* string the handler
  // needs has to live: the handler's own text is allowlisted in the app's CSP
  // by an exact SHA-256 (src/api/mod.rs), so interpolating into the script
  // would change its bytes per locale and break the hash.
  const payload = JSON.stringify({ cards, topics, refreshing }).replace(/<\//g, '<\\/');

  const shelf = personal
    ? `<div class="section-head ${rv()}"><span class="eyebrow">${escapeHtml(recent)}</span>
         <button type="button" class="again" data-refresh>${escapeHtml(refresh)}</button></div>
       ${topics.length ? `<div class="chips ${rv()}">${topics
        .map((topic) => `<span class="chip">${escapeHtml(topic)}</span>`)
        .join('')}</div>` : ''}
       <div class="cards">${cards.map((card, index) => cardHtml(card, index, rv)).join('')}</div>`
    : emptyShelf(news, rv);

  const root = Object.entries(palette.vars || {}).map(([k, v]) => `${k}:${v}`).join(';');
  const motion = animate ? palette.motion : 'none';

  return `<!doctype html><html data-motion="${escapeHtml(motion)}"><head><meta charset="utf-8">
<style>
  :root{color-scheme:${palette.mode === 'light' ? 'light' : 'dark'};${root}}
  html,body{min-height:100%;margin:0}
  html{scrollbar-width:thin;scrollbar-color:color-mix(in srgb,var(--text) 18%,transparent) transparent}
  /* Transparent: the window's own surface shows through, as everywhere else. */
  body{background:transparent;color:var(--text);
       font-family:var(--font-body,system-ui,sans-serif);
       -webkit-font-smoothing:antialiased}
  .home{max-width:52rem;margin:0 auto;padding:44px 28px 56px}
  .eyebrow{font-family:var(--font-mono,ui-monospace,monospace);font-size:10.5px;
           letter-spacing:.06em;text-transform:uppercase;color:var(--muted)}
  .hero{margin-bottom:36px}
  .title{margin:6px 0 8px;font-family:var(--font-display,var(--font-body));font-size:34px;
         font-weight:600;letter-spacing:-.045em;line-height:1.05;color:var(--text)}
  .hair{border:0;border-top:1px solid color-mix(in srgb,var(--text) 9%,transparent);margin:0}
  .section-head{display:flex;align-items:center;justify-content:space-between;gap:16px;
                min-height:32px;padding-bottom:10px;margin-bottom:14px;
                border-bottom:1px solid color-mix(in srgb,var(--text) 9%,transparent)}
  .saved{margin-bottom:40px}
  .saved-grid{display:grid;gap:18px 14px;grid-template-columns:repeat(auto-fill,minmax(84px,1fr))}
  a.bookmark{display:flex;flex-direction:column;gap:8px;min-width:0;text-decoration:none;color:inherit}
  .favicon-wrap{position:relative;display:flex;align-items:center;justify-content:center;
                aspect-ratio:1;border-radius:14px;overflow:hidden;background:var(--ed-pill);
                transition:background .18s ease}
  a.bookmark:hover .favicon-wrap{background:var(--ed-pill-hover)}
  .favicon-initial{position:absolute;inset:0;display:flex;align-items:center;justify-content:center;
                   font-family:var(--font-display,var(--font-body));font-size:24px;font-weight:600;
                   letter-spacing:-.04em;text-transform:uppercase;color:var(--text-secondary)}
  img.favicon{position:relative;width:28px;height:28px;object-fit:contain;background:transparent}
  .bookmark-name{max-width:100%;font-family:var(--font-mono,ui-monospace,monospace);font-size:10.5px;
                 letter-spacing:.06em;text-transform:uppercase;color:var(--text-secondary);
                 white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
  a.bookmark:hover .bookmark-name{color:var(--text)}
  .chips{display:flex;flex-wrap:wrap;gap:6px;margin:-2px 0 8px}
  .chip{font-family:var(--font-mono,ui-monospace,monospace);font-size:10.5px;letter-spacing:.06em;
        text-transform:uppercase;color:var(--text-secondary);padding:5px 10px;border-radius:999px;
        background:var(--ed-pill)}
  .again{flex:none;display:inline-flex;align-items:center;gap:8px;height:30px;padding:0 14px;
         border:0;border-radius:999px;background:var(--ed-pill);color:var(--text);cursor:pointer;
         font-family:var(--font-body,system-ui,sans-serif);font-size:12.5px;font-weight:500;
         transition:background .18s ease}
  .again::after{content:'\\2192';display:inline-block;transition:transform .25s cubic-bezier(.16,1,.3,1)}
  .again:hover{background:var(--ed-pill-hover)}
  .again:hover::after{transform:translateX(3px)}
  .again:disabled{cursor:default;color:var(--muted)}
  .again:focus-visible,a.card:focus-visible,a.bookmark:focus-visible{outline:2px solid var(--accent);outline-offset:2px}
  .cards{display:flex;flex-direction:column}
  a.card{display:flex;gap:16px;padding:16px 0;text-decoration:none;color:inherit;
         border-bottom:1px solid color-mix(in srgb,var(--text) 9%,transparent)}
  a.card:last-child{border-bottom:0}
  .thumb-wrap{flex:none;width:88px;height:88px;border-radius:12px;overflow:hidden;background:var(--ed-pill)}
  .thumb{display:block;width:100%;height:100%;object-fit:cover;transition:transform .5s cubic-bezier(.16,1,.3,1)}
  a.card:hover .thumb{transform:scale(1.04)}
  .body{display:flex;flex-direction:column;gap:6px;min-width:0;flex:1 1 auto;padding-top:2px}
  .meta{font-family:var(--font-mono,ui-monospace,monospace);font-size:10.5px;letter-spacing:.06em;
        text-transform:uppercase;color:var(--muted);white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
  .meta b{font-weight:500;color:var(--text-secondary)}
  .headline{font-family:var(--font-display,var(--font-body));font-size:17px;font-weight:600;
            letter-spacing:-.02em;line-height:1.25;color:var(--text);
            display:-webkit-box;-webkit-line-clamp:2;-webkit-box-orient:vertical;overflow:hidden;
            text-decoration:underline;text-decoration-color:transparent;text-underline-offset:3px;
            text-decoration-thickness:1px;transition:text-decoration-color .18s ease}
  a.card:hover .headline{text-decoration-color:currentColor}
  .snip{font-size:13px;line-height:1.5;color:var(--text-secondary);
        display:-webkit-box;-webkit-line-clamp:2;-webkit-box-orient:vertical;overflow:hidden}
  .empty{padding:28px 0 0}
  .empty .t{font-family:var(--font-display,var(--font-body));font-size:20px;font-weight:600;
            letter-spacing:-.03em;margin:8px 0 6px}
  .empty .s{max-width:32rem;font-size:13.5px;line-height:1.55;color:var(--text-secondary)}
  @media (max-width:520px){
    .home{padding:28px 16px 40px}
    .thumb-wrap{width:64px;height:64px}
    .snip{display:none}
  }
  /* The cascade: each block enters in turn, in the user's motion language
     (the app's listReveal cannot reach into this frame). */
  @keyframes rv-wipe{from{clip-path:inset(0 100% 0 0)}to{clip-path:inset(0 0 0 0)}}
  @keyframes rv-rise{from{opacity:0;transform:translateY(14px)}to{opacity:1;transform:none}}
  @keyframes rv-depth{from{opacity:0;transform:scale(.96);filter:blur(6px)}to{opacity:1;transform:none;filter:blur(0)}}
  .rv{animation:rv-wipe .64s cubic-bezier(.76,0,.24,1) both;animation-delay:calc(min(var(--i,0),18) * 45ms)}
  [data-motion="fluid"] .rv{animation-name:rv-rise;animation-duration:.6s;animation-timing-function:cubic-bezier(.34,1.4,.64,1)}
  [data-motion="depth"] .rv{animation-name:rv-depth;animation-duration:.7s;animation-timing-function:cubic-bezier(.16,1,.3,1)}
  [data-motion="none"] .rv{animation:none}
  @media (prefers-reduced-motion:reduce){.rv{animation:none}.again::after,.thumb{transition:none}}
</style></head><body>
<div class="home">
  ${head}
  ${savedGrid}
  <section id="shelf">${shelf}</section>
</div>
<scr${'ipt'} id="home-data" type="application/json">${payload}</scr${'ipt'}>
<scr${'ipt'}>
(function () {
  var data = { cards: [], topics: [] };
  try { data = JSON.parse(document.getElementById('home-data').textContent || '{}'); } catch (e) {}
  if (!data.cards.length) data.cards = [];

  // A broken favicon falls back to the letter tile under it; a loaded one
  // covers that tile.
  function hideInitial(img) {
    var wrap = img.parentNode;
    var initial = wrap && wrap.querySelector ? wrap.querySelector('.favicon-initial') : null;
    if (initial) initial.style.display = 'none';
  }
  Array.prototype.forEach.call(document.querySelectorAll('img.favicon'), function (img) {
    if (img.complete && img.naturalWidth) hideInitial(img);
    img.addEventListener('load', function () { hideInitial(img); });
  });
  document.addEventListener('error', function (event) {
    var img = event.target;
    if (img && img.tagName === 'IMG' && img.classList && img.classList.contains('favicon')) {
      img.style.display = 'none';
    }
  }, true);

  // A card navigates by asking the parent, which is the only frame allowed to
  // touch the proxy/session state. The sandbox gives this document no
  // same-origin access, so postMessage is the only channel — and it is enough.
  document.addEventListener('click', function (event) {
    var saved = event.target && event.target.closest ? event.target.closest('a.bookmark') : null;
    if (saved) {
      event.preventDefault();
      try {
        parent.postMessage({ type: 'browser:open-bookmark', url: saved.getAttribute('data-url') || '' }, '*');
      } catch (e) {}
      return;
    }
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
      again.textContent = data.refreshing || 'Refreshing…';
      try { parent.postMessage({ type: 'browser:refresh-news' }, '*'); } catch (e) {}
    }
  });
})();
</scr${'ipt'}>
</body></html>`;
}

/** One card. Kept as a string because it is injected into the sandboxed frame. */
function cardHtml(card, index, rv = () => '') {
  // Mono metadata, one line: SOURCE · 2H · TOPIC.
  const meta = [
    card.source ? `<b>${escapeHtml(card.source)}</b>` : '',
    card.age ? escapeHtml(card.age) : '',
    card.topic ? escapeHtml(card.topic) : '',
  ]
    .filter(Boolean)
    .join(' · ');
  const snippet = card.snippet ? `<span class="snip">${escapeHtml(card.snippet)}</span>` : '';
  // The engine's thumbnail is loaded straight from its CDN. `no-referrer` keeps
  // the app's origin out of the request; the image is decorative (the title
  // carries the meaning), so a failed load must not show a broken glyph — the
  // grey tile under it stays.
  const thumb = card.image
    ? `<img class="thumb" src="${escapeHtml(card.image)}" alt="" loading="lazy" referrerpolicy="no-referrer">`
    : '';
  return `<a class="card ${rv()}" href="#" data-index="${index}" title="${escapeHtml(card.title || '')}">
    <span class="thumb-wrap">${thumb}</span>
    <span class="body">
      <span class="meta">${meta}</span>
      <span class="headline">${escapeHtml(card.title || '')}</span>
      ${snippet}
    </span>
  </a>`;
}

/**
 * The page's own icon. Loading it from the site's origin keeps the start page
 * free of a third-party favicon service; a site that does not serve
 * `/favicon.ico` falls back to the letter tile drawn underneath.
 */
function faviconFor(url) {
  try {
    const parsed = new URL(url);
    if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') return '';
    return `${parsed.origin}/favicon.ico`;
  } catch (_) {
    return '';
  }
}

/** One saved page: a grey square tile with its favicon, a mono caption under it. */
function bookmarkCardHtml(bookmark, rv = () => '') {
  const url = String(bookmark?.url || '');
  if (!url) return '';
  let name = String(bookmark?.title || '').trim();
  if (!name) {
    try {
      name = new URL(url).hostname.replace(/^www\./, '');
    } catch (_) {
      name = url;
    }
  }
  if (!name) name = url;
  const initial = Array.from(name.trim())[0] || '•';
  const icon = faviconFor(url);
  const img = icon
    ? `<img class="favicon" src="${escapeHtml(icon)}" alt="" loading="lazy" referrerpolicy="no-referrer">`
    : '';
  return `<a class="bookmark ${rv()}" href="#" data-url="${escapeHtml(url)}" title="${escapeHtml(url)}">
    <span class="favicon-wrap">
      <span class="favicon-initial">${escapeHtml(initial)}</span>
      ${img}
    </span>
    <span class="bookmark-name">${escapeHtml(name)}</span>
  </a>`;
}

/** What to show when there is nothing to personalise from. */
function emptyShelf(news, rv = () => '') {
  const reason = news?.error ? escapeHtml(news.error) : '';
  const body = news?.personalized === false && !reason
    ? escapeHtml(t('plugin.browser.home.emptyBody', null,
      'Search for a few things and this page fills up with news about them.'))
    : reason || escapeHtml(t('plugin.browser.home.emptyRetry', null, 'Nothing to show yet — try refreshing.'));
  return `<div class="section-head ${rv()}"><span class="eyebrow">${escapeHtml(
    t('plugin.browser.home.news', null, 'News'),
  )}</span>
      <button type="button" class="again" data-refresh>${escapeHtml(t('plugin.browser.home.refresh', null, 'Refresh'))}</button></div>
    <div class="empty ${rv()}">
      <div class="t">${escapeHtml(t('plugin.browser.home.emptyTitle', null, 'No stories yet'))}</div>
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
    setStatus(t('plugin.browser.status.loadingRecommendations', null, 'Loading recommendations…'));
  }
  // The idle surface must not be same-origin with the app shell (it is
  // rendered from server data), so it is sandboxed without allow-same-origin.
  tab.frameEl.removeAttribute('allow');
  tab.frameEl.setAttribute('sandbox', 'allow-scripts');
  tab.frameEl.removeAttribute('srcdoc');
  delete tab.frameEl.dataset.home;
  // Clear the previous page *before* awaiting the shelf, so going Home is
  // instant instead of leaving the last site on screen while it loads.
  tab.frameEl.src = 'about:blank';

  let news = { cards: [], topics: [], personalized: false };
  let saved = bookmarks;
  try {
    [news, saved] = await Promise.all([
      fetchNews().catch(() => ({
        cards: [], topics: [], personalized: false,
        error: t('plugin.browser.home.loadFailed', null, 'Could not load recommendations'),
      })),
      fetchBookmarks(),
    ]);
  } catch (_) {
    /* both helpers already fall back to what they have */
  }
  // Re-check: the user may have navigated this tab while the shelf was in
  // flight.
  if (!tab.frameEl || !tab.home) return;
  if (tab.id === activeTabId) setStatus('');
  tab.frameEl.src = 'about:blank';
  // Kept so a theme change can repaint the page without refetching it.
  tab.homeData = { news, saved };
  tab.homePalette = homePalette();
  tab.frameEl.dataset.home = '';
  // Same colour scheme on both sides of the frame, or Chromium paints an
  // opaque canvas behind the (transparent) page.
  tab.frameEl.style.colorScheme = tab.homePalette.mode;
  tab.frameEl.srcdoc = homeDocument(news, saved, { palette: tab.homePalette });
  renderTabs();
  if (tab.id === activeTabId) updateNavButtons();
}

/**
 * The theme or accent changed: repaint every open start page with the new
 * values, without the entrance cascade. A theme's stylesheet arrives a moment
 * after its `theme:change`, so the palette is re-read a few times and a page
 * is only rebuilt when its resolved values actually differ.
 */
let homeThemeTimers = [];
function refreshHomeTheme() {
  for (const timer of homeThemeTimers) clearTimeout(timer);
  homeThemeTimers = [0, 160, 500, 1200].map((delay) => setTimeout(repaintHomes, delay));
}

function repaintHomes() {
  const palette = homePalette();
  const key = paletteKey(palette);
  for (const tab of tabs) {
    if (!tab.home || !tab.homeData || !tab.frameEl) continue;
    if (tab.homePalette && paletteKey(tab.homePalette) === key) continue;
    tab.homePalette = palette;
    tab.frameEl.style.colorScheme = palette.mode;
    tab.frameEl.srcdoc = homeDocument(tab.homeData.news, tab.homeData.saved, { palette, animate: false });
  }
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

/**
 * A link clicked anywhere in the app: core activates this window and hands the
 * URL over (see `web/js/linkRouter.js`), so it opens as a new tab — exactly
 * like a `target="_blank"` link inside a page. The tile may not exist yet
 * (module load precedes mount, and `createTab` needs the frame wrapper), so a
 * pre-mount hand-off waits for `mountBrowserTile` instead of being dropped.
 */
function openCoreUrl(url) {
  if (!tileEl) { pendingCoreOpen = url; return; }
  openInNewTab(null, url);
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
  } else if (data.type === 'browser:open-bookmark') {
    // Both the start page's grid and the toolbar menu open saved pages in a
    // fresh tab, so the start page stays where it is.
    const url = String(data.url || '');
    if (url) openInNewTab(tab, url);
  } else if (data.type === 'browser:refresh-news') {
    void showHome(tab);
  }
}

/**
 * A right-click inside a page: the shell forwards the request (position local
 * to the view item, plus the link and selection state) and the app's own menu
 * opens over the page — the mask hole keeps it visible and clickable. Page
 * verbs travel back as `peakd:view` commands; Copy Link Address goes through
 * the app's clipboard service, so it lands in the Clipboard menu like every
 * other copy. Navigation uses the dedicated ops, the edit verbs the generic
 * `action` op (only a native view can touch its page's DOM).
 */
function openPageContextMenu(tab, menu) {
  if (!nativeAvailable || !menu || typeof menu !== 'object') return;
  if (tab.id !== activeTabId || !tab.native) return;
  const x = Number(menu.x);
  const y = Number(menu.y);
  const r = viewportRect();
  if (!Number.isFinite(x) || !Number.isFinite(y) || !r) return;

  const dpr = r.dpr || 1;
  const send = (op, extra) => nativeSend({ op, id: tab.id, ...extra });
  const editable = menu.editable === true;
  const entries = [];
  if (menu.linkUrl) {
    entries.push({
      type: 'item',
      label: t('plugin.browser.pageMenu.copyLink', null, 'Copy Link Address'),
      icon: 'ui/link',
      onClick: () => void copyText(menu.linkUrl),
    });
    entries.push({ type: 'separator' });
  }
  entries.push(
    {
      type: 'item',
      label: t('plugin.browser.toolbar.back', null, 'Back'),
      icon: 'ui/arrow-left',
      onClick: () => send('back'),
    },
    {
      type: 'item',
      label: t('plugin.browser.toolbar.forward', null, 'Forward'),
      icon: 'ui/forward',
      onClick: () => send('forward'),
    },
    {
      type: 'item',
      label: t('plugin.browser.toolbar.reload', null, 'Reload'),
      icon: 'ui/refresh',
      onClick: () => send('reload'),
    },
    { type: 'separator' },
  );
  if (editable) {
    entries.push(
      {
        type: 'item',
        label: t('plugin.browser.passwords.fillMenu', null, 'Fill password…'),
        icon: 'ui/lock',
        onClick: () => void openPasswordsPanel(),
      },
      { type: 'separator' },
      {
        type: 'item',
        label: t('plugin.browser.pageMenu.cut', null, 'Cut'),
        onClick: () => send('action', { verb: 'Cut' }),
      },
      {
        type: 'item',
        label: t('plugin.browser.pageMenu.copy', null, 'Copy'),
        icon: 'ui/copy',
        disabled: !menu.selection,
        onClick: () => send('action', { verb: 'Copy' }),
      },
      {
        type: 'item',
        label: t('plugin.browser.pageMenu.paste', null, 'Paste'),
        icon: 'ui/clipboard',
        onClick: () => send('action', { verb: 'Paste' }),
      },
    );
  } else {
    entries.push({
      type: 'item',
      label: t('plugin.browser.pageMenu.copy', null, 'Copy'),
      icon: 'ui/copy',
      disabled: !menu.selection,
      onClick: () => send('action', { verb: 'Copy' }),
    });
  }
  entries.push({
    type: 'item',
    label: t('plugin.browser.pageMenu.selectAll', null, 'Select All'),
    icon: 'ui/select-all',
    onClick: () => send('action', { verb: 'SelectAll' }),
  });

  openContextMenu(entries, r.x + x / dpr, r.y + y / dpr);
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
      recordVisit(tab);
      if (tab.id === activeTabId) {
        setStatus('');
        updateBookmarkButton();
      }
    } else if (event.phase === 'started' && tab.id === activeTabId) {
      setStatus(t('plugin.browser.status.loading', null, 'Loading…'));
    }
  } else if (event.type === 'new-window' && typeof event.url === 'string' && event.url) {
    // A `target="_blank"` link in a native view: the shell refused to hand a
    // popup to the OS browser, so open it as a tab here.
    openInNewTab(tab, event.url);
  } else if (event.type === 'context-menu') {
    openPageContextMenu(tab, event.menu);
  } else if (event.type === 'page-js' && typeof event.token === 'string') {
    const waiter = pageJsWaiters.get(event.token);
    if (waiter) {
      pageJsWaiters.delete(event.token);
      waiter(typeof event.result === 'string' ? event.result : '');
    }
  }
};

/* ── Native-view bounds sync ──────────────────────────────────── */

let boundsTimer = null;
let boundsObserver = null;
let tileObserver = null;
let bodyObserver = null;
let stopOccluderWatch = null;
let syncQueued = false;
let menuSyncTimer = null;
/** Core menus slide in with `hudMenuIn`; re-sync once they have landed. */
const MENU_SETTLE_MS = 200;
let shieldTimer = null;
/** How often the open window asks the shell for the live shield state. */
const SHIELD_POLL_MS = 3000;

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

/**
 * A menu opened or closed: re-evaluate the mask now, and once more after the
 * slide-in animation has settled. Sampling the hole mid-animation leaves the
 * menu's top edge clipped by the page until the 250 ms fallback tick.
 */
function onMenuChange() {
  requestSync();
  clearTimeout(menuSyncTimer);
  menuSyncTimer = setTimeout(requestSync, MENU_SETTLE_MS);
}

function startBoundsSync() {
  stopBoundsSync();
  // A native child window is positioned in native pixels, so it must follow the
  // viewport element every frame it moves. A `ResizeObserver` covers resizes and
  // a `MutationObserver` covers the inline style the drag/resize code writes;
  // the one-second interval is only a fallback for anything else (a CSS
  // animation, say). Only the active tab is visible, so only it is tracked.
  boundsTimer = setInterval(syncActiveNative, 1000);
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
  // Core chrome toggles its visibility class after a short transition (chat,
  // travel panel, tile overlay); refresh the mask as soon as it does instead of
  // leaving a stale hole until the bounds fallback.
  stopOccluderWatch = watchNativeOccluders(requestSync);
  window.addEventListener('resize', requestSync);
  window.addEventListener('desktop:changed', requestSync);
  window.addEventListener('overlay:open', requestSync);
  window.addEventListener('fullscreen:change', requestSync);
  // A core popup opening/closing changes what covers the page (it becomes a
  // hole in the view's mask), so re-evaluate on the shared menu signal.
  window.addEventListener('menu:change', onMenuChange);
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
  if (stopOccluderWatch) stopOccluderWatch();
  stopOccluderWatch = null;
  window.removeEventListener('resize', requestSync);
  window.removeEventListener('desktop:changed', requestSync);
  window.removeEventListener('overlay:open', requestSync);
  window.removeEventListener('fullscreen:change', requestSync);
  window.removeEventListener('menu:change', onMenuChange);
  clearTimeout(menuSyncTimer);
  menuSyncTimer = null;
}

/**
 * Keep the shield's blocked count honest while this window is open.
 *
 * The shell only pushes `window.__peakdShield` when something changes, so
 * without a poll the count would look frozen even while the filter is busy.
 */
function startShieldPoll() {
  stopShieldPoll();
  shieldTimer = setInterval(() => {
    if (!document.hidden && nativeAvailable) shellRaw('peakd:shield:query');
  }, SHIELD_POLL_MS);
}

function stopShieldPoll() {
  if (shieldTimer) clearInterval(shieldTimer);
  shieldTimer = null;
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

/**
 * A pick in the top-bar Clipboard menu with this window focused.
 *
 * The page is a native child view, so the app cannot touch its DOM: the shell
 * delivers the text (`peakd:view` `paste` command sets the system clipboard
 * and triggers the engine's Paste action on the tab).
 */
function onClipboardPaste(event) {
  const { text, focused } = event.detail || {};
  if (focused !== BROWSER_PLUGIN || typeof text !== 'string' || !text) return;
  const tab = activeTab();
  if (!nativeAvailable || !tab?.native) return;
  nativeSend({ op: 'paste', id: tab.id, text });
}

/** Clipboard text as an address (or search term) in the active tab. */
async function pasteAndGo() {
  const text = await readClipboardText();
  if (text) void navigateTo(text);
}

/**
 * Entries core splices into this window's right-click menu (PLUGINS.md §19).
 */
export function browserContextMenu() {
  return [
    {
      type: 'item',
      label: t('plugin.browser.tabs.newTab', null, 'New tab'),
      icon: 'ui/plus',
      onClick: () => createTab({}),
    },
    {
      type: 'item',
      label: t('plugin.browser.tabs.newIncognito', null, 'New incognito tab'),
      icon: 'ui/incognito',
      onClick: () => createTab({ incognito: true }),
    },
    {
      type: 'item',
      label: t('plugin.browser.downloads.title', null, 'Downloads'),
      icon: 'ui/download',
      onClick: () => toggleDownloadsPanel(),
    },
    {
      type: 'item',
      label: t('plugin.browser.history.title', null, 'History'),
      icon: 'ui/history',
      onClick: () => void openHistoryPanel(),
    },
    {
      type: 'item',
      label: t('plugin.browser.bookmarks.title', null, 'Bookmarks'),
      icon: 'ui/star',
      onClick: () => void openBookmarksPanel(),
    },
    {
      type: 'item',
      label: isBookmarked(activeTab()?.url)
        ? t('plugin.browser.bookmarks.remove', null, 'Remove bookmark')
        : t('plugin.browser.bookmarks.add', null, 'Bookmark this page'),
      icon: 'ui/star',
      disabled: !activeTab()?.url,
      onClick: () => void toggleBookmark(),
    },
    {
      // The central clipboard service records this like any other copy.
      type: 'item',
      label: t('plugin.browser.ctx.copyAddress', null, 'Copy page address'),
      icon: 'ui/copy',
      disabled: !activeTab()?.url,
      onClick: () => void copyText(activeTab()?.url || '', { source: BROWSER_PLUGIN }),
    },
    {
      type: 'item',
      label: t('plugin.browser.ctx.pasteAndGo', null, 'Paste and go'),
      icon: 'ui/clipboard',
      onClick: () => void pasteAndGo(),
    },
    {
      type: 'item',
      label: t('plugin.browser.ctx.focusAddress', null, 'Focus address bar'),
      icon: 'ui/search',
      disabled: !addressEl,
      onClick: () => addressEl?.input?.focus(),
    },
    {
      type: 'item',
      label: t('plugin.browser.toolbar.reload', null, 'Reload'),
      icon: 'ui/loop',
      disabled: !frameWrapEl,
      onClick: () => reload(),
    },
    {
      type: 'item',
      label: t('plugin.browser.toolbar.home', null, 'Home'),
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
  browserMenu = buildBrowserMenu();
  historyPanel = buildHistoryPanel();
  bookmarksPanel = buildBookmarksPanel();
  tabsPanel = buildTabsPanel();
  passwordsPanel = buildPasswords();
  tileEl.append(
    tabstripEl, buildToolbar(), viewportEl,
    downloadsPanel, browserMenu, historyPanel, bookmarksPanel, tabsPanel, passwordsPanel,
  );

  document.addEventListener('pointerdown', onBrowserPointerDown, true);
  document.addEventListener('keydown', onBrowserKeydown, true);

  updateNavButtons();
  updateShield();
  startBoundsSync();
  startShieldPoll();
  // One tab, on its home surface.
  createTab({});
  // A link clicked before this window mounted now has somewhere to go.
  if (pendingCoreOpen) {
    const url = pendingCoreOpen;
    pendingCoreOpen = null;
    openInNewTab(null, url);
  }
  // Core calls `wireEvents` once, when it first loads this module, but
  // `unmountBrowserTile` drops those listeners (the home shelf's
  // `message`, clipboard, settings). Re-wire on every mount (idempotent), or
  // a window closed with its X and reopened would ignore every bookmark and
  // news click until the plugin was reloaded.
  wireBrowserEvents();
  // Settings (shield, search engine, colour scheme), downloads and bookmarks.
  void loadSettings();
  void loadDownloads();
  void loadBookmarks();
  return tileEl;
}

export function unmountBrowserTile() {
  stopBoundsSync();
  stopShieldPoll();
  window.removeEventListener('message', onFrameMessage);
  window.removeEventListener('clipboard:paste', onClipboardPaste);
  window.removeEventListener('settings:change', onSettingsChanged);
  window.removeEventListener('theme:change', refreshHomeTheme);
  window.removeEventListener('appearance:change', refreshHomeTheme);
  for (const timer of homeThemeTimers) clearTimeout(timer);
  homeThemeTimers = [];
  document.removeEventListener('pointerdown', onBrowserPointerDown, true);
  document.removeEventListener('keydown', onBrowserKeydown, true);
  wired = false;
  for (const tab of tabs) destroyTab(tab);
  tabs = [];
  activeTabId = null;
  downloads = [];
  history = [];
  bookmarks = [];
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
  bookmarkBtn = null;
  menuBtn = null;
  downloadsPanel = null;
  downloadsListEl = null;
  browserMenu = null;
  historyPanel = null;
  historyListEl = null;
  bookmarksPanel = null;
  tabsPanel = null;
  tabsListEl = null;
  passwordsPanel = null;
  passwords = null;
  passwordsBtn = null;
  for (const resolve of pageJsWaiters.values()) resolve('');
  pageJsWaiters.clear();
  bookmarksListEl = null;
}

export function getBrowserTileElement() {
  return tileEl;
}

export function wireBrowserEvents() {
  if (wired) return;
  wired = true;
  window.addEventListener('artifact:saved', onArtifactSaved);
  // Links clicked anywhere in the app (core hands over `url:open`).
  onOpenUrl(BROWSER_PLUGIN, openCoreUrl);
  // A pick in the top-bar Clipboard menu → paste into the focused page.
  window.addEventListener('clipboard:paste', onClipboardPaste);
  // The home shelf talks back through postMessage (card clicks, refresh).
  window.addEventListener('message', onFrameMessage);
  // The Settings window's own ad-block switch writes the same preference:
  // follow it live instead of waiting for this window to mount again.
  window.addEventListener('settings:change', onSettingsChanged);
  // Keep the pages' `prefers-color-scheme` in step with the app theme.
  window.addEventListener('theme:change', pushSettings);
  // The start page is a separate document holding the theme's resolved values.
  window.addEventListener('theme:change', refreshHomeTheme);
  window.addEventListener('appearance:change', refreshHomeTheme);
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
