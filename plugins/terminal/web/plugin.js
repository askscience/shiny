/**
 * terminal.js — the Terminal plugin's window surface.
 *
 * A real login shell on the host, rendered with xterm.js. The server owns the
 * PTY (see plugins/terminal/src/pty.rs); this module is only the glass:
 *
 *   mount  → build the tile, load xterm from `vendor/`, create/reuse a session
 *   stream → `/api/terminal/stream` (SSE frames) carries output, read with a
 *            streaming fetch so the Bearer token goes in the header —
 *            `EventSource` cannot send headers, and its cookie-only auth
 *            breaks wherever the session cookie is missing (e.g. a kiosk
 *            webview that restores localStorage but not its cookie jar)
 *   input  → keystrokes are POSTed to `/api/terminal/input`
 *   resize → the fit addon measures the window, `/api/terminal/resize` tells
 *            the PTY, so `top`, `vim` and friends get the right width
 *
 * Closing the window does NOT kill the shell: the session id lives in
 * `sessionStorage`, so reopening (or reloading the page) reattaches and
 * replays recent output. "Kill session" is the explicit way out.
 */

import { button, toast } from '../../ui/index.js';
import { apiFetch, getToken } from '../../js/api.js';

export const TERMINAL_PLUGIN = 'terminal';

const VENDOR = `/plugins/${TERMINAL_PLUGIN}/vendor`;
const STORAGE_KEY = 'terminal.sessionId';

let tileEl = null;
let hostEl = null;
let titleEl = null;
let statusEl = null;

let term = null;
let fitAddon = null;
let assetsPromise = null;

let sessionId = null;
let streamAbort = null;
let reconnectTimer = null;
let observer = null;
let dead = false;

let inputQueue = [];
let flushTimer = null;
let resizeTimer = null;

let shell = { shell: '', cwd: '' };

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
      // Optional: the canvas renderer. Worth it because the DOM renderer
      // rebuilds text for every changed cell, which pegs a core on a wide
      // terminal showing a redrawing TUI. A load failure is not fatal — the
      // immediately-following code keeps the DOM renderer.
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

function updateTitle() {
  if (!titleEl) return;
  titleEl.textContent = shell.shell ? `Terminal — ${shell.shell.replace('/bin/', '')}` : 'Terminal';
}

/* ── tile DOM ─────────────────────────────────────────────────── */

function buildTile() {
  tileEl = document.createElement('section');
  tileEl.className = 'tile terminal-tile';
  tileEl.dataset.plugin = TERMINAL_PLUGIN;

  const bar = document.createElement('div');
  bar.className = 'terminal-bar';

  titleEl = document.createElement('span');
  titleEl.className = 'terminal-title';
  titleEl.textContent = 'Terminal';

  const mkBtn = (iconName, title, onClick, danger = false) => {
    const el = button({ icon: iconName, variant: 'ghost', onClick });
    el.classList.add('ui-btn--icon', 'terminal-tool');
    if (danger) el.classList.add('terminal-tool--danger');
    el.title = title;
    el.setAttribute('aria-label', title);
    return el;
  };
  const newBtn = mkBtn('ui/plus', 'New shell', () => void restart());
  const clearBtn = mkBtn('ui/trash', 'Clear scrollback', () => term?.clear());
  const killBtn = mkBtn('ui/close', 'Kill session', () => void killSession(), true);

  // Single top bar (Word/Calc convention): title + every action button.
  bar.append(titleEl, newBtn, clearBtn, killBtn);

  hostEl = document.createElement('div');
  hostEl.className = 'terminal-host';

  statusEl = document.createElement('div');
  statusEl.className = 'terminal-status';
  statusEl.textContent = 'starting terminal…';

  tileEl.append(bar, hostEl, statusEl);
  return tileEl;
}

/* ── terminal ─────────────────────────────────────────────────── */

