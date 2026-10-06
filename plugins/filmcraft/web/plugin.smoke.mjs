/**
 * plugin.smoke.mjs — mounts the FilmCraft window against a DOM shim.
 *
 * The editor itself is 27.6 MB of WebAssembly and needs a real browser, so this
 * does not boot it. It checks the parts that are cheap to get wrong and that
 * would otherwise only fail in front of the user:
 *
 *   • the surface contract (default export: name, mount, unmount, getElement),
 *   • the tile markup core expects (`section.tile`, `data-plugin`, a
 *     `data-window-bar` header),
 *   • the relative `./filmcraft_web.js` import and `canvas#filmcraft_canvas`
 *     boot contract, which is what breaks behind the kiosk proxy,
 *   • the stylesheet is requested from the plugin's own path,
 *   • unmount detaches the canvas but keeps the instance for the next mount,
 *   • the relay polls `/api/filmcraft/next` and posts results back.
 *
 * Run:  node plugins/filmcraft/web/plugin.smoke.mjs
 */

import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import vm from 'node:vm';

const here = fileURLToPath(new URL('.', import.meta.url));
const source = await readFile(`${here}plugin.js`, 'utf8');

/* ── A DOM shim just large enough for the surface ──────────────────────── */

/** The subset of DOMTokenList the window uses. */
class TokenList {
  constructor(el) { this.el = el; this.tokens = new Set(); }
  get value() { return [...this.tokens].join(' '); }
  add(...names) { for (const n of names) this.tokens.add(n); }
  remove(...names) { for (const n of names) this.tokens.delete(n); }
  contains(name) { return this.tokens.has(name); }
}

class El {
  constructor(tag) {
    this.tagName = String(tag).toUpperCase();
    this.children = [];
    this.classList = new TokenList(this);
    this.dataset = {};
    this.style = {};
    this.listeners = new Map();
    this.parentElement = null;
    this.textContent = '';
    this.title = '';
    this.type = '';
    this.innerHTML = '';
  }
  get className() { return this.classList.value; }
  set className(v) { this.classList = new TokenList(this); for (const n of String(v).split(/\s+/).filter(Boolean)) this.classList.add(n); }
  append(...kids) { for (const k of kids) { k.parentElement = this; this.children.push(k); } }
  appendChild(k) { this.append(k); return k; }
  remove() {
    if (!this.parentElement) return;
    const i = this.parentElement.children.indexOf(this);
    if (i >= 0) this.parentElement.children.splice(i, 1);
    this.parentElement = null;
  }
  addEventListener(type, fn) {
    if (!this.listeners.has(type)) this.listeners.set(type, []);
    this.listeners.get(type).push(fn);
  }
  fire(type, ev = {}) { for (const fn of this.listeners.get(type) || []) fn({ preventDefault() {}, ...ev }); }
}

const byId = new Map();

const document = {
  body: new El('body'),
  head: new El('head'),
  createElement(tag) { return new El(tag); },
  getElementById(id) { return byId.get(id) || null; },
};

/** The core UI library and the api helper the window imports. */
const ui = {
  button: (o) => {
    const el = new El('button');
    el.type = 'button';
    el.textContent = o.label || '';
    el.addEventListener('click', o.onClick || (() => {}));
    return el;
  },
  iconButton: (o) => {
    const el = new El('button');
    el.type = 'button';
    el.title = o.label || '';
    el.dataset.icon = o.icon || '';
    el.addEventListener('click', o.onClick || (() => {}));
    return el;
  },
  spinner: () => {
    const el = new El('span');
    el.className = 'ui-spinner';
    return el;
  },
};

/** apiFetch, recording calls and answering from a script. */
const calls = [];
let responder = async () => null;
const apiFetch = async (path, options = {}) => {
  calls.push({ path, options });
  if (path.includes('/next')) return responder(path, options);
  return { success: true, data: {} };
};

// `window.filmcraft` is what the real editor installs; the relay drives it.
const filmcraft = {
  execute: async (command) => ({ ok: true, from: 'engine', command }),
  request: async (method) => ({ ok: true, from: 'control', method }),
  info: () => ({ backend: 'WebGPU' }),
  commands: () => [{ id: 'project.inspect' }],
};

const sandbox = {
  document,
  console,
  // Where the real loader serves this module from: the core exposes a plugin's
  // web/ directory at /plugins/<name>/.
  __moduleUrl: 'http://localhost:8080/plugins/filmcraft/plugin.js',
  location: { href: 'http://localhost:8080/', reload() {} },
  localStorage: { getItem: () => null },
  addEventListener() {},
  removeEventListener() {},
  setTimeout,
  clearTimeout,
  fetch: async () => ({ ok: true, headers: { get: () => null }, body: null, arrayBuffer: async () => new ArrayBuffer(8) }),
  window: {},
  URL,
  WebAssembly: { RuntimeError: class extends Error {} },
  Date,
  JSON,
  Promise,
  Number,
  import: undefined,           // replaced per test
};
sandbox.window = sandbox;
sandbox.globalThis = sandbox;
// The editor installs this on start; present from the beginning so the boot
// wait loop sees it immediately.
sandbox.filmcraft = filmcraft;

// Bindings the module would have imported.
for (const [name, value] of Object.entries({ ...ui, apiFetch })) {
  sandbox[name] = value;
}

