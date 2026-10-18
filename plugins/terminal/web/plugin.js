/**
 * terminal.js — the Terminal plugin's window surface.
 *
 * A real login shell on the host, rendered with xterm.js. The server owns the
 * PTY (see plugins/terminal/src/pty.rs); this module is only the glass:
 *
 *   mount  → build the tile, load xterm from `vendor/`, restore the tab list
 *            (one PTY session per tab) from `sessionStorage`
 *   stream → one `/api/terminal/events` (SSE) response for the whole window,
 *            with every session's frames tagged by id — a browser allows only
 *            a handful of connections per origin (Chromium: six) and the app
 *            already holds long-lived streams of its own, so a stream per tab
 *            starved later tabs. Read with a streaming fetch so the Bearer
 *            token goes in the header — `EventSource` cannot send headers,
 *            and its cookie-only auth breaks wherever the session cookie is
 *            missing (e.g. a kiosk webview that restores localStorage but not
 *            its cookie jar)
 *   input  → keystrokes are POSTed to `/api/terminal/input`
 *   resize → the fit addon measures the window, `/api/terminal/resize` tells
 *            the PTY, so `top`, `vim` and friends get the right width
 *   tabs   → the strip is the window's top bar (`data-window-bar`, like the
 *            browser's): one tab per session, `+` opens another, a tab's X (or
 *            its shell exiting) closes it, and the last tab closing respawns.
 *            The label is the shell's OSC title, else `Terminal N`.
 *   render → WebGL where the platform allows it, else canvas, else the built-in
 *            DOM renderer (see `selectRenderer`) — an agent TUI redrawing in
 *            here is what makes the renderer choice matter
 *
 * Closing the window does NOT kill the shells: the tab list lives in
 * `sessionStorage`, so reopening (or reloading the page) reattaches every tab
 * and replays recent output.
 */

import { controlButton, icon, iconButton, toast } from '../../ui/index.js';
import * as coreApi from '../../js/api.js';
import { t, tp } from '../../js/i18n.js';
import { copyText, readClipboardText } from '../../js/clipboard.js';

const { apiFetch, getToken } = coreApi;

export const TERMINAL_PLUGIN = 'terminal';

const VENDOR = `/plugins/${TERMINAL_PLUGIN}/vendor`;
const STORAGE_KEY = 'terminal.tabs';
const LEGACY_STORAGE_KEY = 'terminal.sessionId';

/** The strip's cap — the server also caps live PTY sessions per user. */
const MAX_TABS = 8;

let tileEl = null;
let tabstripEl = null;
let statusEl = null;

let assetsPromise = null;

/**
 * The window's tabs. Each owns one xterm and one PTY session on the server;
 * only the active one's host is visible. `activeTabId` always names one of
 * them while the window is mounted.
 */
let tabs = [];
let activeTabId = null;
let tabSeq = 0;

// One SSE response carries every tab's output (`/api/terminal/events`). A
// browser allows only a handful of connections per origin — Chromium: six —
// and this app already holds long-lived streams of its own (the top-bar
// chips), so a stream per tab ran the pool dry at the second tab: its frames
// never arrived and its input POSTs queued behind the limit. One tagged
// stream per window cannot.
let streamAbort = null;
let reconnectTimer = null;

// Output is coalesced before it reaches xterm. A TUI redraws continuously
// (the agent UI writes ~60 times a second), and xterm repaints the grid for
// every write it processes, so the repaint count — not the byte count — is
// what costs a core on a large terminal. Merging the frames that arrive
// inside one tick keeps the final state and roughly halves the repaints.
const OUT_FLUSH_MS = 33;

/** Reconnect delay after a dropped stream (matches EventSource's retry feel). */
const RECONNECT_DELAY = 1500;

/* ── assets ───────────────────────────────────────────────────── */

function loadScript(src) {
  return new Promise((resolve, reject) => {
    const el = document.createElement('script');
    el.src = src;
    el.async = false;
    el.onload = () => resolve();
    el.onerror = () => reject(new Error(`failed to load ${src}`));
    document.head.appendChild(el);
  });
}

