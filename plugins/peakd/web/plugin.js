/**
 * browser.js — the Browser plugin's window.
 *
 * A focused web viewport rendered *through PEAK'D!'s own filter proxy*: the
 * iframe's origin is the plugin's proxy, not the open internet, so every
 * request the page makes passes through the shared adblock engine. That is the
 * whole design — there is no request the engine cannot see, which is what
 * makes filtering here complete rather than best-effort.
 *
 * Chrome: back / forward / reload / home, an address bar that doubles as a
 * search box, and a shield showing how many requests were blocked.
 *
 * Server state lives on the plugin side (`/api/peakd/*`), so this file owns
 * presentation and local navigation history only.
 */
import { apiFetch } from '/js/api.js';
import { setIcon } from '/ui/index.js';

export const PEAKD_PLUGIN = 'peakd';

const STATE_POLL_MS = 5000;

let tileEl = null;
let frameEl = null;
let addressEl = null;
let backBtn = null;
let forwardBtn = null;
let shieldEl = null;
let shieldCountEl = null;
let filterBtn = null;
let statusEl = null;

let sessionId = null;
let proxyBase = null;
let currentUrl = '';
let title = '';
/** Whether ad filtering is paused (server-owned; this mirrors it). */
let filteringPaused = false;

/** Local navigation history (the iframe is cross-origin, so we keep our own). */
let history = [];
let historyIndex = -1;

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
  const id = 'peakd-plugin-css';
  if (document.getElementById(id)) return;
  const link = document.createElement('link');
  link.id = id;
  link.rel = 'stylesheet';
  link.href = '/plugins/peakd/plugin.css';
  document.head.appendChild(link);
}

/* ── Server calls ─────────────────────────────────────────────── */

async function fetchState() {
  const res = await apiFetch('/api/peakd/state');
  return res?.data || {};
}

async function navigateTo(input, { push = true } = {}) {
  const value = String(input || '').trim();
  if (!value) return;

  setStatus('Loading…');
  let data;
  try {
    const res = await apiFetch('/api/peakd/navigate', {
      method: 'POST',
      body: JSON.stringify({ session_id: sessionId, input: value }),
    });
    data = res?.data;
  } catch (err) {
    setStatus('Could not open that page');
    return;
  }
  if (!data?.view_url) {
    setStatus('Could not open that page');
    return;
  }

  sessionId = data.session?.id || sessionId;
  currentUrl = data.url || value;
  if (addressEl && document.activeElement !== addressEl.input) {
    addressEl.input.value = currentUrl;
  }

  if (push) {
    history = history.slice(0, historyIndex + 1);
    history.push(currentUrl);
    historyIndex = history.length - 1;
  }
  updateNavButtons();

  setStatus('');
  if (frameEl) {
    // Real pages need their own origin: drop the idle-surface sandbox.
    frameEl.removeAttribute('sandbox');
    frameEl.removeAttribute('srcdoc');
    frameEl.src = data.view_url;
  }
  await refreshMetrics();
}

let lastMetrics = null;
let lastRules = 0;

