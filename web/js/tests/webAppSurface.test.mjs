/**
 * Web-app plugin window surface test.
 *
 * A plugin with `kind = "web"` in its `plugin.toml` ships no `web/plugin.js`:
 * its payload is a static site the core serves at `/plugins/<name>/`, mounted
 * in a tile as an iframe. This file pins the contract the tile manager
 * consumes (`mount` / `unmount` / `getElement` / `wireEvents`), the iframe's
 * URL and capabilities, and two deliberate choices:
 *
 *   • no `sandbox` attribute — sandboxing same-origin content is a no-op
 *     unless `allow-same-origin` is dropped, and without it the app loses the
 *     storage (IndexedDB/OPFS) that holds the user's files;
 *   • the same element comes back on every `mount()`, because detaching and
 *     re-adding a tile reloads any iframe inside it (tiles.js reconciles the
 *     tile grid in place for exactly that reason).
 *
 * The repo's JS tests have no DOM, so this file brings a stub of document —
 * enough for the surface builder, which touches nothing else.
 *
 * Run:  node web/js/tests/webAppSurface.test.mjs
 */

let failures = 0;
function check(name, condition, detail = '') {
  if (condition) {
    console.log(`  ok  ${name}`);
  } else {
    failures += 1;
    console.error(`FAIL  ${name}${detail ? ` — ${detail}` : ''}`);
  }
}

/* ── A document, just enough of one ──────────────────────────── */

const created = [];

globalThis.document = {
  createElement(tag) {
    const el = {
      tagName: String(tag).toUpperCase(),
      className: '',
      dataset: {},
      attributes: {},
      children: [],
      listeners: {},
      setAttribute(name, value) { this.attributes[name] = String(value); },
      getAttribute(name) { return Object.hasOwn(this.attributes, name) ? this.attributes[name] : null; },
      appendChild(child) { this.children.push(child); return child; },
      addEventListener(type, listener) { (this.listeners[type] ||= []).push(listener); },
      fire(type, event = {}) { for (const listener of this.listeners[type] || []) listener(event); },
    };
    created.push(el);
    return el;
  },
};

/* ── Browser globals the bridge and its answer path touch ────── */

const windowListeners = {};
globalThis.window = {
  addEventListener(type, listener) { (windowListeners[type] ||= []).push(listener); },
  removeEventListener(type, listener) {
    windowListeners[type] = (windowListeners[type] || []).filter((fn) => fn !== listener);
  },
  dispatchEvent(event) {
    for (const listener of windowListeners[event.type] || []) listener(event);
    return true;
  },
};

const memory = new Map();
Object.defineProperty(globalThis, 'localStorage', {
  configurable: true,
  value: {
    getItem: (key) => (memory.has(key) ? memory.get(key) : null),
    setItem: (key, value) => memory.set(key, String(value)),
    removeItem: (key) => memory.delete(key),
  },
});

const sources = [];
globalThis.EventSource = class {
  constructor(url) {
    this.url = String(url);
    this.listeners = {};
    this.closed = false;
    sources.push(this);
  }
  addEventListener(type, listener) { this.listeners[type] = listener; }
  close() { this.closed = true; }
};

const posted = [];
globalThis.fetch = async (url, options = {}) => {
  // The file picker probes the Files plugin; none is installed in this test, so
  // the shim must degrade quietly (no dialog, no throw) instead of hanging.
  if (String(url).includes('/api/files/home')) {
    return new Response('{"error":"no files plugin"}', {
      status: 404,
      headers: { 'Content-Type': 'application/json' },
    });
  }
  if (String(url).includes('/api/files/raw')) {
    return new Response(new Uint8Array([7, 8]), {
      status: 200,
      headers: { 'Content-Type': 'application/pdf' },
    });
  }
  if (String(url).includes('/api/plugins/craft/control/reply')) {
    posted.push({ url: String(url), body: options.body ? JSON.parse(options.body) : null });
  }
  return new Response('{"success":true}', {
    status: 200,
    headers: { 'Content-Type': 'application/json' },
  });
};

const { webAppSurface, forgetWebAppSurface } = await import('../webAppSurface.js');
const defaultExport = (await import('../webAppSurface.js')).default;

/* ── The surface contract ────────────────────────────────────── */

check('default export is the factory', defaultExport === webAppSurface);