function ensureAssets() {
  if (!assetsPromise) {
    assetsPromise = (async () => {
      if (!document.querySelector(`link[data-terminal-xterm]`)) {
        const css = document.createElement('link');
        css.rel = 'stylesheet';
        css.href = `${VENDOR}/xterm.css`;
        css.dataset.terminalXterm = '1';
        document.head.appendChild(css);
      }
      if (!window.Terminal) await loadScript(`${VENDOR}/xterm.js`);
      if (!window.FitAddon) await loadScript(`${VENDOR}/addon-fit.js`);
      // Renderers, best first, each optional. WebGL draws the whole grid from
      // a glyph atlas in a few draw calls; canvas repaints changed cells with
      // Canvas2D `fillText`; the built-in DOM renderer rebuilds text nodes for
      // every changed cell. It only matters under load: an agent TUI that
      // redraws continuously costs roughly a full core on the canvas/DOM paths
      // in this QtWebEngine kiosk, and a few percent on WebGL. A load failure
      // simply drops to the next renderer down.
      if (!window.WebglAddon) {
        try {
          await loadScript(`${VENDOR}/addon-webgl.js`);
        } catch (_) {
          /* canvas/DOM fallback */
        }
      }
      if (!window.CanvasAddon) {
        try {
          await loadScript(`${VENDOR}/addon-canvas.js`);
        } catch (_) {
          /* DOM renderer fallback */
        }
      }
    })().catch((err) => {
      assetsPromise = null;
      throw err;
    });
  }
  return assetsPromise;
}

/* ── helpers ──────────────────────────────────────────────────── */

async function post(path, body) {
  return apiFetch(path, { method: 'POST', body: JSON.stringify(body || {}) });
}

function b64ToBytes(b64) {
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i += 1) out[i] = bin.charCodeAt(i);
  return out;
}

function accentColor() {
  const value = getComputedStyle(document.documentElement).getPropertyValue('--accent');
  return value.trim() || '#8ab4ff';
}

/** Read a themed custom property off the tile (falling back to the root),
 *  so the xterm palette follows `--terminal-bg` / `--terminal-fg`. */
function tileVar(name, fallback) {
  const el = tileEl || document.documentElement;
  const value = getComputedStyle(el).getPropertyValue(name);
  return value.trim() || fallback;
}

function terminalTheme() {
  return {
    background: tileVar('--terminal-bg', '#0b0b0c'),
    foreground: tileVar('--terminal-fg', '#d8d8dc'),
    cursor: accentColor(),
    selectionBackground: 'rgba(255,255,255,.18)',
  };
}

function setStatus(text) {
  if (statusEl) statusEl.textContent = text;
}

/** Update a tab's status line; only the active tab's text is on screen. */
function updateTabStatus(tab, text) {
  tab.status = text;
  if (tab.id === activeTabId) setStatus(text);
}

/* ── tile DOM ─────────────────────────────────────────────────── */

function buildTile() {
  tileEl = document.createElement('section');
  tileEl.className = 'tile terminal-tile';
  tileEl.dataset.plugin = TERMINAL_PLUGIN;

  tabstripEl = buildTabStrip();

  statusEl = document.createElement('div');
  statusEl.className = 'terminal-status';
  statusEl.textContent = t('plugin.terminal.status.starting', null, 'starting terminal…');

  tileEl.append(tabstripEl, statusEl);
  return tileEl;
}

function buildTabStrip() {
  const strip = document.createElement('div');
  strip.className = 'terminal-tabstrip';
  strip.setAttribute('role', 'tablist');
  strip.setAttribute('aria-label', t('plugin.terminal.tabs.label', null, 'Terminal tabs'));
  // Nominate the strip as the window's top bar: the shared chrome folds it into
  // the header row next to the close/fullscreen controls (see `data-window-bar`
  // in web/js/tiles.js), so the tabs and the new-tab `+` live in the title bar
  // and the centered "Terminal" label is dropped — the browser's arrangement.
  strip.dataset.windowBar = '';
  return strip;
}

function activeTab() {
  return tabs.find((t) => t.id === activeTabId) || null;
}

