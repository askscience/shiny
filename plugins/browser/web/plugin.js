/**
 * browser.js — the Browser plugin's window.
 *
 * A focused web viewport rendered *through PEAK'D!'s own filter proxy*: the
 * iframe's origin is the plugin's proxy, not the open internet, so every
 * request the page makes passes through the shared adblock engine. That is the
 * whole design — there is no request the engine cannot see, which is what
 * makes filtering here complete rather than best-effort.
 *
 * Chrome: a tab strip, back / forward / reload / home, an address bar that
 * doubles as a search box, and a shield showing how many requests were blocked.
 * Links that ask for a new tab open one; right-clicking a link offers the
 * window's own menu; hovering a link shows a preview card.
 *
 * Every control is a core UI component — `iconButton` for the toolbar,
 * `searchBar` for the address field, `icon` for the glyphs — so the window
 * inherits the theme's button metrics, focus rings and accent for free. The
 * plugin's own stylesheet carries layout only (PLUGINS.md §19).
 *
 * Server state lives on the plugin side (`/api/browser/*`), so this file owns
 * presentation and local navigation history only.
 *
 * Navigation is *the frame's* history, not this file's. The proxy injects a
 * small bridge (see `shiny-filter`'s `inject.rs`) into every proxied document
 * that reports the real URL to this window and accepts back/forward/reload
 * commands. That is what makes the toolbar follow link clicks, redirects and
 * SPA navigation instead of only the URLs this file was told to load.
 */
import { apiFetch } from '/js/api.js';
import { openContextMenu } from '/js/contextMenu.js';
import { icon, iconButton, searchBar } from '/ui/index.js';

export const BROWSER_PLUGIN = 'browser';

const STATE_POLL_MS = 5000;

/** How many tabs may be open. Each is a live document, so this is bounded. */
const MAX_TABS = 8;

/** How long the pointer must rest on a link before its preview is fetched. */
const PREVIEW_DEBOUNCE_MS = 250;

let tileEl = null;
let tabstripEl = null;
let frameWrapEl = null;
let viewportEl = null;
let previewEl = null;
let addressEl = null;
let backBtn = null;
let forwardBtn = null;
let shieldEl = null;
let shieldCountEl = null;
let filterBtn = null;
let statusEl = null;

let proxyBase = null;
/** Whether ad filtering is paused (server-owned; this mirrors it). */
let filteringPaused = false;

/**
 * The window's tabs. Each owns its own iframe, its own server session id and
 * its own navigation history; only the active one is visible. `activeTabId`
 * always names one of them while the window is mounted.
 */
let tabs = [];
let activeTabId = null;
let tabSeq = 0;

/** Debounce/target for the hover preview. */
let previewTimer = null;

let pollTimer = null;
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

/* ── Server calls ─────────────────────────────────────────────── */

async function fetchState() {
  const res = await apiFetch('/api/browser/state');
  return res?.data || {};
}

/**
 * The proxy's own address, from the server.
 *
 * Fetched on every navigation rather than cached at mount, because it is a
 * **random port by design** (`ProxyConfig::default()` binds `127.0.0.1:0`) and
 * it changes on every server start. A window that remembered the old port from
 * before a restart showed the browser's own "connection refused" page on every
 * later navigation — the failure looked like a broken proxy but was a stale
 * address.
 */
