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
import { icon, iconButton, searchBar } from '/ui/index.js';

export const BROWSER_PLUGIN = 'browser';

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
/** Whether ad filtering is paused (server-owned; this mirrors it). */
let filteringPaused = false;

/**
 * Navigation-recovery state: how many times the page currently in the frame
 * has been attempted. One retry is allowed, and the count belongs to the page
 * — resetting it inside the retry would loop forever (a bug this file shipped
 * for exactly one test run).
 */
let frameAttempt = 0;
/** Whether the current navigation reached the proxy at all. */
let frameLoaded = false;

/** Local navigation history, reconciled from the frame's own reports. */
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

/**
 * How long a navigation may go without any load at all before it is treated as
 * a failure.
 *
 * `error` does not fire for a cross-origin frame's network failure in every
 * engine (WebKit in particular is unreliable here), so the recovery cannot rest
 * on that event alone. The proxy always answers fast when it is up — it is on
 * loopback — so "nothing loaded at all" within this window means the address is
 * dead, not that the page is slow. Anything that *did* start loading cancels
 * the timer, so slow sites are never interrupted.
 */
const LOAD_WATCHDOG_MS = 5000;
let loadWatchdog = null;

function clearLoadWatchdog() {
  if (loadWatchdog) clearTimeout(loadWatchdog);
  loadWatchdog = null;
}

/** Load a proxy URL in the viewport. */
function loadFrame(viewUrl) {
  if (!frameEl) return;
  frameLoaded = false;
  clearLoadWatchdog();
  loadWatchdog = setTimeout(() => {
    loadWatchdog = null;
    // The frame never reached the proxy at all.
    if (!frameLoaded) onFrameError();
  }, LOAD_WATCHDOG_MS);
  // Real pages need their own origin: drop the idle-surface sandbox.
  frameEl.removeAttribute('sandbox');
  frameEl.removeAttribute('srcdoc');
  frameEl.src = viewUrl;
}

/** A load started: the proxy answered, so this navigation is alive. */
function onFrameLoad() {
  frameLoaded = true;
  clearLoadWatchdog();
}