/** The tab's label: the shell's OSC title when it sets one, else its number. */
function tabLabel(tab) {
  return tab.title || t('plugin.terminal.tab.label', { n: tab.seq }, 'Terminal {n}');
}

/** The tab strip: one button per tab, plus a new-tab action. */
function renderTabs() {
  if (!tabstripEl) return;
  const nodes = [];
  for (const tab of tabs) {
    const el = document.createElement('div');
    el.className = 'terminal-tab' + (tab.id === activeTabId ? ' is-active' : '');
    el.setAttribute('role', 'tab');
    el.setAttribute('aria-selected', String(tab.id === activeTabId));

    const label = controlButton();
    label.type = 'button';
    label.className = 'terminal-tab-label';
    const text = document.createElement('span');
    text.className = 'terminal-tab-text';
    text.textContent = tabLabel(tab);
    label.appendChild(text);
    label.title = tab.shell ? `${tab.shell} — ${tab.cwd}` : tabLabel(tab);
    label.addEventListener('click', () => setActiveTab(tab.id));

    const close = controlButton();
    close.type = 'button';
    close.className = 'terminal-tab-close';
    close.setAttribute('aria-label', t('plugin.terminal.tab.close', { name: tabLabel(tab) }, 'Close {name}'));
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
    label: t('plugin.terminal.tab.new', null, 'New tab'),
    onClick: () => void createTab({}),
  });
  add.classList.add('terminal-newtab');
  nodes.push(add);
  tabstripEl.replaceChildren(...nodes);
}

/* ── per-tab terminal ─────────────────────────────────────────── */

function createTerminalFor(tab) {
  const term = new window.Terminal({
    cursorBlink: true,
    fontFamily: 'ui-monospace, SFMono-Regular, Menlo, Consolas, monospace',
    fontSize: 13,
    scrollback: 5000,
    theme: terminalTheme(),
  });
  tab.term = term;
  tab.fitAddon = new window.FitAddon.FitAddon();
  term.loadAddon(tab.fitAddon);
  term.open(tab.hostEl);
  selectRenderer(tab);

  term.onData((data) => queueInput(tab, data));
  // Ctrl/Cmd+C and Ctrl/Cmd+V have to be decided by the terminal, not the
  // browser: without this, Ctrl+C is always a SIGINT (xterm sends \x03) and a
  // selection cannot be copied — xterm's WebGL selection is not a DOM
  // selection, so the browser's own copy would find nothing.
  term.attachCustomKeyEventHandler((event) => onTerminalKey(tab, event));
  // The shell's own title escape (OSC 0/2) is the tab label.
  term.onTitleChange((title) => {
    tab.title = (title || '').trim();
    renderTabs();
  });

  tab.observer = new ResizeObserver(() => onHostResized(tab));
  tab.observer.observe(tab.hostEl);

  fit(tab);
}

/**
 * Load the canvas renderer as the fallback below WebGL. Idempotent, and a
 * no-op when the addon is missing or refuses to load — whatever renderer is
 * already active (the DOM one) stays in place.
 */
function useCanvasRenderer(tab) {
  if (tab.canvasAddon || !tab.term || !window.CanvasAddon) return;
  try {
    tab.canvasAddon = new window.CanvasAddon.CanvasAddon();
    tab.term.loadAddon(tab.canvasAddon);
  } catch (_) {
    tab.canvasAddon = null; // keep the DOM renderer
  }
}

/**
 * Pick the fastest renderer the platform can give us: WebGL, then canvas,
 * then the built-in DOM renderer (see `ensureAssets`). A lost WebGL context
 * demotes to canvas rather than leaving a blank terminal.
 */
