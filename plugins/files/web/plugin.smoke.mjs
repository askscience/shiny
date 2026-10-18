/**
 * Files-window smoke test.
 *
 * There is no headless browser in this environment, so this exercises the real
 * window (`plugins/files/web/plugin.js`) against a small DOM shim: mount it,
 * wait for the listing, then drive the quick look. Space opens a bare overlay
 * (no window frame around the file), the details ride the bottom scrim, the
 * side arrows step through the folder *in place*, the keyboard still walks the
 * same list, and a player keeps the bottom edge for its own controls.
 *
 * What this protects is the seam a Rust test cannot reach: the quick look's
 * overlay contract and the wiring behind it. The real `modal()`/`transition()`
 * from `web/ui` run here, so the shared overlay lifecycle is under test too.
 *
 * Run:  node --experimental-vm-modules plugins/files/web/plugin.smoke.mjs
 */
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import vm from 'node:vm';

const here = dirname(fileURLToPath(import.meta.url));
const webRoot = join(here, '..', '..', '..', 'web');
const source = readFileSync(join(here, 'plugin.js'), 'utf8');

/* ── The DOM shim ─────────────────────────────────────────────── */

function makeStyle() {
  return {
    _p: {},
    setProperty(k, v) { this._p[k] = String(v); },
    removeProperty(k) { delete this._p[k]; },
    getPropertyValue(k) { return this._p[k] || ''; },
  };
}

/** Enough selector support for this window: `.class`, `tag`, `[attr]`, lists. */
function matchesSimple(el, part) {
  if (!part) return false;
  if (part.includes(':hover')) return true;
  const tag = part.match(/^[a-zA-Z]+/);
  if (tag && el.tagName !== tag[0].toUpperCase()) return false;
  const own = String(el.className || '').split(/\s+/);
  for (const cls of part.matchAll(/\.([\w-]+)/g)) if (!own.includes(cls[1])) return false;
  for (const attr of part.matchAll(/\[([\w-]+)\]/g)) if (el.attrs[attr[1]] === undefined) return false;
  return true;
}
function matchesSelector(el, selector) {
  return String(selector).split(',').some((part) => matchesSimple(el, part.trim()));
}
function descendants(el, out = []) {
  for (const kid of el.children || []) { out.push(kid); descendants(kid, out); }
  return out;
}

/** The shim's own HTMLElement, so `instanceof` checks in core code hold. */
class HTMLElementStub {}

