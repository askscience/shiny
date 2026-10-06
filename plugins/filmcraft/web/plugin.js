/**
 * FilmCraft window — the editor, mounted in a Shiny tile.
 *
 * FilmCraft's web build is a WebAssembly bundle of the desktop app: the same
 * engine, the same egui UI, started by eframe's web runner on a canvas. This
 * module boots that bundle on a canvas inside the tile, so the editor is a
 * normal plugin window rather than an iframe (the core sends
 * `X-Frame-Options: DENY` on every response, so an iframe would be refused).
 *
 * Three details the standalone build gets for free and we have to arrange:
 *
 *  • The glue is imported with a **relative** specifier. A module specifier
 *    resolves against the module's own URL, and the core serves this plugin's
 *    `web/` at `/plugins/filmcraft/`, so `./filmcraft_web.js` finds it — and
 *    still works behind the kiosk filtering proxy, where a root-absolute path
 *    would resolve to the proxy origin.
 *  • The audio worklet is loaded by the app from the **document root**
 *    (`audioWorklet.addModule("audio-worklet.js")`). The plugin claims that
 *    path with a public route so audio works in the shell too.
 *  • A Rust panic inside a frame leaves a dead canvas, so the app's own fatal
 *    overlay is reproduced here rather than lost with the standalone index.html.
 *
 * The editor is a single instance per page: the wasm module can be started once
 * and its state (project, OPFS recovery, the audio context) is global. Closing
 * the tile therefore detaches the canvas instead of destroying it, and
 * re-opening re-attaches the same editor with the project intact.
 */

import { button, iconButton, spinner, toast } from '../../ui/index.js';
import { apiFetch } from '../../js/api.js';

export const FILMCRAFT_PLUGIN = 'filmcraft';

/** How often the window asks the relay for work. */
const POLL_MS = 400;
/** Command ids whose result is a large blob we only summarize. */
const BIG_RESULT_BYTES = 4096;

let tileEl = null;
let stageEl = null;      // the tile's content area
let statusEl = null;     // status line in the header
let bootEl = null;       // boot overlay (progress / errors)

/** The live editor, kept across mounts: { holder, canvas, ready }. */
let app = null;
let booting = null;      // in-flight boot promise, so a remount waits instead of racing
let pollTimer = null;
let lastPanic = null;

/* ── Fatal-error capture ─────────────────────────────────────────────── */

const consoleError = console.error.bind(console);
console.error = (...args) => {
  const text = args.map(String).join(' ');
  if (/panicked at|memory allocation of/.test(text)) lastPanic = text;
  consoleError(...args);
};

/**
 * Show a hard failure in the tile: the app stops answering after a Rust panic
 * or an out-of-memory abort, and a frozen canvas tells the user nothing.
 */
function showFatal(what) {
  const message = lastPanic || String(what?.stack || what?.message || what || 'unknown error');
  if (!bootEl) return;
  bootEl.innerHTML = '';
  const box = text('div', 'filmcraft-fatal', '');
  box.append(text('strong', '', 'FilmCraft stopped working'));
  box.append(text('pre', '', message.split('\n\nStack:')[0]));
  box.append(text('p', '', 'Unsaved changes are kept in the browser and come back when you reload.'));
  box.append(button({
    label: 'Reload the desktop',
    onClick: () => location.reload(),
  }));
  bootEl.appendChild(box);
  bootEl.classList.remove('filmcraft-boot--loading');
  bootEl.classList.add('filmcraft-boot--error');
}

function onRuntimeError(event) {
  const err = event.error || event.message;
  const fatal = err instanceof WebAssembly.RuntimeError
    || /panicked at|unreachable|memory access out of bounds/.test(String(err?.message || err));
  if (fatal && app?.ready) showFatal(err);
}
addEventListener('error', onRuntimeError);
addEventListener('unhandledrejection', onRuntimeError);

/* ── Boot ────────────────────────────────────────────────────────────── */

/**
 * Fetch the wasm bundle with progress, so a 27 MB download over a remote
 * connection does not look like a hung window.
 */