const surface = webAppSurface('photocraft');
check('surface carries the plugin name', surface.name === 'photocraft');
check('surface exposes the window contract',
  typeof surface.mount === 'function'
  && typeof surface.unmount === 'function'
  && typeof surface.getElement === 'function'
  && typeof surface.wireEvents === 'function',
  JSON.stringify(Object.keys(surface).sort()));

const tile = surface.mount();
check('mount returns a section with the plugin identity',
  tile.tagName === 'SECTION' && tile.dataset.plugin === 'photocraft'
  && tile.className.split(' ').includes('tile'), JSON.stringify(tile.className));
check('the tile is marked as a web-app window', tile.className.includes('tile--webapp'));
check('getElement returns the mounted element', surface.getElement() === tile);
check('wireEvents is safe to call', surface.wireEvents() === undefined);
check('unmount keeps the element (the shell detaches it)', surface.unmount() === undefined
  && surface.getElement() === tile);

/* ── The iframe ──────────────────────────────────────────────── */

const frames = created.filter((el) => el.tagName === 'IFRAME');
check('exactly one iframe per surface', frames.length === 1, `got ${frames.length}`);
const frame = frames[0];

check('the iframe points at the plugin document root',
  /^file:\/\/.*\/plugins\/photocraft\/$/.test(frame.src), frame.src);
check('the iframe is styled and identified',
  frame.className === 'webapp-frame' && frame.dataset.plugin === 'photocraft');
check('the frame is allowed the capabilities the apps ask for',
  ['fullscreen', 'clipboard-read', 'clipboard-write', 'autoplay']
    .every((token) => frame.attributes.allow.includes(token)),
  frame.attributes.allow);
check('the frame may go fullscreen', frame.attributes.allowfullscreen === '');
check('no sandbox attribute: a same-origin app keeps its storage',
  frame.getAttribute('sandbox') === null);
check('the iframe lives inside the tile', tile.children.includes(frame));

/* ── Stability across re-renders ─────────────────────────────── */

check('the same surface object comes back', webAppSurface('photocraft') === surface);
check('re-mounting reuses the same element (no iframe reload)',
  webAppSurface('photocraft').mount() === tile);
check('another plugin gets its own surface',
  webAppSurface('filmcraft') !== surface
  && webAppSurface('filmcraft').mount() !== tile);

forgetWebAppSurface('photocraft');
const rebuilt = webAppSurface('photocraft');
check('forgetting a surface rebuilds it', rebuilt !== surface && rebuilt.mount() !== tile);

/* ── Live AI control: the page half of the control channel ───── */

const vsurface = webAppSurface('vectorcraft');
const vframe = vsurface.mount().children.find((el) => el.tagName === 'IFRAME');

// A build without the page API attaches nothing; the headless server stays in
// charge and nothing about the window changes.
vframe.contentWindow = {};
vframe.fire('load');
check('no page API: nothing is attached', sources.length === 0, `${sources.length} sources`);

// A build with it attaches one stream, looked up by plugin id.
const calls = [];
vframe.contentWindow = {
  vectorcraft: {
    request: async (method, params) => { calls.push({ method, params }); return { method }; },
  },
};
vframe.fire('load');
check('the page API opens one control stream', sources.length === 1, `${sources.length}`);
check('the stream is scoped to this app id',
  sources[0].url.includes('/api/plugins/craft/control/events?app=vectorcraft'), sources[0].url);

// A forwarded request runs in the app and its answer goes back on the wire.
sources[0].listeners.message({ data: JSON.stringify({
  id: 7, method: 'engine.execute', params: { command: 'select.all' },
}) });
await new Promise((resolve) => setTimeout(resolve, 10));
check('the request reaches the app with its params',
  calls.length === 1 && calls[0].method === 'engine.execute'
  && calls[0].params.command === 'select.all', JSON.stringify(calls));
check('the answer goes back to the proxy',
  posted.length === 1 && posted[0].url.includes('/api/plugins/craft/control/reply')
  && posted[0].body.app === 'vectorcraft' && posted[0].body.id === 7
  && posted[0].body.ok === true, JSON.stringify(posted));