function makeElement(tag) {
  const el = {
    tagName: String(tag).toUpperCase(),
    children: [],
    attrs: {},
    style: makeStyle(),
    classList: {
      _read(node) { return String(node.className || '').split(/\s+/).filter(Boolean); },
      _write(node, list) { node.className = [...new Set(list)].join(' '); },
      add(...c) { this._write(this._owner, [...this._read(this._owner), ...c]); },
      remove(...c) { this._write(this._owner, this._read(this._owner).filter((x) => !c.includes(x))); },
      toggle(c, on) {
        const node = this._owner;
        const has = this._read(node).includes(c);
        const want = on === undefined ? !has : !!on;
        if (want) this.add(c); else this.remove(c);
        return want;
      },
      contains(c) { return this._read(this._owner).includes(c); },
    },
    className: '',
    type: '',
    title: '',
    disabled: false,
    hidden: false,
    inert: false,
    src: '',
    alt: '',
    loading: '',
    decoding: '',
    poster: '',
    playsInline: false,
    value: '',
    min: 0,
    max: 100,
    step: 1,
    width: 0,
    height: 0,
    tabIndex: 0,
    scrollLeft: 0,
    scrollTop: 0,
    clientWidth: 400,
    clientHeight: 300,
    scrubbing: false,
    paused: true,
    muted: false,
    volume: 1,
    currentTime: 0,
    duration: 0,
    ended: false,
    _listeners: {},
    _parent: null,
    append(...kids) { for (const kid of kids) { kid._parent = el; el.children.push(kid); } },
    appendChild(kid) { kid._parent = el; el.children.push(kid); return kid; },
    remove() {
      const parent = el._parent;
      if (parent) { parent.children = parent.children.filter((c) => c !== el); el._parent = null; }
    },
    removeAttribute(name) { delete el.attrs[name]; },
    setAttribute(name, value) { el.attrs[name] = String(value); },
    getAttribute(name) { return el.attrs[name]; },
    querySelector(sel) { return descendants(el).find((node) => matchesSelector(node, sel)) || null; },
    querySelectorAll(sel) { return descendants(el).filter((node) => matchesSelector(node, sel)); },
    contains(node) {
      let walk = node;
      while (walk) { if (walk === el) return true; walk = walk._parent; }
      return false;
    },
    matches(sel) { return matchesSelector(el, sel); },
    closest() { return null; },
    getBoundingClientRect() { return { left: 40, top: 80, width: 400, height: 300, right: 440, bottom: 380 }; },
    getClientRects() { return [{}]; },
    getContext(kind) { return kind === '2d' ? { drawImage() {}, clearRect() {}, putImageData() {} } : null; },
    addEventListener(type, fn) { (el._listeners[type] ||= []).push(fn); },
    removeEventListener(type, fn) {
      el._listeners[type] = (el._listeners[type] || []).filter((f) => f !== fn);
    },
    dispatch(type, event = {}) {
      const detail = { stopPropagation() {}, preventDefault() {}, ...event };
      for (const fn of [...(el._listeners[type] || [])]) fn(detail);
    },
    setPointerCapture() {},
    releasePointerCapture() {},
    scrollIntoView() {},
    blur() {},
    focus() {},
    play() { el.paused = false; return Promise.resolve(); },
    pause() { el.paused = true; },
    load() {},
    requestFullscreen() { return Promise.resolve(); },
    setLoading() {},
    setLabel() {},
  };
  // Real DOM semantics: assigning textContent/html replaces the children.
  let text = '';
  Object.defineProperty(el, 'textContent', {
    get() { return text; },
    set(value) { text = String(value ?? ''); el.children = []; },
  });
  Object.defineProperty(el, 'innerHTML', {
    get() { return ''; },
    set() { el.children = []; },
  });
  // Connected means: somewhere under a root the shim handed to the document.
  Object.defineProperty(el, 'isConnected', {
    get() {
      let walk = el;
      while (walk) { if (walk.__root) return true; walk = walk._parent; }
      return false;
    },
  });
  el.classList._owner = el;
  Object.setPrototypeOf(el, HTMLElementStub.prototype);
  el.dataset = new Proxy({}, {
    set(_t, key, value) {
      el.attrs[`data-${String(key).replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`)}`] = String(value);
      return true;
    },
    get(_t, key) {
      return el.attrs[`data-${String(key).replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`)}`];
    },
  });
  return el;
}

const body = makeElement('body');
body.__root = true;
const documentStub = {
  body,
  activeElement: null,
  documentElement: makeElement('html'),
  fullscreenElement: null,
  createElement: makeElement,
  createElementNS: (_ns, tag) => makeElement(tag),
  getElementById: () => null,
  querySelector: () => null,
  querySelectorAll: () => [],
  exitFullscreen: () => {},
  addEventListener: () => {},
  removeEventListener: () => {},
  dispatchEvent: () => {},
};

const storage = new Map();
const localStorageStub = {
  getItem: (k) => (storage.has(k) ? storage.get(k) : null),
  setItem: (k, v) => { storage.set(k, String(v)); },
  removeItem: (k) => { storage.delete(k); },
};

const windowListeners = {};
const windowStub = {
  addEventListener: (type, fn) => { (windowListeners[type] ||= []).push(fn); },
  removeEventListener: (type, fn) => {
    windowListeners[type] = (windowListeners[type] || []).filter((f) => f !== fn);
  },
  dispatchEvent: () => {},
  confirm: () => true,
  document: documentStub,
  localStorage: localStorageStub,
  setTimeout: () => 0,
  clearTimeout: () => {},
  setInterval: () => 0,
  clearInterval: () => {},
  matchMedia: () => ({ matches: false, addEventListener() {}, removeEventListener() {} }),
};

class IntersectionObserverStub {
  constructor(cb) { this.cb = cb; }
  observe() {}
  unobserve() {}
  disconnect() {}
}