async function navigateTo(input, { retry = false } = {}) {
  const value = String(input || '').trim();
  if (!value) return;

  setStatus('Loading…');
  let data;
  try {
    const res = await apiFetch('/api/browser/navigate', {
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

  // The server hands back a URL on the proxy it is running *now*. If the window
  // is holding a base from an earlier server start, adopt the new one — a
  // mismatch is the exact cause of a "connection refused" viewport.
  const nextBase = data.proxy_base || (data.view_url || '').match(/^https?:\/\/[^/]+/)?.[0];
  if (nextBase) proxyBase = nextBase;

  sessionId = data.session?.id || sessionId;
  recordLocation(data.url || value);

  setStatus('');
  // A fresh navigation gets its own retry budget; an error retry does not, or
  // a dead proxy would re-arm the budget forever (the bug the counter exists
  // to prevent).
  if (!retry) frameAttempt = 0;
  frameAttempt += 1;
  loadFrame(data.view_url);
  await refreshMetrics();
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
  const onPage = currentUrl !== '';
  if (backBtn) backBtn.disabled = !onPage || historyIndex <= 0;
  if (forwardBtn) forwardBtn.disabled = !onPage || historyIndex >= history.length - 1;
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

  frameEl = document.createElement('iframe');
  frameEl.className = 'browser-frame';
  // The pages we proxy are the open web, not the theme: a white canvas is
  // correct here, and a sandbox is deliberately NOT used — many sites need
  // scripts, forms and same-origin storage to work at all, and the requests
  // are already filtered upstream.
  frameEl.setAttribute('referrerpolicy', 'no-referrer');
  frameEl.setAttribute('allow', 'clipboard-write; fullscreen');
  // A navigation can fail before any page is reached — most often because the
  // proxy address the frame was pointed at no longer exists (see
  // `refreshProxyBase`). Without this the viewport shows the browser's own
  // "connection refused" page and the window has no way back.
  frameEl.addEventListener('error', onFrameError);
  frameEl.addEventListener('load', onFrameLoad);
  wrap.appendChild(frameEl);

  statusEl = document.createElement('div');
  statusEl.className = 'browser-status hidden';
  wrap.appendChild(statusEl);

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
function onFrameError() {
  if (!frameEl || currentUrl === '') return; // the idle surface is a srcdoc
  if (frameAttempt > 1) {
    setStatus('Lost the connection to the filter proxy — reload the window');
    return;
  }
  setStatus('Reconnecting to the filter proxy…');
  void (async () => {
    const state = await refreshProxyBase();
    const base = state?.proxy_base;
    if (!state?.ready || !base) {
      setStatus('The filter proxy is not running — it starts with the server');
      return;
    }
    // Rebuild against the live proxy and hand the frame a URL on the new port.
    // The attempt counter belongs to the page, not to this retry, so a second
    // failure ends in the message above instead of looping forever.
    void navigateTo(currentUrl, { retry: true });
  })();
}

/**
 * Reconcile this window's history with a URL the frame reports as current.
 *
 * The frame owns the real history (its back/forward is browser-native), so
 * this is a mirror for the address bar and for enabling the buttons: an
 * adjacent URL means the user went back or forward, anything else is a new
 * navigation and truncates the forward entries.
 */
function recordLocation(url) {
  if (!url) return;
  currentUrl = url;
  if (addressEl && document.activeElement !== addressEl.input) {
    addressEl.input.value = url;
  }
  if (history[historyIndex] !== url) {
    if (history[historyIndex + 1] === url) {
      historyIndex += 1;
    } else if (history[historyIndex - 1] === url) {
      historyIndex -= 1;
    } else {
      const existing = history.lastIndexOf(url);
      if (existing >= 0) {
        historyIndex = existing;
      } else {
        history = history.slice(0, historyIndex + 1);
        history.push(url);
        historyIndex = history.length - 1;
      }
    }
  }
  updateNavButtons();
}

/** Ask the frame to move through *its* history — no server round-trip. */
function go(delta) {
  if (!frameEl?.contentWindow || !currentUrl) return;
  const cmd = delta < 0 ? 'back' : 'forward';
  frameEl.contentWindow.postMessage({ type: 'shiny:cmd', cmd }, '*');
}

function reload() {
  if (!currentUrl) {
    void showHome();
    return;
  }
  if (frameEl?.contentWindow) {
    frameEl.contentWindow.postMessage({ type: 'shiny:cmd', cmd: 'reload' }, '*');
  }
}

function goHome() {
  void showHome();
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

/** The idle surface: the start page, with related news. */
async function showHome() {
  currentUrl = '';
  if (addressEl) addressEl.input.value = '';
  if (!frameEl) return;
  // The idle surface must not be same-origin with the app shell (it is
  // rendered from server data), so it is sandboxed without allow-same-origin.
  frameEl.removeAttribute('allow');
  frameEl.setAttribute('sandbox', 'allow-scripts');
  frameEl.removeAttribute('srcdoc');
  // Clear the previous page *before* awaiting the shelf, so going Home is
  // instant instead of leaving the last site on screen while it loads.
  frameEl.src = 'about:blank';
  setStatus('Loading recommendations…');

  let news = { cards: [], topics: [], personalized: false };
  try {
    news = await fetchNews();
  } catch (_) {
    news = { cards: [], topics: [], personalized: false, error: 'Could not load recommendations' };
  }
  // Re-check: the user may have navigated while the shelf was in flight.
  if (!frameEl || currentUrl) return;
  setStatus('');
  frameEl.src = 'about:blank';
  frameEl.srcdoc = homeDocument(news);
  // `srcdoc` counts as a navigation for history purposes only if we say so.
  history = [];
  historyIndex = -1;
  updateNavButtons();
}

/** A card in the home shelf was clicked. */
function onHomeCard(url, topic) {
  if (!url) return;
  // Best-effort: the click is a ranking signal, not a precondition for
  // navigating. A failed POST must not stop the page from opening.
  void apiFetch('/api/browser/news/click', {
    method: 'POST',
    body: JSON.stringify({ url, topic }),
  }).catch(() => {});
  void navigateTo(url);
}

/**
 * Messages from the framed surface.
 *
 * `event.source` is the frame's WindowProxy, not the `<iframe>` element, so the
 * guard must compare against `frameEl.contentWindow` (the old element compare
 * silently dropped every card click).
 */
function onFrameMessage(event) {
  const frameWindow = frameEl?.contentWindow;
  if (!frameWindow || event.source !== frameWindow) return;
  const data = event.data;
  if (!data || typeof data !== 'object') return;

  if (data.type === 'shiny:location') {
    // A proxied page reporting where it actually is (link click, redirect,
    // SPA navigation) — keep the address bar and history in step.
    if (typeof data.url === 'string' && data.url) recordLocation(data.url);
  } else if (data.type === 'browser:open-card') {
    void onHomeCard(String(data.url || ''), data.topic ? String(data.topic) : undefined);
  } else if (data.type === 'browser:refresh-news') {
    void showHome();
  }
}

/* ── AI-driven navigation ─────────────────────────────────────── */

/**
 * The AI asked for a page: load it in this window.
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
    currentUrl = ''; // this is a navigation, not a refresh of the home surface
    void navigateTo(url);
  }
}

/** Entries core splices into this window's right-click menu (PLUGINS.md §19). */
export function browserContextMenu() {
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

export function mountBrowserTile() {
  if (tileEl) return tileEl;
  ensureStylesheet();

  tileEl = document.createElement('section');
  tileEl.className = 'tile browser-tile';
  tileEl.dataset.plugin = BROWSER_PLUGIN;
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

export function unmountBrowserTile() {
  if (pollTimer) clearInterval(pollTimer);
  pollTimer = null;
  window.removeEventListener('message', onFrameMessage);
  wired = false;
  // Release the frame explicitly: an iframe left in the DOM keeps running
  // scripts and holding connections after its window is gone.
  if (frameEl) {
    frameEl.removeEventListener('error', onFrameError);
    frameEl.removeEventListener('load', onFrameLoad);
    frameEl.src = 'about:blank';
  }
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
  currentUrl = '';
  frameAttempt = 0;
  frameLoaded = false;
  clearLoadWatchdog();
}

export function getBrowserTileElement() {
  return tileEl;
}

export function wireBrowserEvents() {
  if (wired) return;
  wired = true;
  window.addEventListener('artifact:saved', onArtifactSaved);
  // Both the home shelf and proxied pages talk back through postMessage; the
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