// An app-side refusal comes back as an error line with the same id.
vframe.contentWindow = {
  vectorcraft: { request: async () => { throw new Error('nothing selected'); } },
};
vframe.fire('load');
check('a reload replaces the stream', sources.length === 2 && sources[0].closed === true);
sources[1].listeners.message({ data: JSON.stringify({ id: 8, method: 'engine.execute' }) });
await new Promise((resolve) => setTimeout(resolve, 10));
check('an app error is reported, not swallowed',
  posted.length === 2 && posted[1].body.id === 8 && posted[1].body.ok === false
  && posted[1].body.error === 'nothing selected', JSON.stringify(posted[1]));

forgetWebAppSurface('vectorcraft');
check('forgetting a surface closes its stream', sources[1].closed === true);

// The detection is by plugin id, so the same code serves any Craft app.
const other = webAppSurface('photocraft');
const otherFrame = other.mount().children.find((el) => el.tagName === 'IFRAME');
otherFrame.contentWindow = { photocraft: { request: async () => ({}) } };
otherFrame.fire('load');
check('another app id gets its own stream',
  sources.length === 3 && sources[2].url.includes('app=photocraft'), `${sources.length}`);
forgetWebAppSurface('photocraft');
check('its stream closes with it', sources[2].closed === true);

/* ── In-app Open…: file inputs are served by the shell picker ── */

const wsurface = webAppSurface('wordcraft');
const wframe = wsurface.mount().children.find((el) => el.tagName === 'IFRAME');

class RealmInput {
  constructor() {
    this.type = 'file';
    this.disabled = false;
    this.files = null;
    this.events = [];
    this.accept = '';
    this.multiple = false;
    this.nativeClicks = 0;
    this.nativeShows = 0;
  }
  click() { this.nativeClicks += 1; }
  showPicker() { this.nativeShows += 1; }
  dispatchEvent(event) { this.events.push(event.type); }
}
class RealmDataTransfer {
  constructor() {
    this.files = [];
    this.items = { add: (file) => this.files.push(file) };
  }
}
class RealmEvent {
  constructor(type, options = {}) { this.type = type; this.bubbles = !!options.bubbles; }
}
class RealmFile {
  constructor(parts, name, options = {}) { this.parts = parts; this.name = name; this.type = options.type || ''; }
}

wframe.contentWindow = {
  HTMLInputElement: RealmInput,
  DataTransfer: RealmDataTransfer,
  Event: RealmEvent,
  File: RealmFile,
};
wframe.fire('load');

const fileInput = new RealmInput();
fileInput.accept = 'image/*';
fileInput.click();
check('a file input is answered by the shell, not the browser dialog',
  fileInput.nativeClicks === 0, `${fileInput.nativeClicks} native clicks`);
fileInput.showPicker();
check('showPicker is intercepted too', fileInput.nativeShows === 0);

const textInput = new RealmInput();
textInput.type = 'text';
textInput.click();
check('any other input keeps its native behavior', textInput.nativeClicks === 1);

// The picker needs the Files plugin; with it absent nothing throws, and the
// app simply sees no files (the shim degrades quietly).
await new Promise((resolve) => setTimeout(resolve, 10));
check('a missing Files plugin is not an error', fileInput.files === null);

/* ── “Open with app”: Files hands a file to the page API ─────── */

let opened = null;
wframe.contentWindow.wordcraft = {
  openFile(name, bytes) { opened = { name, bytes }; return true; },
};
window.dispatchEvent({
  type: 'files:open',
  detail: { plugin: 'wordcraft', path: 'Documents/notes.docx', name: 'notes.docx' },
});
await new Promise((resolve) => setTimeout(resolve, 20));
check('“Open with app” hands the file to the page API',
  opened?.name === 'notes.docx' && opened.bytes.length === 2,
  JSON.stringify(opened && { name: opened.name, bytes: opened.bytes.length }));

// A build without the API is left alone — the same gate as live control.
opened = null;
wframe.contentWindow.wordcraft = {};
window.dispatchEvent({ type: 'files:open', detail: { plugin: 'wordcraft', path: 'Documents/a.pdf' } });
await new Promise((resolve) => setTimeout(resolve, 10));
check('a build without openFile is not handed a file', opened === null);

forgetWebAppSurface('wordcraft');

console.log(failures === 0
  ? '\nwebAppSurface: all checks passed'
  : `\nwebAppSurface: ${failures} check(s) failed`);
process.exit(failures === 0 ? 0 : 1);