const context = vm.createContext(sandbox);
// Turn the ES module into a script the shim can run: drop the import lines
// (their bindings are shimmed below) and the `export` keyword.
const asScript = source
  .replace(/^import .*$/gm, '')
  .replace(/^export default \{/gm, 'globalThis.__surface = {')
  .replace(/^export (const|function)/gm, '$1')
  // `import.meta.url` is module-only, and a dynamic `import()` inside a vm
  // script would escape the shim; both are replaced with sandbox globals.
  .replaceAll('import.meta.url', '__moduleUrl')
  .replaceAll(/\bimport\(/g, '__dynamicImport(');
vm.runInContext(asScript, context, { filename: 'plugin.js' });

vm.runInContext(
  'globalThis.__glue = { default: async () => {}, start: async () => { globalThis.__started = true; } };',
  context,
);
// Stands in for the dynamic import of the wasm-bindgen glue, and records the
// specifier so the test can assert it is relative.
sandbox.__dynamicImport = async (spec) => {
  context.__imported.push(spec);
  return context.__glue;
};
context.__imported = [];

const surface = context.__surface;


/* ── The contract ──────────────────────────────────────────────────────── */

assert.equal(surface.name, 'filmcraft', 'the surface must name the plugin');
for (const m of ['mount', 'unmount', 'getElement']) {
  assert.equal(typeof surface[m], 'function', `default export needs ${m}()`);
}

const tile = surface.mount();
assert.ok(tile, 'mount() must return an element');
assert.equal(tile.tagName, 'SECTION', 'the tile must be a <section>');
assert.ok(tile.classList.contains('tile'), 'the tile needs core\'s `tile` class');
assert.equal(tile.dataset.plugin, 'filmcraft', 'data-plugin must identify the window');
// `dataset.windowBar = ''` is what core looks for to lift the window controls
// into the merged header, so assert on the property rather than on the key.
const header = tile.children.find((c) => c.dataset.windowBar !== undefined);
assert.ok(header, 'the tile needs a header carrying data-window-bar');
assert.ok(tile.classList.contains('filmcraft-tile'), 'the tile carries the plugin prefix for plugin.css');

/* ── The stylesheet comes from the plugin's own path ───────────────────── */

await new Promise((r) => setTimeout(r, 0));
const link = document.head.children[0];
assert.ok(link && link.rel === 'stylesheet', 'plugin.css must be requested');
assert.match(link.href, /\/plugins\/filmcraft\/plugin\.css$/, `stylesheet path was ${link.href}`);

/* ── Boot: relative glue, the canvas id, and the wasm as bytes ─────────── */

await new Promise((r) => setTimeout(r, 50));
assert.ok(
  context.__imported.includes('./filmcraft_web.js'),
  `the glue must be imported relatively, got ${JSON.stringify(context.__imported)}`,
);
assert.equal(context.__started, true, 'the editor must be started');

// The editor instance is appended to the stage; find the canvas by walking it.
function findCanvas(root) {
  for (const child of root.children) {
    if (child.tagName === 'CANVAS') return child;
    const found = findCanvas(child);
    if (found) return found;
  }
  return null;
}
const stage = tile.children.find((c) => c.classList.contains('filmcraft-stage'));
assert.ok(stage, 'the tile needs a stage the editor fills');
const canvas = findCanvas(stage);
assert.ok(canvas, 'the editor canvas must be mounted in the stage');
assert.equal(canvas.id, 'filmcraft_canvas', 'the canvas id must match the app\'s start(canvas)');
byId.set('filmcraft_canvas', canvas);
assert.ok(canvas.classList.contains('filmcraft-canvas'), 'the canvas carries the plugin prefix for plugin.css');

/* ── The relay ─────────────────────────────────────────────────────────── */

let delivered = null;
responder = async () => {
  if (delivered) return null;
  delivered = { id: 7, method: 'engine.execute', params: { command: 'project.inspect' } };
  return { success: true, data: delivered };
};

await new Promise((r) => setTimeout(r, 700));
const posted = calls.find((c) => c.path.endsWith('/result'));
assert.ok(posted, 'the window must post the answer back');
const body = JSON.parse(posted.options.body);
assert.equal(body.id, 7, 'the answer must carry the request id');
assert.equal(body.ok, true, 'a successful command reports ok');
assert.equal(body.result.from, 'engine', 'engine.execute must run through window.filmcraft');

/* ── Unmount detaches, keeps the instance ──────────────────────────────── */

const holder = canvas.parentElement;
const unmounted = surface.unmount();
assert.equal(unmounted, null, 'unmount() should return null');
assert.ok(!holder.parentElement, 'closing the tile must detach the editor holder');
assert.equal(canvas.parentElement, holder, 'the canvas itself is kept alive, not destroyed');
assert.equal(byId.get('filmcraft_canvas'), canvas, 'the instance is kept for the next mount');

// Core drops the tile element on deactivate, so a fresh one is built — what
// must survive is the editor itself: one wasm start, one canvas, still mounted.
const again = surface.mount();
assert.ok(again && again.classList.contains('tile'), 're-opening must build a tile again');
assert.ok(again !== tile, 'the closed tile is not resurrected');
assert.equal(holder.parentElement?.classList.contains('filmcraft-stage'), true,
  're-opening must re-attach the editor holder to the new tile');
assert.equal(context.__imported.filter((s) => s === './filmcraft_web.js').length, 1,
  'the wasm module must still be started only once');

// Close it again so the poll timer is cleared, then exit: the timer would
// otherwise keep the process alive.
surface.unmount();
console.log('filmcraft window smoke: ok');
process.exit(0);