/* ── Module stubs for the core imports ────────────────────────── */

const calls = { toasts: [], notices: [], fetches: [] };

const ENTRIES = [
  { name: 'Pictures', path: 'Pictures', kind: 'dir', count: 4 },
  {
    name: 'cover.png', path: 'Pictures/cover.png', kind: 'file',
    size: 248832, modified: 1700000000, ext: 'png', is_image: true,
  },
  {
    name: 'notes.txt', path: 'Pictures/notes.txt', kind: 'file',
    size: 512, modified: 1700000000, ext: 'txt', is_text: true,
  },
  {
    name: 'holiday.mp4', path: 'Pictures/holiday.mp4', kind: 'file',
    size: 48234496, modified: 1700000000, ext: 'mp4', is_video: true,
  },
];

const apiFetch = async (url) => {
  calls.fetches.push(url);
  if (url.startsWith('/api/files/list')) return { data: { path: '', parent: null, entries: ENTRIES } };
  if (url.startsWith('/api/files/read')) return { data: { content: 'hello from the shim' } };
  if (url.startsWith('/api/files/text')) return { data: { content: 'hello from the shim' } };
  if (url.startsWith('/api/files/video-info')) {
    return { data: { duration: 120, video_codec: 'h264', audio_codec: 'aac' } };
  }
  throw new Error(`unexpected fetch: ${url}`);
};

const button = ({ label = '', icon, variant, size, onClick, disabled } = {}) => {
  const el = makeElement('button');
  el.className = `ui-btn ui-btn--${variant || 'primary'}${size && size !== 'md' ? ` ui-btn--${size}` : ''}`;
  el.type = 'button';
  if (label) el.textContent = label;
  if (disabled) el.disabled = true;
  if (icon) el.appendChild(iconElement(icon));
  if (onClick) el.addEventListener('click', onClick);
  return el;
};
const iconElement = (name) => {
  const el = makeElement('span');
  el.className = 'ui-icon';
  el.attrs['data-icon'] = name;
  return el;
};
const iconButton = ({ icon, label, onClick, variant, size, className } = {}) => {
  const el = button({ icon, variant: variant || 'ghost', size, onClick, className });
  el.classList.add('ui-btn--icon');
  if (label) { el.setAttribute('aria-label', label); el.title = label; }
  return el;
};

const moduleSource = source.replace(/^import\s+[\s\S]*?;[ \t]*\n?/gm, '');
const coreSource = (rel) => readFileSync(join(webRoot, rel), 'utf8')
  .replace(/^import\s+[\s\S]*?;[ \t]*\n?/gm, '')
  .replace(/^export /gm, '');

const context = vm.createContext({
  console,
  document: documentStub,
  window: windowStub,
  localStorage: localStorageStub,
  navigator: {},
  HTMLElement: HTMLElementStub,
  IntersectionObserver: IntersectionObserverStub,
  // The window gets its own timers: nothing here waits on them, and a hover
  // must not hold the process open for the player's idle delay.
  setTimeout: () => 0,
  clearTimeout: () => {},
  setInterval: () => 0,
  clearInterval: () => {},
  // The player drives its scrubber from rAF; the shim never calls back.
  requestAnimationFrame: () => 1,
  cancelAnimationFrame: () => {},
  JSON, String, Number, Array, Object, Math, RegExp, parseInt, parseFloat, Error, Promise, Set, Map, Date, URL,
  // web/ui factories the window imports
  button,
  iconButton,
  icon: (name) => iconElement(name),
  controlButton: () => { const el = makeElement('button'); el.type = 'button'; return el; },
  toolButton: (name, label, onClick) => iconButton({ icon: name, label, onClick, size: 'sm' }),
  setIcon: async () => {},
  toolbar: (children = [], { className = '' } = {}) => {
    const el = makeElement('div');
    el.className = `ui-toolbar ${className}`;
    el.append(...children);
    return el;
  },
  searchBar: ({ placeholder, value, onInput } = {}) => {
    const wrap = makeElement('div');
    wrap.className = 'ui-search';
    const input = makeElement('input');
    input.value = value || '';
    input.placeholder = placeholder || '';
    if (onInput) input.addEventListener('input', () => onInput(input.value));
    wrap.input = input;
    wrap.appendChild(input);
    return wrap;
  },
  input: () => makeElement('input'),
  uiInput: () => makeElement('input'),
  slider: () => makeElement('input'),
  uiSlider: () => makeElement('input'),
  emptyState: ({ title = '', body: text = '' } = {}) => {
    const el = makeElement('div');
    el.className = 'ui-empty';
    const t = makeElement('p');
    t.textContent = title;
    const b = makeElement('p');
    b.textContent = text;
    el.append(t, b);
    return el;
  },
  toast: (text, opts) => { calls.toasts.push({ text, opts }); },
  notify: (payload) => { calls.notices.push(payload); },
  confirmDialog: async () => true,
  promptDialog: async () => null,
  setTileGlow: () => {},
  setTileGlowFromUrl: async () => {},
  glowGradient: (a) => a,
  // plugin-side core imports
  openContextMenu: (entries) => { const el = makeElement('div'); el.entries = entries; return el; },
  copyText: async () => true,
  pluginForFile: () => null,
  openWithPlugin: async () => false,
  pickFiles: async () => [],
  apiFetch,
  // A key with no English text in the shim still renders its fallback.
  t: (key, params, fallback) => fallback ?? key,
  tp: (key, count, params, fallback) => fallback ?? key,
});