async function fetchWithProgress(url, onProgress) {
  const response = await fetch(url);
  if (!response.ok) throw new Error(`${url}: HTTP ${response.status}`);
  const total = Number(response.headers.get('content-length')) || 0;
  if (!response.body || !total) {
    onProgress(0, 0);
    return new Uint8Array(await response.arrayBuffer());
  }
  const reader = response.body.getReader();
  const chunks = [];
  let got = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    chunks.push(value);
    got += value.length;
    onProgress(got, total);
  }
  const out = new Uint8Array(got);
  let at = 0;
  for (const chunk of chunks) {
    out.set(chunk, at);
    at += chunk.length;
  }
  return out;
}

/**
 * Start the editor. Resolves once `window.filmcraft` answers, i.e. when the
 * engine is up and the UI can take commands.
 */
async function boot() {
  if (app?.ready) return app;
  if (booting) return booting;

  booting = (async () => {
    const holder = document.createElement('div');
    holder.className = 'filmcraft-holder';
    const canvas = document.createElement('canvas');
    canvas.id = 'filmcraft_canvas';
    canvas.className = 'filmcraft-canvas';
    holder.appendChild(canvas);
    // Behind the boot overlay: it is removed when the engine answers.
    stageEl.appendChild(holder);
    const instance = { holder, canvas, ready: false, commands: null };
    app = instance;

    try {
      bootEl.classList.remove('filmcraft-boot--error');
      bootEl.classList.add('filmcraft-boot--loading');

      // The glue is an ES module served next to this file. Relative on purpose
      // (see the header): the kiosk proxy serves the app under /p/….
      const glue = await import('./filmcraft_web.js');
      const wasmUrl = new URL('./filmcraft_web_bg.wasm', import.meta.url);
      const bytes = await fetchWithProgress(wasmUrl, (got, total) => {
        setStatus(total
          ? `Loading editor… ${(got / 1048576).toFixed(1)} / ${(total / 1048576).toFixed(1)} MB`
          : `Loading editor… ${(got / 1048576).toFixed(1)} MB`);
      });

      // A WebGPU browser gets the GPU compositor; otherwise FilmCraft falls
      // back to WebGL2 on its own, and `?cpu`/`?webgl` are honoured by the
      // standalone build through the document URL.
      await glue.default({ module_or_path: bytes });
      await glue.start('filmcraft_canvas');

      // The control API is installed by `start`; wait for the engine to be
      // reachable so a mount never reports ready too early.
      const deadline = Date.now() + 30000;
      while (!(window.filmcraft && window.filmcraft.info) && Date.now() < deadline) {
        await new Promise((r) => setTimeout(r, 50));
      }
      if (!window.filmcraft?.execute) throw new Error('the editor did not come up');

      instance.ready = true;
      bootEl.classList.add('filmcraft-boot--done');
      bootEl.remove();
      bootEl = null;
      setStatus('Editor ready');
      // Drain anything queued while the window was closed.
      void pollOnce();
      return instance;
    } catch (err) {
      showFatal(err);
      setStatus('Failed to start');
      app = null;
      throw err;
    } finally {
      booting = null;
    }
  })();

  return booting;
}

/* ── Relay: the assistant talks to this window ──────────────────────── */

function setStatus(message) {
  if (statusEl) statusEl.textContent = message;
}

/** Small DOM helper: an element with a class and text. */
function text(tag, className, content) {
  const el = document.createElement(tag);
  el.className = className;
  el.textContent = content;
  return el;
}

/** Ask the relay for one request and run it in the editor. */
async function pollOnce() {
  let reply;
  try {
    reply = await apiFetch('/api/filmcraft/next');
  } catch (err) {
    // A transient network error must not kill the loop; try again next tick.
    setStatus(`Relay offline: ${err.message}`);
    return;
  }
  // Core wraps plugin responses in `{success, data}`; 204 leaves it empty.
  const next = reply?.data;
  if (!next || !next.id) return;

  const { id, method, params } = next;
  let body;
  try {
    const value = method === 'engine.execute'
      ? await window.filmcraft.execute(params.command, params.params || {})
      : await window.filmcraft.request(method, params || {});
    // A screenshot or a whole frame comes back as base64; do not echo it into
    // the chat transcript.
    const text = JSON.stringify(value ?? null);
    if (text && text.length > BIG_RESULT_BYTES) {
      body = { id, ok: true, result: { note: 'result omitted (too large)', bytes: text.length } };
    } else {
      body = { id, ok: true, result: value ?? null };
    }
    setStatus(`Ran ${params?.command || method}`);
  } catch (err) {
    body = { id, ok: false, error: String(err?.message || err) };
    setStatus(`Failed: ${params?.command || method}`);
  }
  try {
    await apiFetch('/api/filmcraft/result', { method: 'POST', body: JSON.stringify(body) });
  } catch (_) {
    /* the caller may have timed out already */
  }
}

