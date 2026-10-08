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
 *   render → WebGL where the platform allows it, else canvas, else the built-in
 *            DOM renderer (see `selectRenderer`) — an agent TUI redrawing in
 *            here is what makes the renderer choice matter
 *
 * Closing the window does NOT kill the shell: the session id lives in
 * `sessionStorage`, so reopening (or reloading the page) reattaches and
 * replays recent output. "Kill session" is the explicit way out.
 */

import { button, toast } from '../../ui/index.js';
import { apiFetch, getToken } from '../../js/api.js';
import { copyText, readClipboardText } from '../../js/clipboard.js';

export const TERMINAL_PLUGIN = 'terminal';

const VENDOR = `/plugins/${TERMINAL_PLUGIN}/vendor`;
const STORAGE_KEY = 'terminal.sessionId';

let tileEl = null;
let hostEl = null;
let titleEl = null;
let statusEl = null;

let term = null;
let fitAddon = null;
let canvasAddon = null;
let assetsPromise = null;

let sessionId = null;
let streamAbort = null;
let reconnectTimer = null;
let observer = null;
let dead = false;

let inputQueue = [];
let flushTimer = null;
let resizeTimer = null;

// Output is coalesced before it reaches xterm. A TUI redraws continuously
// (the agent UI writes ~60 times a second), and xterm repaints the grid for
// every write it processes, so the repaint count — not the byte count — is
// what costs a core on a large terminal. Merging the frames that arrive
// inside one tick keeps the final state and roughly halves the repaints.
const OUT_FLUSH_MS = 33;
let outQueue = [];
let outBytes = 0;
let outTimer = null;

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

/** Show the running shell in the bar. The window title bar already names the
 *  plugin, so this no longer repeats "Terminal". */
function updateTitle() {
  if (!titleEl) return;
  titleEl.textContent = shell.shell ? shell.shell.replace('/bin/', '') : '';
}

/* ── tile DOM ─────────────────────────────────────────────────── */

function buildTile() {
  tileEl = document.createElement('section');
  tileEl.className = 'tile terminal-tile';
  tileEl.dataset.plugin = TERMINAL_PLUGIN;

  const bar = document.createElement('div');
  bar.className = 'terminal-bar';
  bar.dataset.windowBar = '';

  titleEl = document.createElement('span');
  titleEl.className = 'terminal-title';
  titleEl.textContent = '';

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

  // Single top bar (Word/Calc convention): shell name + every action button.
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
  selectRenderer();

  term.onData((data) => queueInput(data));
  // Ctrl/Cmd+C and Ctrl/Cmd+V have to be decided by the terminal, not the
  // browser: without this, Ctrl+C is always a SIGINT (xterm sends \x03) and a
  // selection cannot be copied — xterm's WebGL selection is not a DOM
  // selection, so the browser's own copy would find nothing.
  term.attachCustomKeyEventHandler(onTerminalKey);

  observer = new ResizeObserver(() => onHostResized());
  observer.observe(hostEl);

  fit();
  await attachSession();
}

/**
 * Load the canvas renderer as the fallback below WebGL. Idempotent, and a
 * no-op when the addon is missing or refuses to load — whatever renderer is
 * already active (the DOM one) stays in place.
 */
function useCanvasRenderer() {
  if (canvasAddon || !term || !window.CanvasAddon) return;
  try {
    canvasAddon = new window.CanvasAddon.CanvasAddon();
    term.loadAddon(canvasAddon);
  } catch (_) {
    canvasAddon = null; // keep the DOM renderer
  }
}

/**
 * Pick the fastest renderer the platform can give us: WebGL, then canvas,
 * then the built-in DOM renderer (see `ensureAssets`). A lost WebGL context
 * demotes to canvas rather than leaving a blank terminal.
 */