// The real overlay lifecycle (modal + shared motion/resources), minus imports.
vm.runInContext(coreSource('ui/motion.js'), context);
vm.runInContext(coreSource('js/resources.js'), context);
vm.runInContext(coreSource('ui/components/overlay.js'), context);

const mod = new vm.SourceTextModule(moduleSource, { context });
await mod.link(() => { throw new Error('unexpected import'); });
await mod.evaluate();

/* ── Assertions ───────────────────────────────────────────────── */

const failures = [];
const check = (name, condition, detail = '') => {
  if (condition) console.log(`  ok   ${name}`);
  else { failures.push(`${name}${detail ? ` — ${detail}` : ''}`); console.log(`  FAIL ${name}${detail ? ` — ${detail}` : ''}`); }
};
const tick = (ms = 40) => new Promise((resolve) => setTimeout(resolve, ms));

const findAll = (root, predicate, out = []) => {
  if (predicate(root)) out.push(root);
  for (const kid of root.children || []) findAll(kid, predicate, out);
  return out;
};
const byClass = (root, cls) => findAll(root, (node) => String(node.className).split(/\s+/).includes(cls));
const inBody = (cls) => byClass(body, cls);
const first = (cls) => inBody(cls)[0];
const press = (key) => {
  for (const fn of [...(windowListeners.keydown || [])]) fn({ key, preventDefault() {} });
};

console.log('files window smoke test');

check('module exports the window surface', typeof mod.namespace.mountFilesTile === 'function');

const tile = mod.namespace.mountFilesTile();
mod.namespace.wireFilesEvents();
body.appendChild(tile);
check('mount returns a tile element', !!tile);
check('tile is tagged for its plugin', tile.attrs['data-plugin'] === 'files', tile.attrs['data-plugin']);
check('the window has a places sidebar and a content pane',
  !!byClass(tile, 'files-sidebar')[0] && !!byClass(tile, 'files-content')[0]);

await tick(80);
check('the listing rendered one entry per file',
  byClass(tile, 'files-entry').length === ENTRIES.length,
  String(byClass(tile, 'files-entry').length));
check('nothing is open before Space', !first('files-look'));

/* Space opens the quick look on the selected file. */
const image = byClass(tile, 'files-entry')[1];
image.dispatch('click', {});
press(' ');
await tick(60);

const look = first('files-look');
check('Space opens the quick look', !!look);
check('the overlay is open', look?.classList.contains('is-open'));
check('the file has no window frame around it',
  !!first('files-look-surface') && inBody('files-preview-card').length === 0);
check('the footer navigation is gone with the frame',
  inBody('files-preview-nav').length === 0
  && inBody('files-preview-meta').length === 0
  && inBody('files-preview-hint').length === 0);
check('the file fills the overlay',
  !!first('files-look-stage') && !!first('files-look-view')
  && first('files-preview-img')?.src.includes('cover.png'),
  first('files-preview-img')?.src);