function startPolling() {
  if (pollTimer) return;
  const tick = async () => {
    await pollOnce();
    pollTimer = setTimeout(tick, POLL_MS);
  };
  pollTimer = setTimeout(tick, POLL_MS);
}

function stopPolling() {
  if (pollTimer) clearTimeout(pollTimer);
  pollTimer = null;
}

/* ── Tile ────────────────────────────────────────────────────────────── */

/**
 * Load the window's own stylesheet.
 *
 * The one deliberate exception to "plugins ship no CSS", and the same one the
 * Browser window makes: a canvas filling a window under an overlay is a layout,
 * not a component. The controls are still core components, so their metrics and
 * accent come from `web/ui/ui.css`; `plugin.css` only arranges the area around
 * them, scoped under `.filmcraft-tile`.
 *
 * Relative on purpose, like the glue import: the kiosk filtering proxy serves
 * the app under `/p/…`, where a root-absolute path would leave the origin.
 */
function ensureStylesheet() {
  if (document.getElementById('filmcraft-plugin-css')) return;
  const link = document.createElement('link');
  link.id = 'filmcraft-plugin-css';
  link.rel = 'stylesheet';
  link.href = new URL('./plugin.css', import.meta.url).href;
  document.head.appendChild(link);
}

export function mountTile() {
  ensureStylesheet();
  if (tileEl) {
    // Re-opening a tile we already built: put the editor back on screen.
    if (app?.holder && app.holder.parentElement !== stageEl) stageEl.appendChild(app.holder);
    if (app?.ready) void boot();
    startPolling();
    return tileEl;
  }

  tileEl = document.createElement('section');
  tileEl.className = 'tile filmcraft-tile';
  tileEl.dataset.plugin = FILMCRAFT_PLUGIN;

  const bar = document.createElement('div');
  bar.className = 'tile-header';
  bar.dataset.windowBar = '';

  const title = document.createElement('div');
  title.className = 'tile-header-title';
  title.textContent = 'FilmCraft';

  statusEl = document.createElement('div');
  statusEl.className = 'filmcraft-status';

  const spacer = document.createElement('div');
  spacer.className = 'tile-header-spacer';

  const reloadBtn = iconButton({
    icon: 'ui/refresh',
    label: 'Reload the editor (reloads this page)',
    onClick: () => location.reload(),
  });

  bar.append(title, statusEl, spacer, reloadBtn);

  stageEl = document.createElement('div');
  stageEl.className = 'filmcraft-stage';

  bootEl = document.createElement('div');
  bootEl.className = 'filmcraft-boot filmcraft-boot--loading';
  bootEl.append(spinner(), text('div', 'filmcraft-boot-text', 'Loading the editor…'));

  stageEl.appendChild(bootEl);
  tileEl.append(bar, stageEl);
  document.body.appendChild(tileEl);   // hidden until mounted into a slot

  // A previous session may have left a live editor behind (the tile was closed,
  // the plugin deactivated and activated again). Adopt it instead of booting a
  // second one: the wasm module can only be started once per page.
  if (app?.holder) {
    stageEl.appendChild(app.holder);
    if (app.ready) {
      bootEl.remove();
      bootEl = null;
      setStatus('Editor ready');
    } else {
      void boot().catch(() => { /* shown in the tile by showFatal */ });
    }
  } else {
    void boot().catch(() => { /* shown in the tile by showFatal */ });
  }

  startPolling();
  return tileEl;
}

export function unmountTile() {
  stopPolling();
  // The editor keeps running: it holds the project and the audio context, and
  // the wasm module can only be started once per page. Detach it; the next
  // mount re-attaches the same instance.
  if (app?.holder && app.holder.parentElement === stageEl) {
    app.holder.remove();
  }
  tileEl?.remove();
  tileEl = null;
  stageEl = null;
  statusEl = null;
  bootEl = null;
  return null;
}

export function getTileElement() {
  return tileEl;
}

export default {
  name: FILMCRAFT_PLUGIN,
  icon: 'ui/video',
  mount: mountTile,
  unmount: unmountTile,
  getElement: getTileElement,
};