async function initTerminal() {
  if (term || !hostEl) return;
  try {
    await ensureAssets();
  } catch (err) {
    setStatus('could not load xterm.js');
    toast('Terminal: xterm.js failed to load', { type: 'error' });
    return;
  }
  if (!hostEl || term) return;

  term = new window.Terminal({
    cursorBlink: true,
    fontFamily: 'ui-monospace, SFMono-Regular, Menlo, Consolas, monospace',
    fontSize: 13,
    scrollback: 5000,
    theme: terminalTheme(),
  });
  fitAddon = new window.FitAddon.FitAddon();
  term.loadAddon(fitAddon);
  term.open(hostEl);

  // Canvas renderer where available (see ensureAssets): the DOM renderer
  // rebuilds text spans for every changed row, which is what made a large
  // terminal showing a redrawing TUI cost a whole core.
  if (window.CanvasAddon) {
    try {
      term.loadAddon(new window.CanvasAddon.CanvasAddon());
    } catch (_) {
      /* keep the DOM renderer */
    }
  }

  term.onData((data) => queueInput(data));

  observer = new ResizeObserver(() => onHostResized());
  observer.observe(hostEl);

  fit();
  await attachSession();
}

function fit() {
  try {
    fitAddon?.fit();
  } catch (_) {
    /* hidden or zero-sized window — harmless */
  }
}

function onHostResized() {
  fit();
  if (!term || !sessionId) return;
  clearTimeout(resizeTimer);
  resizeTimer = setTimeout(() => {
    if (!sessionId || dead) return;
    void post('/api/terminal/resize', { session: sessionId, cols: term.cols, rows: term.rows }).catch(() => {});
  }, 150);
}

function queueInput(data) {
  inputQueue.push(data);
  if (flushTimer) return;
  flushTimer = setTimeout(() => void flushInput(), 8);
}

async function flushInput() {
  flushTimer = null;
  const data = inputQueue.join('');
  inputQueue = [];
  if (!data || !sessionId) return;
  try {
    await post('/api/terminal/input', { session: sessionId, data });
  } catch (_) {
    /* the stream will report the session state */
  }
}

/* ── session lifecycle ────────────────────────────────────────── */

async function attachSession() {
  sessionId = sessionStorage.getItem(STORAGE_KEY) || null;

  if (!sessionId) {
    const dims = fitAddon?.proposeDimensions?.() || null;
    try {
      const res = await post('/api/terminal/sessions', {
        cols: dims?.cols || term?.cols || 80,
        rows: dims?.rows || term?.rows || 24,
      });
      sessionId = res?.data?.id || null;
      if (sessionId) sessionStorage.setItem(STORAGE_KEY, sessionId);
    } catch (err) {
      setStatus('could not start a shell');
      toast(err?.message || 'Terminal: could not start a shell', { type: 'error' });
      return;
    }
  }
  if (!sessionId) {
    setStatus('could not start a shell');
    return;
  }
  connect();
}

/* ── output stream ────────────────────────────────────────────── */

/** Reconnect delay after a dropped stream (matches EventSource's retry feel). */
const RECONNECT_DELAY = 1500;

/** Close the current stream reader and cancel a pending reconnect. */
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

function connect() {
  stopStream();
  dead = false;
  setStatus('connecting…');
  const session = sessionId;
  const ctrl = new AbortController();
  streamAbort = ctrl;
  void runStream(session, ctrl);
}

/**
 * Read the SSE frames off `/api/terminal/stream` with a streaming fetch.
 * `EventSource` would be the obvious tool, but it cannot set an
 * `Authorization` header — and the session cookie it falls back on is not
 * always there (the kiosk webview keeps localStorage but not its cookies, so
 * the app is signed in while `EventSource` gets a 401 and dies with
 * `readyState === CLOSED`). A fetch body reader authenticates exactly like
 * every other call and reports real HTTP statuses.
 */