function selectRenderer() {
  if (!term) return;
  if (window.WebglAddon) {
    try {
      const webgl = new window.WebglAddon.WebglAddon();
      webgl.onContextLoss(() => {
        try {
          webgl.dispose();
        } catch (_) {
          /* already disposed */
        }
        useCanvasRenderer();
        console.info('terminal: WebGL context lost — renderer =',
          canvasAddon ? 'canvas' : 'dom');
      });
      term.loadAddon(webgl);
      console.info('terminal: renderer = webgl');
      return;
    } catch (err) {
      console.info('terminal: WebGL unavailable, trying canvas:', err?.message ?? err);
    }
  }
  useCanvasRenderer();
  console.info('terminal: renderer =', canvasAddon ? 'canvas' : 'dom');
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

/* ── clipboard ────────────────────────────────────────────────── */

/**
 * Ctrl/Cmd+C copies the selection when there is one and stays a SIGINT
 * otherwise — the standard terminal trade-off. Ctrl/Cmd+V pastes. Shift is
 * not excluded, so the classic Ctrl+Shift+C / Ctrl+Shift+V pair works too.
 *
 * Returning false tells xterm the key was handled: it must not reach the PTY,
 * which is what keeps a copy from interrupting the foreground process.
 */
function onTerminalKey(event) {
  if (event.type !== 'keydown') return true; // keypress repeats pass through
  if (!(event.ctrlKey || event.metaKey) || event.altKey) return true;
  const key = (event.key || '').toLowerCase();
  if (key === 'c') return copySelection() ? false : true;
  if (key === 'v') {
    void pasteFromClipboard();
    return false;
  }
  return true;
}

/** Copy the current xterm selection through the central clipboard service. */
function copySelection() {
  const selection = term?.getSelection?.() || '';
  if (!selection) return false;
  void copyText(selection, { source: TERMINAL_PLUGIN });
  return true;
}

/**
 * Paste clipboard text into the shell. `term.paste` honours bracketed-paste
 * mode (vim/tmux see a paste instead of a burst of typed keys) and flows
 * through `onData` like any other input.
 */
async function pasteFromClipboard() {
  const text = await readClipboardText();
  if (!text) {
    toast('Clipboard unavailable', { type: 'error' });
    return;
  }
  term?.paste(text);
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
        if (dead) {
          stopStream();
          return;
        }
      }
      if (start > 0) buffer = buffer.slice(start);
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
    queueOutput(b64ToBytes(msg.data));
  } else if (msg.type === 'exit') {
    dead = true;
    flushOutput();
    term?.write('\r\n\x1b[2m[session ended — “New shell” starts another]\x1b[0m\r\n');
    setStatus('session ended');
  }
}

/** Buffer a chunk of shell output and schedule the coalesced write. */
function queueOutput(bytes) {
  outQueue.push(bytes);
  outBytes += bytes.length;
  if (outTimer === null) outTimer = window.setTimeout(flushOutput, OUT_FLUSH_MS);
}

/** Write everything buffered so far to xterm as a single chunk. */
function flushOutput() {
  if (outTimer !== null) {
    window.clearTimeout(outTimer);
    outTimer = null;
  }
  if (!outQueue.length) return;
  let merged;
  if (outQueue.length === 1) {
    merged = outQueue[0];
  } else {
    merged = new Uint8Array(outBytes);
    let at = 0;
    for (const chunk of outQueue) {
      merged.set(chunk, at);
      at += chunk.length;
    }
  }
  outQueue = [];
  outBytes = 0;
  term?.write(merged);
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
  if (outTimer !== null) clearTimeout(outTimer);
  resizeTimer = null;
  flushTimer = null;
  outTimer = null;
  inputQueue = [];
  outQueue = [];
  outBytes = 0;
  try {
    term?.dispose();
  } catch (_) {
    /* already gone */
  }
  term = null;
  fitAddon = null;
  canvasAddon = null;
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
  /* A pick in the top-bar Clipboard menu with this window focused: paste it
   * straight into the shell (the menu already put it on the OS clipboard). */
  window.addEventListener('clipboard:paste', (event) => {
    const { text, focused } = event.detail || {};
    if (focused === TERMINAL_PLUGIN && text) term?.paste(text);
  });
}

export function terminalContextMenu() {
  return [
    {
      type: 'item',
      label: 'Copy',
      icon: 'ui/copy',
      disabled: !term?.getSelection?.(),
      onClick: () => copySelection(),
    },
    { type: 'item', label: 'Paste', icon: 'ui/clipboard', onClick: () => void pasteFromClipboard() },
    { type: 'separator' },
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