function selectRenderer(tab) {
  if (!tab.term) return;
  if (window.WebglAddon) {
    try {
      const webgl = new window.WebglAddon.WebglAddon();
      webgl.onContextLoss(() => {
        try {
          webgl.dispose();
        } catch (_) {
          /* already disposed */
        }
        useCanvasRenderer(tab);
        console.info('terminal: WebGL context lost — renderer =',
          tab.canvasAddon ? 'canvas' : 'dom');
      });
      tab.term.loadAddon(webgl);
      console.info('terminal: renderer = webgl');
      return;
    } catch (err) {
      console.info('terminal: WebGL unavailable, trying canvas:', err?.message ?? err);
    }
  }
  useCanvasRenderer(tab);
  console.info('terminal: renderer =', tab.canvasAddon ? 'canvas' : 'dom');
}

function fit(tab) {
  try {
    tab.fitAddon?.fit();
  } catch (_) {
    /* hidden or zero-sized window — harmless */
  }
}

function onHostResized(tab) {
  if (tab.id !== activeTabId) return; // hidden hosts measure 0×0
  fit(tab);
  if (!tab.term || !tab.sessionId) return;
  window.clearTimeout(tab.resizeTimer);
  tab.resizeTimer = window.setTimeout(() => {
    if (!tab.sessionId || tab.id !== activeTabId) return;
    void post('/api/terminal/resize', { session: tab.sessionId, cols: tab.term.cols, rows: tab.term.rows }).catch(() => {});
  }, 150);
}

function queueInput(tab, data) {
  tab.inputQueue.push(data);
  if (tab.flushTimer) return;
  tab.flushTimer = setTimeout(() => void flushInput(tab), 8);
}

async function flushInput(tab) {
  tab.flushTimer = null;
  const data = tab.inputQueue.join('');
  tab.inputQueue = [];
  if (!data || !tab.sessionId) return;
  try {
    await post('/api/terminal/input', { session: tab.sessionId, data });
  } catch (_) {
    /* the stream will report the session state */
  }
}

/* ── clipboard ────────────────────────────────────────────────── */

/**
 * Ctrl/Cmd+C copies the selection when there is one and stays a SIGINT
 * otherwise — the standard terminal trade-off. Ctrl/Cmd+V pastes. Shift is
 * not excluded, so the classic Ctrl+Shift+C / Ctrl+Shift+V pair works too.
 *
 * Returning false tells xterm the key was handled: it must not reach the PTY,
 * which is what keeps a copy from interrupting the foreground process.
 */
function onTerminalKey(tab, event) {
  if (event.type !== 'keydown') return true; // keypress repeats pass through
  if (!(event.ctrlKey || event.metaKey) || event.altKey) return true;
  const key = (event.key || '').toLowerCase();
  if (key === 'c') return copySelection(tab) ? false : true;
  if (key === 'v') {
    void pasteFromClipboard(tab);
    return false;
  }
  return true;
}

/** Copy the tab's current xterm selection through the central clipboard service. */
function copySelection(tab) {
  const selection = tab?.term?.getSelection?.() || '';
  if (!selection) return false;
  void copyText(selection, { source: TERMINAL_PLUGIN });
  return true;
}

/**
 * Paste clipboard text into the tab's shell. `term.paste` honours bracketed-paste
 * mode (vim/tmux see a paste instead of a burst of typed keys) and flows
 * through `onData` like any other input.
 */
async function pasteFromClipboard(tab) {
  const text = await readClipboardText();
  if (!text) {
    toast(t('plugin.terminal.toast.clipboardUnavailable', null, 'Clipboard unavailable'), { type: 'error' });
    return;
  }
  tab?.term?.paste(text);
}

/* ── tabs ─────────────────────────────────────────────────────── */