check('the details ride the bottom scrim',
  first('files-look-name')?.textContent === 'cover.png'
  && (first('files-look-meta')?.textContent || '').startsWith('Pictures/cover.png · 243 KB'),
  `${first('files-look-name')?.textContent} / ${first('files-look-meta')?.textContent}`);
check('an image says it can be zoomed',
  first('files-look-note')?.textContent === 'Click image to zoom',
  first('files-look-note')?.textContent);
check('clicking the image toggles the zoom',
  (first('files-preview-img').dispatch('click', {}),
    first('files-preview-img').classList.contains('is-zoomed')));

const arrows = () => inBody('files-look-arrow');
check('the overlay carries two side arrows', arrows().length === 2, String(arrows().length));
check('the arrows are labelled for the pointer and the screen reader',
  arrows().every((a) => a.attrs['aria-label'] && a.title)
  && arrows()[0].attrs['aria-label'] !== arrows()[1].attrs['aria-label'],
  JSON.stringify(arrows().map((a) => a.attrs['aria-label'])));
check('the arrows sit on the left and right edges',
  arrows()[0].classList.contains('is-prev') && arrows()[1].classList.contains('is-next'));

/* Stepping keeps the same overlay alive — a hover on the arrow survives it.
   The folder sorts by name: cover.png, holiday.mp4, notes.txt. */
arrows()[1].dispatch('click', {});
await tick(60);
check('the arrow steps to the next file',
  first('files-look-name')?.textContent === 'holiday.mp4',
  first('files-look-name')?.textContent);
check('stepping reuses the open overlay', first('files-look') === look);
check('the file behind it stays selected',
  byClass(tile, 'files-entry')[2].classList.contains('is-selected'));
check('a player keeps the bottom edge for its controls',
  first('files-look-surface')?.classList.contains('is-media')
  && !!first('files-player-bar') && !!first('files-player-seek'));
check('the details clear the control bar for a player',
  (first('files-look-meta')?.textContent || '').startsWith('Pictures/holiday.mp4 · 46 MB'),
  first('files-look-meta')?.textContent);

/* The control bar follows the pointer and steps aside while playing. */
const player = first('files-player');
check('the control bar starts up', !!player && !player.classList.contains('is-idle'));
player.dispatch('pointerenter', {});
check('resting on the bar keeps it up', !player.classList.contains('is-idle'));
player.classList.add('is-idle');
player.dispatch('pointermove', {});
check('a pointer move brings the controls back', !player.classList.contains('is-idle'));

/* The keyboard walks the same list. */
press('ArrowRight');
await tick(60);
check('the arrow key steps too',
  first('files-look-name')?.textContent === 'notes.txt',
  first('files-look-name')?.textContent);
check('the text file reads on the surface',
  first('files-preview-text')?.textContent === 'hello from the shim',
  first('files-preview-text')?.textContent);
check('the note counts what is around it',
  first('files-look-note')?.textContent === '3 items · ← → to move',
  first('files-look-note')?.textContent);
check('the file behind it stays selected',
  byClass(tile, 'files-entry')[3].classList.contains('is-selected'));

/* Wrap-around, then close. */
press('ArrowRight');
await tick(60);
check('the last file wraps to the first',
  first('files-preview-img')?.src.includes('cover.png') && !first('files-player-bar'));
check('the media flag follows the file',
  !first('files-look-surface').classList.contains('is-media'));

press('Escape');
await tick(60);
check('Escape closes the quick look', inBody('files-look').length === 0);

/* It opens again — and the window takes it down with itself. */
press(' ');
await tick(60);
check('Space reopens it on the same selection',
  !!first('files-look') && first('files-look-name')?.textContent === 'cover.png');
mod.namespace.unmountFilesTile();
await tick(60);
check('unmount closes the quick look', inBody('files-look').length === 0);
check('unmount takes the tile out of the document', !tile.isConnected);

if (failures.length) {
  console.error(`\n${failures.length} failure(s):`);
  for (const f of failures) console.error(` - ${f}`);
  process.exit(1);
}
console.log('\nall files window smoke checks passed');