async function refreshMetrics() {
  try {
    const res = await apiFetch('/api/peakd/metrics');
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
 * The two shield states, drawn inline.
 *
 * The theme icon set has no shield glyph and core does not let a plugin add
 * one, so these are local SVG strings in the app mark's bold 2px style
 * (PLUGINS.md §19 "Icon style"). `currentColor` everywhere, so the button's
 * own colour still drives them — the paused state is communicated by the
 * slash *and* the button colour, never by colour alone.
 */
const SHIELD_ON = `<svg viewBox="0 0 24 24" width="15" height="15" fill="none"
  stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
  aria-hidden="true"><path d="M12 2.9l7 2.9v5.1c0 4.3-2.9 8.2-7 9.5-4.1-1.3-7-5.2-7-9.5V5.8z"
  fill="currentColor" stroke="none"/><path d="M8.9 12.1l2.2 2.2 4-4.4" stroke="var(--bg, #101014)"/></svg>`;

const SHIELD_OFF = `<svg viewBox="0 0 24 24" width="15" height="15" fill="none"
  stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
  aria-hidden="true"><path d="M12 2.9l7 2.9v5.1c0 4.3-2.9 8.2-7 9.5-4.1-1.3-7-5.2-7-9.5V5.8z"/>
  <path d="M4.8 4.1l14.4 15.8"/></svg>`;

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
    const res = await apiFetch('/api/peakd/filter/toggle', {
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
    }
  }
  if (filterBtn) {
    filterBtn.innerHTML = filteringPaused ? SHIELD_OFF : SHIELD_ON;
    filterBtn.classList.toggle('is-paused', filteringPaused);
    // Say what a click *does*, not what the current state is.
    filterBtn.title = filteringPaused
      ? 'Ad blocking is off — click to turn it back on'
      : 'Ad blocking is on — click to turn it off for pages that break';
    filterBtn.setAttribute('aria-pressed', String(filteringPaused));
    filterBtn.setAttribute('aria-label', filteringPaused ? 'Resume ad blocking' : 'Pause ad blocking');
  }
}

function updateNavButtons() {
  if (backBtn) backBtn.disabled = historyIndex <= 0;
  if (forwardBtn) forwardBtn.disabled = historyIndex >= history.length - 1;
}

/** A toolbar button built from the theme icon set. */
function toolButton(icon, label, onClick) {
  const btn = document.createElement('button');
  btn.type = 'button';
  btn.className = 'peakd-btn';
  btn.title = label;
  btn.setAttribute('aria-label', label);
  void setIcon(btn, icon, { size: 15 });
  btn.addEventListener('click', onClick);
  return btn;
}

function buildToolbar() {
  const bar = document.createElement('div');
  bar.className = 'peakd-bar';

  backBtn = toolButton('ui/arrow-left', 'Back', () => go(-1));
  forwardBtn = toolButton('ui/forward', 'Forward', () => go(1));
  const reloadBtn = toolButton('ui/refresh', 'Reload', () => reload());
  const homeBtn = toolButton('ui/launcher', 'Home', () => goHome());

  addressEl = document.createElement('form');
  addressEl.className = 'peakd-address';
  const input = document.createElement('input');
  input.type = 'text';
  input.className = 'peakd-input';
  input.placeholder = 'Search or enter an address';
  input.setAttribute('autocomplete', 'off');
  input.setAttribute('spellcheck', 'false');
  input.setAttribute('aria-label', 'Address');
  addressEl.appendChild(input);
  addressEl.addEventListener('submit', (e) => {
    e.preventDefault();
    input.blur();
    void navigateTo(input.value);
  });
  addressEl.input = input;

  shieldEl = document.createElement('button');
  shieldEl.type = 'button';
  shieldEl.className = 'peakd-shield';
  shieldEl.title = 'Requests blocked';
  const shieldIcon = document.createElement('span');
  shieldIcon.className = 'peakd-shield-icon';
  void setIcon(shieldIcon, 'ui/power', { size: 15 });
  shieldCountEl = document.createElement('span');
  shieldCountEl.className = 'peakd-shield-count';
  shieldCountEl.textContent = '0';
  shieldEl.append(shieldIcon, shieldCountEl);
  shieldEl.addEventListener('click', () => void refreshMetrics());

  filterBtn = document.createElement('button');
  filterBtn.type = 'button';
  filterBtn.className = 'peakd-filter-btn';
  filterBtn.innerHTML = SHIELD_ON;
  filterBtn.addEventListener('click', () => void toggleFiltering());

  bar.append(backBtn, forwardBtn, reloadBtn, homeBtn, addressEl, filterBtn, shieldEl);
  return bar;
}

function buildViewport() {
  const wrap = document.createElement('div');
  wrap.className = 'peakd-viewport';

  frameEl = document.createElement('iframe');
  frameEl.className = 'peakd-frame';
  // The pages we proxy are the open web, not the theme: a white canvas is
  // correct here, and a sandbox is deliberately NOT used — many sites need
  // scripts, forms and same-origin storage to work at all, and the requests
  // are already filtered upstream.
  frameEl.setAttribute('referrerpolicy', 'no-referrer');
  // Not sandboxed on purpose: real sites need scripts, forms, storage and
  // same-origin requests to work, and every request they make is already
  // filtered by the proxy that serves them.
  frameEl.setAttribute('allow', 'clipboard-write; fullscreen');
  wrap.appendChild(frameEl);

  statusEl = document.createElement('div');
  statusEl.className = 'peakd-status hidden';
  wrap.appendChild(statusEl);

  return wrap;
}

/* ── Navigation ───────────────────────────────────────────────── */

function go(delta) {
  const next = historyIndex + delta;
  if (next < 0 || next >= history.length) return;
  historyIndex = next;
  const url = history[historyIndex];
  currentUrl = url;
  if (addressEl) addressEl.input.value = url;
  updateNavButtons();
  void navigateTo(url, { push: false });
}

function reload() {
  if (!frameEl) return;
  // Re-issue through the server so the page comes back filtered and counted.
  if (currentUrl) void navigateTo(currentUrl, { push: false });
  else frameEl.src = frameEl.src;
}

function goHome() {
  void showHome();
}

/** The idle surface: what to do, and how filtering is doing. */
async function showHome() {
  currentUrl = '';
  if (addressEl) addressEl.input.value = '';
  setStatus('');
  if (!frameEl) return;
  // The idle surface must not be same-origin with the app shell (it is
  // inert), so it is rendered sandboxed without allow-same-origin.
  frameEl.removeAttribute('allow');
  frameEl.setAttribute('sandbox', 'allow-scripts');
  frameEl.srcdoc = `<!doctype html><html><head><meta charset="utf-8">
<style>
  html,body{height:100%;margin:0}
  body{display:flex;align-items:center;justify-content:center;
       background:var(--bg,#101014);color:var(--text,#eee);
       font-family:var(--font-body,system-ui,sans-serif)}
  .h{text-align:center;max-width:26rem;padding:1.5rem}
  .t{font-size:1.05rem;font-weight:600;margin-bottom:.4rem}
  .s{opacity:.7;font-size:.85rem;line-height:1.5}
</style></head><body><div class="h">
  <div class="t">Peak'd Browser</div>
  <div class="s">Type an address or a search above.<br>
  Pages load through PEAK'D!'s ad blocker.</div>
</div></body></html>`;
  // `srcdoc` counts as a navigation for history purposes only if we say so.
  history = [];
  historyIndex = -1;
  updateNavButtons();
}

/* ── AI-driven navigation ─────────────────────────────────────── */

function onArtifactSaved(event) {
  const art = event?.detail;
  if (!art || art.artifact_type !== 'browser_page') return;
  const url = art?.payload?.url || art?.params?.url || art?.payload?.view_url;
  if (url) void navigateTo(url);
}

/** Entries core splices into this window's right-click menu (PLUGINS.md §19). */
export function peakdContextMenu() {
  return [
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
      disabled: !frameEl,
      onClick: () => reload(),
    },
    {
      type: 'item',
      label: 'Home',
      icon: 'ui/launcher',
      disabled: !tileEl,
      onClick: () => void showHome(),
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

export function mountPeakdTile() {
  if (tileEl) return tileEl;
  ensureStylesheet();

  tileEl = document.createElement('section');
  tileEl.className = 'tile peakd-tile';
  tileEl.dataset.plugin = PEAKD_PLUGIN;
  tileEl.append(buildToolbar(), buildViewport());

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
    await showHome();
  })();

  pollTimer = setInterval(() => void refreshMetrics(), STATE_POLL_MS);
  return tileEl;
}

export function unmountPeakdTile() {
  if (pollTimer) clearInterval(pollTimer);
  pollTimer = null;
  // Release the frame explicitly: an iframe left in the DOM keeps running
  // scripts and holding connections after its window is gone.
  if (frameEl) frameEl.src = 'about:blank';
  tileEl?.remove();
  tileEl = null;
  frameEl = null;
  addressEl = null;
  backBtn = null;
  forwardBtn = null;
  shieldEl = null;
  shieldCountEl = null;
  filterBtn = null;
  statusEl = null;
  history = [];
  historyIndex = -1;
}

export function getPeakdTileElement() {
  return tileEl;
}

export function wirePeakdEvents() {
  if (wired) return;
  wired = true;
  window.addEventListener('artifact:saved', onArtifactSaved);
}

export default {
  name: 'peakd',
  icon: 'ui/launcher',
  mount: mountPeakdTile,
  unmount: unmountPeakdTile,
  getElement: getPeakdTileElement,
  wireEvents: wirePeakdEvents,
  contextMenu: peakdContextMenu,
};