async function refreshProxyBase() {
  try {
    const state = await fetchState();
    if (state.proxy_base) proxyBase = state.proxy_base;
    return state;
  } catch (_) {
    return null;
  }
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

/** How long a navigation may go without any load before it is a failure. */
const LOAD_WATCHDOG_MS = 5000;

function clearWatchdog(tab) {
  if (tab?.watchdog) clearTimeout(tab.watchdog);
  if (tab) tab.watchdog = null;
}

/** Create a tab, its iframe, and (unless a URL is given) its home surface. */
function createTab({ url = '', activate = true } = {}) {
  const tab = {
    id: `t${++tabSeq}`,
    sessionId: null,
    title: '',
    url: '',
    home: true,
    history: [],
    historyIndex: -1,
    attempt: 0,
    loaded: false,
    watchdog: null,
    frameEl: null,
    onError: null,
    onLoad: null,
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
  const frame = tab.frameEl;
  if (!frame) return;
  if (tab.onError) frame.removeEventListener('error', tab.onError);
  if (tab.onLoad) frame.removeEventListener('load', tab.onLoad);
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
  for (const t of tabs) t.frameEl.classList.toggle('is-active', t.id === id);
  if (addressEl && document.activeElement !== addressEl.input) {
    addressEl.input.value = tab?.url || '';
  }
  hidePreview();
  renderTabs();
  updateNavButtons();
}

/** The tab strip: one button per tab, plus a new-tab action. */
function renderTabs() {
  if (!tabstripEl) return;
  const nodes = [];
  for (const tab of tabs) {
    const el = document.createElement('div');
    el.className = 'browser-tab' + (tab.id === activeTabId ? ' is-active' : '');
    el.setAttribute('role', 'tab');
    el.setAttribute('aria-selected', String(tab.id === activeTabId));

    const label = document.createElement('button');
    label.type = 'button';
    label.className = 'browser-tab-label';
    label.textContent = tabTitle(tab);
    label.title = tab.url || 'New tab';
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

function buildFrame(tab) {
  const frame = document.createElement('iframe');
  frame.className = 'browser-frame';
  // The pages we proxy are the open web, not the theme: a white canvas is
  // correct here, and a sandbox is deliberately NOT used — many sites need
  // scripts, forms and same-origin storage to work at all, and the requests
  // are already filtered upstream.
  frame.setAttribute('referrerpolicy', 'no-referrer');
  frame.setAttribute('allow', 'clipboard-write; fullscreen');
  tab.onError = () => onFrameError(tab);
  tab.onLoad = () => onFrameLoad(tab);
  frame.addEventListener('error', tab.onError);
  frame.addEventListener('load', tab.onLoad);
  return frame;
}

/** Load a proxy URL in one tab's viewport. */
function loadFrame(tab, viewUrl) {
  if (!tab?.frameEl) return;
  tab.loaded = false;
  clearWatchdog(tab);
  tab.watchdog = setTimeout(() => {
    tab.watchdog = null;
    if (!tab.loaded) onFrameError(tab);
  }, LOAD_WATCHDOG_MS);
  // Real pages need their own origin: drop the idle-surface sandbox.
  tab.frameEl.removeAttribute('sandbox');
  tab.frameEl.removeAttribute('srcdoc');
  tab.frameEl.src = viewUrl;
}

function onFrameLoad(tab) {
  tab.loaded = true;
  clearWatchdog(tab);
}

async function navigateTo(input, { tab = activeTab(), retry = false } = {}) {
  const value = String(input || '').trim();
  if (!value || !tab) return;

  if (tab.id === activeTabId) setStatus('Loading…');
  let data;
  try {
    const res = await apiFetch('/api/browser/navigate', {
      method: 'POST',
      body: JSON.stringify({ session_id: tab.sessionId, input: value }),
    });
    data = res?.data;
  } catch (err) {
    if (tab.id === activeTabId) setStatus('Could not open that page');
    return;
  }
  if (!data?.view_url) {
    if (tab.id === activeTabId) setStatus('Could not open that page');
    return;
  }

  // The server hands back a URL on the proxy it is running *now*. If the window
  // is holding a base from an earlier server start, adopt the new one — a
  // mismatch is the exact cause of a "connection refused" viewport.
  const nextBase = data.proxy_base || (data.view_url || '').match(/^https?:\/\/[^/]+/)?.[0];
  if (nextBase) proxyBase = nextBase;

  tab.sessionId = data.session?.id || tab.sessionId;
  tab.home = false;
  recordLocation(tab, data.url || value);

  if (tab.id === activeTabId) setStatus('');
  // A fresh navigation gets its own retry budget; an error retry does not, or
  // a dead proxy would re-arm the budget forever (the bug the counter exists
  // to prevent).
  if (!retry) tab.attempt = 0;
  tab.attempt += 1;
  loadFrame(tab, data.view_url);
  if (tab.id === activeTabId) void refreshMetrics();
}

let lastMetrics = null;
let lastRules = 0;

async function refreshMetrics() {
  try {
    const res = await apiFetch('/api/browser/metrics');
    lastMetrics = res?.data?.metrics ?? null;
    lastRules = res?.data?.rules ?? 0;
    if (typeof res?.data?.filtering_paused === 'boolean') {
      filteringPaused = res.data.filtering_paused;
    }
    renderShield(lastMetrics, lastRules);
  } catch (_) {
    /* the proxy may be starting up; the shield just stays as it was */
  }
}

/**
 * Turn ad filtering off or back on, server-side.
 *
 * Some sites genuinely cannot work with their trackers removed, so this is the
 * escape hatch from a broken page. The proxy is what filters, so the state
 * lives there and every tab sees it.
 */
async function toggleFiltering() {
  const next = !filteringPaused;
  filteringPaused = next; // optimistic: the button must respond immediately
  renderShield(lastMetrics, lastRules);
  try {
    const res = await apiFetch('/api/browser/filter/toggle', {
      method: 'POST',
      body: JSON.stringify({ paused: next }),
    });
    filteringPaused = !!res?.data?.filtering_paused;
    setStatus(filteringPaused ? 'Ad blocking is off for every page' : '');
  } catch (_) {
    filteringPaused = !next; // rolled back: never claim a state we did not set
    setStatus('Could not change ad blocking');
  }
  renderShield(lastMetrics, lastRules);
}

/* ── Chrome ───────────────────────────────────────────────────── */

function setStatus(text) {
  if (!statusEl) return;
  statusEl.textContent = text || '';
  statusEl.classList.toggle('hidden', !text);
}

function renderShield(metrics, rules) {
  if (shieldCountEl) {
    const blocked = metrics?.blocked || 0;
    shieldCountEl.textContent = String(blocked);
    if (shieldEl) {
      shieldEl.title = rules
        ? `${blocked} requests blocked · ${rules.toLocaleString()} filter rules loaded`
        : `${blocked} requests blocked`;
      shieldEl.classList.toggle('is-active', blocked > 0 && !filteringPaused);
      shieldEl.classList.toggle('is-paused', filteringPaused);
      shieldEl.setAttribute(
        'aria-label',
        `${blocked} requests blocked — refresh the count`,
      );
    }
  }
  if (filterBtn) {
    filterBtn.classList.toggle('is-paused', filteringPaused);
    // Say what a click *does*, not what the current state is.
    const label = filteringPaused
      ? 'Ad blocking is off — click to turn it back on'
      : 'Ad blocking is on — click to turn it off for pages that break';
    filterBtn.title = label;
    filterBtn.setAttribute('aria-label', label);
    filterBtn.setAttribute('aria-pressed', String(filteringPaused));
  }
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

  shieldEl = document.createElement('button');
  shieldEl.type = 'button';
  shieldEl.className = 'browser-shield';
  shieldEl.title = 'Requests blocked';
  const shieldIcon = icon('ui/power', { size: 15 });
  shieldIcon.classList.add('browser-shield-icon');
  shieldCountEl = document.createElement('span');
  shieldCountEl.className = 'browser-shield-count';
  shieldCountEl.textContent = '0';
  shieldEl.append(shieldIcon, shieldCountEl);
  shieldEl.addEventListener('click', () => void refreshMetrics());

  // A live state needs a glyph that reads in both states. The theme has no
  // shield, so this is `ui/power`: whole when filtering is on, and the paused
  // state is carried by the button's own colour plus `aria-pressed` (never by
  // colour alone).
  filterBtn = iconButton({
    icon: 'ui/power',
    size: 'sm',
    label: 'Pause ad blocking',
    onClick: () => void toggleFiltering(),
  });
  filterBtn.classList.add('browser-filter-btn');

  bar.append(backBtn, forwardBtn, reloadBtn, homeBtn, addressEl, filterBtn, shieldEl);
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

  previewEl = document.createElement('div');
  previewEl.className = 'browser-preview hidden';
  wrap.appendChild(previewEl);

  return wrap;
}

/* ── Navigation ───────────────────────────────────────────────── */

/**
 * Recover from a viewport that could not reach the proxy.
 *
 * The proxy listens on a random loopback port that changes every time the
 * server starts, so a window left open across a restart holds a dead address
 * and every navigation lands on the browser's "connection refused" page. This
 * asks the server for the current address and retries once. If it fails again
 * the cause is something else (proxy not up yet, server restarting), so the
 * user gets a sentence instead of a bare browser error.
 */
function onFrameError(tab) {
  if (!tab || tab.url === '') return; // the idle surface is a srcdoc
  if (tab.attempt > 1) {
    if (tab.id === activeTabId) setStatus('Lost the connection to the filter proxy — reload the window');
    return;
  }
  if (tab.id === activeTabId) setStatus('Reconnecting to the filter proxy…');
  void (async () => {
    const state = await refreshProxyBase();
    const base = state?.proxy_base;
    if (!state?.ready || !base) {
      if (tab.id === activeTabId) setStatus('The filter proxy is not running — it starts with the server');
      return;
    }
    // Rebuild against the live proxy and hand the frame a URL on the new port.
    // The attempt counter belongs to the page, not to this retry, so a second
    // failure ends in the message above instead of looping forever.
    void navigateTo(tab.url, { tab, retry: true });
  })();
}

/**
 * Reconcile one tab's history with a URL the frame reports as current.
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

/** Ask the active frame to move through *its* history — no server round-trip. */
function go(delta) {
  const tab = activeTab();
  if (!tab?.frameEl?.contentWindow || !tab.url) return;
  const cmd = delta < 0 ? 'back' : 'forward';
  tab.frameEl.contentWindow.postMessage({ type: 'shiny:cmd', cmd }, '*');
}

function reload() {
  const tab = activeTab();
  if (!tab) return;
  if (!tab.url) {
    void showHome(tab);
    return;
  }
  if (tab.frameEl?.contentWindow) {
    tab.frameEl.contentWindow.postMessage({ type: 'shiny:cmd', cmd: 'reload' }, '*');
  }
}

function goHome() {
  void showHome(activeTab());
}

/* ── Home surface: related-news cards ─────────────────────────────
 *
 * `about:home` used to be a bare "type an address" panel. It is now the
 * browser's start page: a shelf of news chosen from what the user actually
 * searches for (`/api/browser/news`), with an explicit card click feeding the
 * ranking back (`/api/browser/news/click`).
 *
 * The surface is rendered inside the sandboxed idle iframe rather than in the
 * window's own DOM. That is deliberate: it is the same element real pages load
 * into, so there is no second layout to keep in sync, and the sandbox (no
 * `allow-same-origin`) means the start page cannot reach the app shell even
 * though it is built from data the server sent.
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

/**
 * The window's own menu for a right-clicked link.
 *
 * The frame cannot show a native menu that honours "open in new tab" (its
 * target is the OS browser, which would leave the filtered window), so the
 * shim hands the click up and the parent draws the menu.
 */
function showLinkMenu(tab, url, x, y) {
  if (!/^https?:\/\//.test(url)) return;
  const rect = tab?.frameEl?.getBoundingClientRect ? tab.frameEl.getBoundingClientRect() : null;
  const left = (rect?.left || 0) + (Number(x) || 0);
  const top = (rect?.top || 0) + (Number(y) || 0);
  openContextMenu(
    [
      { type: 'heading', label: 'Link' },
      {
        type: 'item',
        label: 'Open in new tab',
        icon: 'ui/launcher',
        onClick: () => openInNewTab(tab, url),
      },
      {
        type: 'item',
        label: 'Open in current tab',
        icon: 'ui/arrow-left',
        onClick: () => void navigateTo(url, { tab }),
      },
      { type: 'separator' },
      {
        type: 'item',
        label: 'Copy link',
        icon: 'ui/doc',
        onClick: () => copyText(url),
      },
    ],
    left,
    top,
  );
}

function copyText(text) {
  try {
    navigator.clipboard?.writeText(text).catch(() => {});
  } catch (_) {
    /* clipboard is best-effort */
  }
}

/* ── Link preview ─────────────────────────────────────────────── */

/**
 * A hovered link. Debounced so a pointer sweeping across a page does not fire
 * a request per anchor, and cancelled the moment it leaves the link.
 */
function previewHover(tab, url, rect) {
  if (!/^https?:\/\//.test(url)) return;
  if (tab.id !== activeTabId) return;
  if (previewTimer) clearTimeout(previewTimer);
  previewTimer = setTimeout(() => {
    previewTimer = null;
    void fetchPreview(tab, url, rect);
  }, PREVIEW_DEBOUNCE_MS);
}

function previewLeave() {
  if (previewTimer) clearTimeout(previewTimer);
  previewTimer = null;
  hidePreview();
}

async function fetchPreview(tab, url, rect) {
  try {
    const res = await apiFetch('/api/browser/preview', {
      method: 'POST',
      body: JSON.stringify({ url }),
    });
    const data = res?.data;
    if (!data || tab.id !== activeTabId) return;
    showPreview(data, rect);
  } catch (_) {
    /* no preview is a valid outcome */
  }
}

function showPreview(data, rect) {
  if (!previewEl) return;
  const title = escapeHtml(data.title || data.url || '');
  const site = escapeHtml(data.site || '');
  const description = data.description
    ? `<div class="browser-preview-desc">${escapeHtml(data.description)}</div>`
    : '';
  const image = data.image
    ? `<img class="browser-preview-img" src="${escapeHtml(data.image)}" alt="" referrerpolicy="no-referrer">`
    : '';
  previewEl.innerHTML = `<div class="browser-preview-body">
      ${image}
      <div class="browser-preview-text">
        ${site ? `<div class="browser-preview-site">${site}</div>` : ''}
        <div class="browser-preview-title">${title}</div>
        ${description}
      </div>
    </div>`;
  const frameRect = frameWrapEl?.getBoundingClientRect ? frameWrapEl.getBoundingClientRect() : null;
  const r = rect || {};
  const left = Math.max(8, (frameRect?.left || 0) + (Number(r.x) || 0));
  const top = Math.max(8, (frameRect?.top || 0) + (Number(r.y) || 0) + (Number(r.h) || 0) + 8);
  previewEl.style.left = `${left}px`;
  previewEl.style.top = `${top}px`;
  previewEl.classList.remove('hidden');
}

function hidePreview() {
  if (previewEl) previewEl.classList.add('hidden');
}

/**
 * Messages from the framed surfaces.
 *
 * `event.source` is the frame's WindowProxy, not the `<iframe>` element, so the
 * guard matches it against each tab's `contentWindow` — with more than one tab
 * open, the message must update the tab that sent it, never the active one.
 */
function onFrameMessage(event) {
  const data = event.data;
  if (!data || typeof data !== 'object') return;
  const tab = tabs.find((t) => t.frameEl && t.frameEl.contentWindow === event.source);
  if (!tab) return;

  if (data.type === 'shiny:location') {
    if (typeof data.title === 'string' && data.title) tab.title = data.title;
    if (typeof data.url === 'string' && data.url) recordLocation(tab, data.url);
    renderTabs();
  } else if (data.type === 'browser:open-card') {
    void onHomeCard(tab, String(data.url || ''), data.topic ? String(data.topic) : undefined);
  } else if (data.type === 'browser:refresh-news') {
    void showHome(tab);
  } else if (data.type === 'shiny:new-tab') {
    if (typeof data.url === 'string') openInNewTab(tab, data.url);
  } else if (data.type === 'shiny:link-menu') {
    showLinkMenu(tab, String(data.url || ''), data.x, data.y);
  } else if (data.type === 'shiny:hover-link') {
    previewHover(tab, String(data.url || ''), data.rect);
  } else if (data.type === 'shiny:leave-link') {
    previewLeave();
  }
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
    { type: 'separator' },
    {
      type: 'item',
      label: 'Refresh block count',
      icon: 'ui/power',
      disabled: !shieldEl,
      onClick: () => void refreshMetrics(),
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
  tileEl.append(tabstripEl, buildToolbar(), viewportEl);

  updateNavButtons();
  void (async () => {
    try {
      const state = await fetchState();
      proxyBase = state.proxy_base || null;
      filteringPaused = !!state.filtering_paused;
      lastMetrics = state.metrics ?? null;
      lastRules = state.rules ?? 0;
      renderShield(lastMetrics, lastRules);
      if (!proxyBase) {
        setStatus('Filter engine starting…');
        // One retry covers the plugin's proxy still warming up.
        setTimeout(async () => {
          const again = await fetchState().catch(() => null);
          if (again?.proxy_base) {
            proxyBase = again.proxy_base;
            renderShield(again.metrics, again.rules);
            setStatus('');
          }
        }, 1500);
      }
    } catch (_) {
      setStatus('Filter engine unavailable');
    }
    // One tab, on its home surface.
    createTab({});
  })();

  pollTimer = setInterval(() => void refreshMetrics(), STATE_POLL_MS);
  return tileEl;
}

export function unmountBrowserTile() {
  if (pollTimer) clearInterval(pollTimer);
  pollTimer = null;
  window.removeEventListener('message', onFrameMessage);
  wired = false;
  previewLeave();
  for (const tab of tabs) destroyTab(tab);
  tabs = [];
  activeTabId = null;
  tileEl?.remove();
  tileEl = null;
  tabstripEl = null;
  frameWrapEl = null;
  viewportEl = null;
  previewEl = null;
  addressEl = null;
  backBtn = null;
  forwardBtn = null;
  shieldEl = null;
  shieldCountEl = null;
  filterBtn = null;
  statusEl = null;
}

export function getBrowserTileElement() {
  return tileEl;
}

export function wireBrowserEvents() {
  if (wired) return;
  wired = true;
  window.addEventListener('artifact:saved', onArtifactSaved);
  // The home shelves and proxied pages talk back through postMessage; the
  // frame's navigation reports depend on it.
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