function createTab({ sessionId = null, activate = true } = {}) {
  if (tabs.length >= MAX_TABS) {
    toast(tp('plugin.terminal.toast.tabLimit', MAX_TABS), { type: 'error' });
    return null;
  }
  const seq = ++tabSeq;
  const tab = {
    id: `t${seq}`,
    seq,
    sessionId: null,
    title: '',
    shell: '',
    cwd: '',
    status: t('plugin.terminal.status.connecting', null, 'connecting…'),
    term: null,
    fitAddon: null,
    canvasAddon: null,
    hostEl: null,
    observer: null,
    resizeTimer: null,
    resyncing: false,
    inputQueue: [],
    flushTimer: null,
    outQueue: [],
    outBytes: 0,
    outTimer: null,
  };
  tab.hostEl = document.createElement('div');
  tab.hostEl.className = 'terminal-host';
  tileEl.insertBefore(tab.hostEl, statusEl);
  tabs.push(tab);
  // Activate before opening the terminal: xterm measures its font from the
  // live DOM, and a terminal opened inside a `display: none` host would
  // measure 0×0 and then refuse to fit (see setActiveTab). With the tab
  // activated first, the host is visible by the time xterm opens.
  if (activate) setActiveTab(tab.id);
  createTerminalFor(tab);
  if (activate) {
    onHostResized(tab);
    tab.term?.focus?.();
  } else {
    renderTabs();
  }
  void attachSession(tab, sessionId);
  return tab;
}

/** Tear a tab down: timers, observer, xterm, host. */
function destroyTab(tab) {
  if (tab.observer) {
    tab.observer.disconnect();
    tab.observer = null;
  }
  window.clearTimeout(tab.flushTimer);
  window.clearTimeout(tab.resizeTimer);
  if (tab.outTimer !== null) window.clearTimeout(tab.outTimer);
  tab.flushTimer = null;
  tab.resizeTimer = null;
  tab.outTimer = null;
  tab.inputQueue = [];
  tab.outQueue = [];
  tab.outBytes = 0;
  try {
    tab.term?.dispose();
  } catch (_) {
    /* already gone */
  }
  tab.term = null;
  tab.fitAddon = null;
  tab.canvasAddon = null;
  tab.hostEl?.remove();
  tab.hostEl = null;
}

function closeTab(id) {
  const idx = tabs.findIndex((t) => t.id === id);
  if (idx < 0) return;
  const [tab] = tabs.splice(idx, 1);
  const wasActive = activeTabId === id;
  const session = tab.sessionId;
  destroyTab(tab);
  // Best-effort: end the shell server-side. A failure here cannot stop the tab
  // from closing.
  if (session) {
    void post('/api/terminal/close', { session }).catch(() => {});
  }

  if (wasActive) {
    activeTabId = null;
    const next = tabs[idx] || tabs[idx - 1] || null;
    if (next) {
      setActiveTab(next.id);
    } else {
      // The window always keeps one shell: the last tab closing (its X, or a
      // shell exiting) opens a fresh one.
      renderTabs();
      createTab({});
      persist();
      return;
    }
  } else {
    renderTabs();
  }
  persist();
}

function setActiveTab(id) {
  const tab = tabs.find((t) => t.id === id) || null;
  if (!tab) return;
  activeTabId = id;
  for (const t of tabs) t.hostEl?.classList.toggle('is-active', t.id === id);
  const term = tab.term;
  if (term) {
    // A terminal opened while its host was hidden never measured its font
    // (xterm reads 0×0 from the unlaid-out DOM) and `FitAddon` then refuses to
    // fit it — `proposeDimensions()` bails while `cell.width` is 0. `resize`
    // with the current size makes xterm take the measurement when its size is
    // invalid, and only then can fit() size it to the host.
    try {
      term.resize(term.cols, term.rows);
    } catch (_) {
      /* not opened yet */
    }
    fit(tab);
    try {
      term.refresh(0, (term.rows || 1) - 1);
    } catch (_) {
      /* refresh is best-effort */
    }
  }
  onHostResized(tab);
  setStatus(tab.status || '');
  persist();
  renderTabs();
  // Clicking a tab (or the `+`) must leave the keyboard in the shell, like
  // every terminal app — and renderTabs() just replaced the button that held
  // focus. Focus after the rebuild so nothing steals it back.
  term?.focus?.();
}

/* ── persistence ──────────────────────────────────────────────── */

/** Remember which sessions the tabs hold, so a remount reattaches them. */
function persist() {
  const current = activeTab();
  try {
    sessionStorage.setItem(STORAGE_KEY, JSON.stringify({
      ids: tabs.map((t) => t.sessionId).filter(Boolean),
      active: current?.sessionId || null,
    }));
  } catch (_) {
    /* storage full or blocked — the tabs just won't restore */
  }
}