async function runStream(session, ctrl) {
  const token = getToken();
  let res;
  try {
    res = await fetch(`/api/terminal/stream?session=${encodeURIComponent(session)}`, {
      headers: token ? { Authorization: `Bearer ${token}` } : {},
      cache: 'no-store',
      signal: ctrl.signal,
    });
  } catch (_) {
    if (ctrl.signal.aborted) return;
    scheduleReconnect(session);
    return;
  }
  if (ctrl.signal.aborted || streamAbort !== ctrl) return;

  if (!res.ok || !res.body) {
    if (ctrl.signal.aborted) return;
    if (res.status === 404) {
      // The shell is gone server-side (server restarted, session closed):
      // forget it so the next attempt starts a fresh one.
      sessionId = null;
      sessionStorage.removeItem(STORAGE_KEY);
      dead = true;
      setStatus('session unavailable — “New shell” to start one');
      return;
    }
    if (res.status === 401) {
      setStatus('session expired — sign in again');
      scheduleReconnect(session, 5000);
      return;
    }
    scheduleReconnect(session);
    return;
  }

  setStatus('connected');
  const reader = res.body.getReader();
  const decoder = new TextDecoder();
  let buffer = '';
  try {
    for (;;) {
      const { value, done } = await reader.read();
      if (done) break;
      buffer += decoder.decode(value, { stream: true });
      let end;
      while ((end = buffer.indexOf('\n\n')) !== -1) {
        const frame = buffer.slice(0, end);
        buffer = buffer.slice(end + 2);
        const line = frame.split('\n').find((l) => l.startsWith('data:'));
        if (line) handleFrame(line.slice(5).trim());
        if (dead) {
          stopStream();
          return;
        }
      }
    }
  } catch (_) {
    if (ctrl.signal.aborted) return;
  }
  if (ctrl.signal.aborted || streamAbort !== ctrl || dead) return;
  // The server closed the stream without an `exit` frame: network drop or a
  // core restart. Reattach; the scrollback replay covers what was missed.
  scheduleReconnect(session);
}

function scheduleReconnect(session, delay = RECONNECT_DELAY) {
  if (dead || !sessionId || sessionId !== session) return;
  setStatus('reconnecting…');
  window.clearTimeout(reconnectTimer);
  reconnectTimer = window.setTimeout(() => {
    reconnectTimer = null;
    if (!dead && sessionId === session) connect();
  }, delay);
}

function handleFrame(data) {
  let msg;
  try {
    msg = JSON.parse(data);
  } catch (_) {
    return;
  }
  if (msg.type === 'ready') {
    shell = { shell: msg.shell || '', cwd: msg.cwd || '' };
    updateTitle();
    setStatus(`${msg.shell || 'shell'} — ${msg.cwd || ''}`);
    onHostResized();
  } else if (msg.type === 'out') {
    term?.write(b64ToBytes(msg.data));
  } else if (msg.type === 'exit') {
    dead = true;
    term?.write('\r\n\x1b[2m[session ended — “New shell” starts another]\x1b[0m\r\n');
    setStatus('session ended');
  }
}

async function restart() {
  await killSession({ quiet: true });
  fit();
  await attachSession();
}

async function killSession({ quiet = false } = {}) {
  stopStream();
  const id = sessionId;
  sessionId = null;
  dead = false;
  sessionStorage.removeItem(STORAGE_KEY);
  if (id) {
    try {
      await post('/api/terminal/close', { session: id });
    } catch (err) {
      if (!quiet) toast(err?.message || 'Terminal: could not close the session', { type: 'error' });
    }
  }
  if (!quiet) {
    term?.reset();
    setStatus('session closed — “New shell” to start one');
  }
}

/* ── window surface contract ──────────────────────────────────── */

export function mountTerminalTile() {
  if (tileEl) return tileEl;
  buildTile();
  void initTerminal();
  return tileEl;
}

export function unmountTerminalTile() {
  stopStream();
  if (observer) {
    observer.disconnect();
    observer = null;
  }
  clearTimeout(resizeTimer);
  clearTimeout(flushTimer);
  resizeTimer = null;
  flushTimer = null;
  inputQueue = [];
  try {
    term?.dispose();
  } catch (_) {
    /* already gone */
  }
  term = null;
  fitAddon = null;
  tileEl = null;
  hostEl = null;
  titleEl = null;
  statusEl = null;
  shell = { shell: '', cwd: '' };
}

export function getTerminalTileElement() {
  return tileEl;
}

export function wireTerminalEvents() {
  /* Re-read the themed palette when the user changes appearance/theme. */
  window.addEventListener('appearance:change', () => {
    if (!term) return;
    term.options.theme = { ...term.options.theme, ...terminalTheme() };
  });
}

export function terminalContextMenu() {
  return [
    { type: 'item', label: 'New shell', icon: 'ui/plus', onClick: () => void restart() },
    { type: 'item', label: 'Clear scrollback', icon: 'ui/trash', onClick: () => term?.clear() },
    { type: 'separator' },
    { type: 'item', label: 'Kill session', icon: 'ui/close', danger: true, onClick: () => void killSession() },
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