/** The stored tab list, migrating the pre-tabs single-session key. */
function readStoredTabs() {
  let stored = null;
  try {
    stored = JSON.parse(sessionStorage.getItem(STORAGE_KEY) || 'null');
  } catch (_) {
    stored = null;
  }
  const ids = Array.isArray(stored?.ids)
    ? stored.ids.filter((id) => typeof id === 'string' && id)
    : [];
  const active = typeof stored?.active === 'string' ? stored.active : null;
  const legacy = sessionStorage.getItem(LEGACY_STORAGE_KEY);
  if (legacy) {
    sessionStorage.removeItem(LEGACY_STORAGE_KEY);
    if (!ids.length) return { ids: [legacy], active: legacy };
  }
  return { ids, active };
}

async function restoreTabs() {
  const { ids, active } = readStoredTabs();
  for (const id of ids) {
    if (!tileEl) return; // unmounted while restoring
    createTab({ sessionId: id, activate: false });
  }
  if (!tabs.length) {
    createTab({});
    return;
  }
  const target = tabs.find((t) => t.sessionId === active) || tabs[0];
  setActiveTab(target.id);
}

/* ── session lifecycle ────────────────────────────────────────── */

/** Give a tab a PTY: reattach to `stored`, or start a fresh shell. */
async function attachSession(tab, stored = null) {
  if (stored) {
    tab.sessionId = stored;
    updateTabStatus(tab, t('plugin.terminal.status.connecting', null, 'connecting…'));
    return; // the window stream replays it — or answers `gone`
  }
  const dims = tab.fitAddon?.proposeDimensions?.() || null;
  let res;
  try {
    res = await post('/api/terminal/sessions', {
      cols: dims?.cols || tab.term?.cols || 80,
      rows: dims?.rows || tab.term?.rows || 24,
    });
  } catch (err) {
    if (!tabs.includes(tab)) return; // closed while starting
    toast(err?.message || t('plugin.terminal.toast.startFailed', null, 'Terminal: could not start a shell'), { type: 'error' });
    if (tabs.length > 1) {
      closeTab(tab.id);
      return;
    }
    updateTabStatus(tab, t('plugin.terminal.status.startFailed', null, 'could not start a shell'));
    return;
  }
  if (!tabs.includes(tab)) return;
  tab.sessionId = res?.data?.id || null;
  if (!tab.sessionId) {
    updateTabStatus(tab, t('plugin.terminal.status.startFailed', null, 'could not start a shell'));
    return;
  }
  updateTabStatus(tab, t('plugin.terminal.status.connecting', null, 'connecting…'));
  persist();
}

function connect() {
  stopStream();
  const ctrl = new AbortController();
  streamAbort = ctrl;
  void runStream(ctrl);
}

/** Close the window stream and cancel a pending reconnect. */
function stopStream() {
  window.clearTimeout(reconnectTimer);
  reconnectTimer = null;
  const ctrl = streamAbort;
  streamAbort = null;
  if (ctrl) {
    try {
      ctrl.abort();
    } catch (_) {
      /* already aborted */
    }
  }
}

function requestStream(path, options) {
  if (typeof coreApi.apiRequest === 'function') return coreApi.apiRequest(path, options);
  // Preserve hot-install compatibility with an already-open older core page.
  const token = getToken();
  return fetch(path, { ...options, headers: token ? { Authorization: `Bearer ${token}` } : {} });
}

/**
 * Read the window's SSE frames off `/api/terminal/events` with a streaming
 * fetch — one connection for every tab (see `streamAbort`).
 * `EventSource` would be the obvious tool, but it cannot set an
 * `Authorization` header — and the session cookie it falls back on is not
 * always there (the kiosk webview keeps localStorage but not its cookies, so
 * the app is signed in while `EventSource` gets a 401 and dies with
 * `readyState === CLOSED`). A fetch body reader authenticates exactly like
 * every other call and reports real HTTP statuses.
 */
async function runStream(ctrl) {
  const expect = tabs.map((t) => t.sessionId).filter(Boolean).join(',');
  let res;
  try {
    const query = expect ? `?expect=${encodeURIComponent(expect)}` : '';
    res = await requestStream(`/api/terminal/events${query}`, {
      cache: 'no-store',
      signal: ctrl.signal,
    });
  } catch (_) {
    if (ctrl.signal.aborted || streamAbort !== ctrl) return;
    scheduleReconnect();
    return;
  }
  if (ctrl.signal.aborted || streamAbort !== ctrl) return;

  if (!res.ok || !res.body) {
    if (ctrl.signal.aborted || streamAbort !== ctrl) return;
    if (res.status === 401) {
      const tab = activeTab();
      if (tab) updateTabStatus(tab, t('plugin.terminal.status.expired', null, 'session expired — sign in again'));
      scheduleReconnect(5000);
      return;
    }
    scheduleReconnect();
    return;
  }

  const reader = res.body.getReader();
  const decoder = new TextDecoder();
  let buffer = '';
  try {
    for (;;) {
      const { value, done } = await reader.read();
      if (done) break;
      buffer += decoder.decode(value, { stream: true });
      // Consume frames by index and slice the remainder once: reslicing the
      // whole buffer per frame is quadratic, and a busy TUI can pack many
      // frames into one read.
      let start = 0;
      let end;
      while ((end = buffer.indexOf('\n\n', start)) !== -1) {
        const frame = buffer.slice(start, end);
        start = end + 2;
        const line = frame.split('\n').find((l) => l.startsWith('data:'));
        if (line) handleFrame(line.slice(5).trim());
        if (!tileEl) {
          stopStream();
          return;
        }
      }
      if (start > 0) buffer = buffer.slice(start);
    }
  } catch (_) {
    if (ctrl.signal.aborted) return;
  }
  if (ctrl.signal.aborted || streamAbort !== ctrl || !tileEl) return;
  // The server closed the stream without an exit for every tab: network drop
  // or a core restart. Reattach; each session's replay refills its terminal.
  scheduleReconnect();
}

function scheduleReconnect(delay = RECONNECT_DELAY) {
  if (!tileEl) return;
  const tab = activeTab();
  if (tab) updateTabStatus(tab, t('plugin.terminal.status.reconnecting', null, 'reconnecting…'));
  window.clearTimeout(reconnectTimer);
  reconnectTimer = window.setTimeout(() => {
    reconnectTimer = null;
    // The reconnected stream replays every session from the start; each tab
    // resets itself in its `ready` handler, before the replay lands.
    for (const t of tabs) t.resyncing = true;
    connect();
  }, delay);
}

/** Dispatch one multiplexed frame to the tab that owns its session. */
function handleFrame(data) {
  let msg;
  try {
    msg = JSON.parse(data);
  } catch (_) {
    return;
  }
  const session = msg.session;
  if (!session) return;
  if (msg.type === 'gone') {
    // A session the window restored is gone server-side (server restart, the
    // shell was killed): a tab lives only as long as its shell, so close it.
    const tab = tabs.find((t) => t.sessionId === session);
    if (tab) closeTab(tab.id);
    return;
  }
  const tab = tabs.find((t) => t.sessionId === session);
  if (!tab) return; // a session this window no longer shows
  if (msg.type === 'ready') {
    tab.shell = msg.shell || '';
    tab.cwd = msg.cwd || '';
    if (tab.resyncing) {
      // Reconnect: the server is about to replay this session's scrollback
      // from the start, so drop what the terminal already shows first.
      tab.resyncing = false;
      try {
        tab.term?.reset();
      } catch (_) {
        /* reset is best-effort */
      }
    }
    updateTabStatus(tab, t('plugin.terminal.status.connected', null, 'connected'));
    renderTabs(); // the tooltip can name the shell now
    onHostResized(tab);
  } else if (msg.type === 'out') {
    queueOutput(tab, b64ToBytes(msg.data));
  } else if (msg.type === 'exit') {
    // The shell is gone: a tab lives only as long as its shell. Closing the
    // last tab opens a fresh one (see `closeTab`).
    closeTab(tab.id);
  }
}

/** Buffer a chunk of a tab's output and schedule the coalesced write. */
function queueOutput(tab, bytes) {
  tab.outQueue.push(bytes);
  tab.outBytes += bytes.length;
  if (tab.outTimer === null) tab.outTimer = window.setTimeout(() => flushOutput(tab), OUT_FLUSH_MS);
}

/** Write everything buffered so far to the tab's terminal as a single chunk. */
function flushOutput(tab) {
  if (tab.outTimer !== null) {
    window.clearTimeout(tab.outTimer);
    tab.outTimer = null;
  }
  if (!tab.outQueue.length) return;
  let merged;
  if (tab.outQueue.length === 1) {
    merged = tab.outQueue[0];
  } else {
    merged = new Uint8Array(tab.outBytes);
    let at = 0;
    for (const chunk of tab.outQueue) {
      merged.set(chunk, at);
      at += chunk.length;
    }
  }
  tab.outQueue = [];
  tab.outBytes = 0;
  tab.term?.write(merged);
}

/* ── window surface contract ──────────────────────────────────── */

export function mountTerminalTile() {
  if (tileEl) return tileEl;
  buildTile();
  void boot();
  return tileEl;
}

/** Load xterm (once) and restore the tab list. */
async function boot() {
  try {
    await ensureAssets();
  } catch (err) {
    setStatus(t('plugin.terminal.status.xtermFailed', null, 'could not load xterm.js'));
    toast(t('plugin.terminal.toast.xtermFailed', null, 'Terminal: xterm.js failed to load'), { type: 'error' });
    return;
  }
  if (!tileEl) return; // unmounted while loading
  await restoreTabs();
  if (tileEl) connect();
}

export function unmountTerminalTile() {
  stopStream();
  for (const tab of tabs) destroyTab(tab);
  tabs = [];
  activeTabId = null;
  tileEl = null;
  tabstripEl = null;
  statusEl = null;
}

export function getTerminalTileElement() {
  return tileEl;
}

export function wireTerminalEvents() {
  /* Re-read the themed palette when the user changes appearance/theme. */
  window.addEventListener('appearance:change', () => {
    for (const tab of tabs) {
      if (!tab.term) continue;
      tab.term.options.theme = { ...tab.term.options.theme, ...terminalTheme() };
    }
  });
  /* A pick in the top-bar Clipboard menu with this window focused: paste it
   * straight into the active shell (the menu already put it on the OS clipboard). */
  window.addEventListener('clipboard:paste', (event) => {
    const { text, focused } = event.detail || {};
    if (focused !== TERMINAL_PLUGIN || !text) return;
    activeTab()?.term?.paste(text);
  });
}

export function terminalContextMenu() {
  const tab = activeTab();
  return [
    {
      type: 'item',
      label: t('plugin.terminal.ctx.copy', null, 'Copy'),
      icon: 'ui/copy',
      disabled: !tab?.term?.getSelection?.(),
      onClick: () => copySelection(tab),
    },
    { type: 'item', label: t('plugin.terminal.ctx.paste', null, 'Paste'), icon: 'ui/clipboard', disabled: !tab, onClick: () => void pasteFromClipboard(tab) },
    { type: 'separator' },
    { type: 'item', label: t('plugin.terminal.tab.new', null, 'New tab'), icon: 'ui/plus', onClick: () => void createTab({}) },
    { type: 'item', label: t('plugin.terminal.ctx.clearScrollback', null, 'Clear scrollback'), icon: 'ui/trash', disabled: !tab, onClick: () => tab?.term?.clear() },
    { type: 'separator' },
    { type: 'item', label: t('plugin.terminal.ctx.closeTab', null, 'Close tab'), icon: 'ui/close', danger: true, disabled: !tab, onClick: () => { if (tab) closeTab(tab.id); } },
  ];
}

export default {
  name: TERMINAL_PLUGIN,
  icon: 'ui/monitor',
  mount: mountTerminalTile,
  unmount: unmountTerminalTile,
  getElement: getTerminalTileElement,
  wireEvents: wireTerminalEvents,
  contextMenu: terminalContextMenu,
};
