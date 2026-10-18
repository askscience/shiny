/**
 * Image-window smoke test.
 *
 * There is no headless browser in this environment, so this exercises the real
 * window (`plugins/image/web/editor.js`) against a small DOM shim: mount the
 * window, wait for the document load, then drive the dock — the look filmstrip
 * (preview strip + apply), the sliders (preview vs commit vs re-centre), the
 * one-clicks, and the agent/files integrations.
 *
 * What this protects is the seam a Rust test cannot reach: the dock contract
 * and the `agent:actions` / `files:open` wiring.
 *
 * Run:  node --experimental-vm-modules plugins/image/web/plugin.smoke.mjs
 */
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import vm from 'node:vm';

const here = dirname(fileURLToPath(import.meta.url));
const source = readFileSync(join(here, 'editor.js'), 'utf8');

/* ── The DOM shim ─────────────────────────────────────────────── */

function makeContext2d() {
  return {
    imageSmoothingEnabled: true,
    putImageData() {},
    drawImage() {},
    clearRect() {},
  };
}

function makeStyle() {
  return {
    _p: {},
    setProperty(k, v) { this._p[k] = String(v); },
    removeProperty(k) { delete this._p[k]; },
    getPropertyValue(k) { return this._p[k] || ''; },
  };
}

function makeElement(tag) {
  const el = {
    tagName: String(tag).toUpperCase(),
    children: [],
    attrs: {},
    style: makeStyle(),
    classList: {
      _read(el) { return String(el.className || '').split(/\s+/).filter(Boolean); },
      _write(el, list) { el.className = [...new Set(list)].join(' '); },
      add(...c) { const el = this._owner; this._write(el, [...this._read(el), ...c]); },
      remove(...c) { const el = this._owner; this._write(el, this._read(el).filter((x) => !c.includes(x))); },
      toggle(c, on) {
        const el = this._owner;
        const has = this._read(el).includes(c);
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
    src: '',
    alt: '',
    loading: '',
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
    querySelector(sel) {
      const want = String(sel).replace(/^\./, '');
      const stack = [...(el.children || [])];
      while (stack.length) {
        const node = stack.shift();
        if (String(node.className || '').split(/\s+/).includes(want)) return node;
        stack.push(...(node.children || []));
      }
      return null;
    },
    querySelectorAll(sel) {
      const want = String(sel).replace(/^\./, '');
      const out = [];
      const stack = [...(el.children || [])];
      while (stack.length) {
        const node = stack.shift();
        if (String(node.className || '').split(/\s+/).includes(want)) out.push(node);
        stack.push(...(node.children || []));
      }
      return out;
    },
    getBoundingClientRect() {
      return { left: 40, top: 80, width: 400, height: 300, right: 440, bottom: 380 };
    },
    getContext(kind) { return kind === '2d' ? makeContext2d() : null; },
    addEventListener(type, fn) { (el._listeners[type] ||= []).push(fn); },
    removeEventListener(type, fn) {
      el._listeners[type] = (el._listeners[type] || []).filter((f) => f !== fn);
    },
    dispatch(type, event = {}) { for (const fn of el._listeners[type] || []) fn(event); },
    setPointerCapture() {},
    releasePointerCapture() {},
    scrollIntoView() {},
    blur() {},
    focus() {},
    closest() { return null; },
  };
  // Real DOM semantics: assigning textContent replaces the children.
  let text = '';
  Object.defineProperty(el, 'textContent', {
    get() { return text; },
    set(value) {
      text = String(value ?? '');
      el.children = [];
    },
  });
  el.classList._owner = el;
  el.dataset = new Proxy({}, {
    set(_t, key, value) {
      const name = `data-${String(key).replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`)}`;
      el.attrs[name] = String(value);
      return true;
    },
    get(_t, key) {
      const name = `data-${String(key).replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`)}`;
      return el.attrs[name];
    },
  });
  return el;
}

const body = makeElement('body');
const documentStub = {
  body,
  activeElement: null,
  createElement: makeElement,
  createElementNS: (_ns, tag) => makeElement(tag),
  getElementById: () => null,
  querySelector: () => null,
  querySelectorAll: () => [],
  addEventListener: () => {},
  removeEventListener: () => {},
  dispatchEvent: () => {},
};

const windowListeners = {};
const windowStub = {
  addEventListener: (type, fn) => { (windowListeners[type] ||= []).push(fn); },
  removeEventListener: (type, fn) => {
    windowListeners[type] = (windowListeners[type] || []).filter((f) => f !== fn);
  },
  confirm: () => true,
  dispatchEvent: () => {},
  document: documentStub,
  setTimeout: (...args) => setTimeout(...args),
  clearTimeout: (id) => clearTimeout(id),
  setInterval: (...args) => setInterval(...args),
  clearInterval: (id) => clearInterval(id),
};

/* ── Module stubs for the core imports ────────────────────────── */

const LOOKS = [
  { key: 'portra', name: 'Portra' },
  { key: 'noir', name: 'Noir' },
  { key: 'neon', name: 'Neon' },
  { key: 'pixel', name: 'Pixel' },
];

const calls = {
  rawApply: [],
  createImage: [],
  renders: 0,
  listImages: 0,
  metas: 0,
  looks: 0,
  toasts: [],
  pickFiles: 0,
  contextMenu: [],
  revokes: 0,
};

class BlobStub {
  constructor() { this.size = 3; this.type = 'image/png'; }
}

const META = {
  doc1: { image_id: 'doc1', title: 'photo.png', width: 4, height: 4, updated_at: 'now', looks: LOOKS, look: null, adjust: {} },
  doc2: { image_id: 'doc2', title: 'new.png', width: 4, height: 4, updated_at: 'now', looks: LOOKS, look: null, adjust: {} },
};

const api = {
  listImages: async () => {
    calls.listImages += 1;
    return {
      images: [{ image_id: 'doc1', title: 'photo.png', width: 4, height: 4, updated_at: 'now' }],
      count: 1,
    };
  },
  fetchImageMeta: async (id) => {
    calls.metas += 1;
    return META[id] || null;
  },
  fetchRender: async () => {
    calls.renders += 1;
    return { w: 4, h: 4, buf: new ArrayBuffer(4 * 4 * 4) };
  },
  rawApply: async (id, body, opts) => {
    calls.rawApply.push({ id, body, opts });
    if (body.settings) {
      // Mirror the server: partial merge into the live dial state.
      const state = { ...(calls.settingsState || {}) };
      for (const [k, v] of Object.entries(body.settings)) {
        if (!v) delete state[k]; else state[k] = v;
      }
      calls.settingsState = state;
      return { w: 4, h: 4, buf: new ArrayBuffer(4 * 4 * 4), look: calls.lookState || null, adjust: state };
    }
    // Operations: a lone look op swaps the selection (replaceable, not baked);
    // anything else bakes settings + look away (destructive edit).
    const ops = body.operations;
    const pure = ops.length === 1 && ops[0]?.op === 'look';
    const look = pure ? (ops[0].name || null) : null;
    calls.settingsState = pure ? (calls.settingsState || {}) : {};
    calls.lookState = look;
    return { w: 4, h: 4, buf: new ArrayBuffer(4 * 4 * 4), look, adjust: calls.settingsState };
  },
  fetchLooks: async () => {
    calls.looks += 1;
    return new BlobStub();
  },
  createImage: async (file) => {
    calls.createImage.push(file);
    return { image_id: 'doc2', title: 'new.png', width: 4, height: 4 };
  },
  deleteImage: async () => ({}),
  fetchPng: async () => new BlobStub(),
  blobFromHome: async () => new BlobStub(),
};

const button = ({ label = '', icon, variant, size, onClick, disabled } = {}) => {
  const el = makeElement('button');
  el.className = `ui-btn ui-btn--${variant || 'primary'}${size && size !== 'md' ? ` ui-btn--${size}` : ''}`;
  el.type = 'button';
  if (label) el.textContent = label;
  if (disabled) el.disabled = true;
  if (icon) el.appendChild(makeElement('span'));
  if (onClick) el.addEventListener('click', onClick);
  return el;
};

const setIcon = async () => {};
const toast = (text, opts) => { calls.toasts.push({ text, opts }); };
const slider = ({ min = 0, max = 100, step = 1, value = 0, onInput } = {}) => {
  const el = makeElement('input');
  el.type = 'range';
  el.min = min;
  el.max = max;
  el.step = step;
  el.value = String(value);
  if (onInput) el.addEventListener('input', () => onInput(Number(el.value), {}));
  return el;
};
const emptyState = ({ title = '', body: text = '' } = {}) => {
  const el = makeElement('div');
  el.className = 'ui-empty';
  const t = makeElement('p');
  t.className = 'ui-empty-title';
  t.textContent = title;
  const b = makeElement('p');
  b.textContent = text;
  el.append(t, b);
  return el;
};
const openContextMenu = (entries) => { calls.contextMenu.push(entries); return makeElement('div'); };
const saveOrDownload = async () => {};
const pickFiles = async () => {
  calls.pickFiles += 1;
  return [new FileStub('picked.png')];
};
const onOpenFromFiles = () => {};
class FileStub {
  constructor(name) { this.name = name; this.type = 'image/png'; this.size = 3; }
}

const moduleSource = source.replace(/^import\s+[\s\S]*?;[ \t]*\n?/gm, '');

const context = vm.createContext({
  console, document: documentStub, window: windowStub,
  button, setIcon, slider, toast, emptyState, openContextMenu,
  confirmDialog: async () => true,
  el: (tag, cls, text) => { const node = makeElement(tag); node.className = cls || ''; if (text != null) node.textContent = text; return node; },
  toolButton: (name, label, onClick) => button({ icon: name, label, onClick }),
  toolbar: (children = [], { className = '' } = {}) => { const node = makeElement('div'); node.className = `ui-toolbar ${className}`; node.append(...children); return node; },
  controlButton: () => { const node = makeElement('button'); node.type = 'button'; return node; },
  t: (key, params, fallback) => fallback ?? key,
  saveOrDownload, pickFiles, onOpenFromFiles, api,
  URL: {
    createObjectURL: () => 'blob:test',
    revokeObjectURL: () => { calls.revokes += 1; },
  },
  ImageData: class { constructor(data, w, h) { this.data = data; this.width = w; this.height = h; } },
  ResizeObserver: class { constructor(cb) { this.cb = cb; } observe() {} disconnect() {} },
  File: FileStub,
  CustomEvent: class { constructor(type, opts) { this.type = type; this.detail = opts?.detail; } },
  Element: class {},
  navigator: {},
  URLSearchParams,
  setTimeout, clearTimeout, setInterval, clearInterval,
  JSON, String, Number, Array, Object, Math, RegExp, parseInt, parseFloat, Error, Promise, Set, Map,
  Blob: BlobStub, Date,
});

// Exercise the real shared dial implementation against the DOM shim.
vm.runInContext(readFileSync(join(here, '../../../web/ui/components/dial.js'), 'utf8').replace('export function dial', 'function dial'), context);
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

function findAll(root, predicate, out = []) {
  if (predicate(root)) out.push(root);
  for (const kid of root.children || []) findAll(kid, predicate, out);
  return out;
}
const findByClass = (root, cls) => findAll(root, (el) => String(el.className).split(/\s+/).includes(cls));
const byLabel = (root, label) => findAll(root, (el) => el.attrs?.['aria-label'] === label)[0];

console.log('image window smoke test');

check('module exports the window surface', typeof mod.namespace.mountEditor === 'function');

const tile = mod.namespace.mountEditor();
mod.namespace.wireEditorEvents();
check('mount returns a tile element', !!tile);
check('tile is tagged for its plugin', tile.attrs['data-plugin'] === 'image', tile.attrs['data-plugin']);
check('tile nominates a window bar', 'data-window-bar' in (findByClass(tile, 'image-appbar')[0]?.attrs || {}));
check('the stage has a canvas', !!findByClass(tile, 'image-view')[0]);
check('there is no layers UI', findByClass(tile, 'image-layer-row').length === 0);
check('the dock has the look row and the dial bank', !!findByClass(tile, 'image-look-row')[0] && !!findByClass(tile, 'image-dial-row')[0]);

await tick(80);

/* The document loaded. */
const title = findByClass(tile, 'image-appbar-title')[0];
check('the title shows the open image', title?.textContent === 'photo.png', title?.textContent);
check('the zoom chip shows a percentage', /%$/.test(findByClass(tile, 'image-zoom-chip')[0]?.textContent || ''), findByClass(tile, 'image-zoom-chip')[0]?.textContent);
check('the pixels rendered', calls.renders >= 1, String(calls.renders));

/* The look filmstrip: one thumbnail per look, painted from the strip PNG. */
const chips = () => findByClass(tile, 'image-look-chip');
const lookRow = findByClass(tile, 'image-look-row')[0];
check('the filmstrip has one chip per look', chips().length === LOOKS.length, String(chips().length));
check('chips are labelled (no visible text needed)', chips().every((c, i) => c.attrs['aria-label'] === LOOKS[i].name), JSON.stringify(chips().map((c) => c.attrs['aria-label'])));
check('chips carry their index for the strip slice', chips().every((c, i) => c.style.getPropertyValue('--i') === String(i)));
check('the preview strip was fetched and painted', calls.looks >= 1 && lookRow.style.getPropertyValue('--look-url').includes('blob:'), lookRow.style.getPropertyValue('--look-url'));
check('the strip knows its look count', lookRow.style.getPropertyValue('--look-n') === String(LOOKS.length));

/* Looks are a replaceable selection: pick one, swap it, clear it. */
const before = calls.rawApply.length;
chips()[0].dispatch('click', {});
await tick(40);
check('a chip selects its look', calls.rawApply[before]?.body?.operations?.[0]?.op === 'look' && calls.rawApply[before]?.body?.operations?.[0]?.name === 'portra', JSON.stringify(calls.rawApply[before]));
check('the selected chip is ringed', chips()[0].classList.contains('is-active'));
const looksBefore = calls.looks;
chips()[2].dispatch('click', {});
await tick(40);
check(
  'picking another look replaces the selection',
  calls.rawApply.at(-1)?.body?.operations?.[0]?.name === 'neon' && chips()[2].classList.contains('is-active') && !chips()[0].classList.contains('is-active'),
  JSON.stringify(calls.rawApply.at(-1)),
);
await tick(600);
check('swapping a look does not refetch the strip', calls.looks === looksBefore, `${looksBefore} -> ${calls.looks}`);
chips()[2].dispatch('click', {});
await tick(40);
check(
  'clicking the selected look clears it',
  calls.rawApply.at(-1)?.body?.operations?.[0]?.op === 'look' && !calls.rawApply.at(-1)?.body?.operations?.[0]?.name && chips().every((c) => !c.classList.contains('is-active')),
  JSON.stringify(calls.rawApply.at(-1)),
);

/* Dials are persistent settings: values stay put, fine-tuning replaces. */
const dials = findByClass(tile, 'ui-dial');
check('the dock has the eight adjustment dials', dials.length === 8, String(dials.length));
const dialValue = (d) => findByClass(d, 'ui-dial-value')[0].textContent;
const dialArc = (d) => findByClass(d, 'ui-dial-arc')[0].attrs['stroke-dasharray'];
const beforeDial = calls.rawApply.length;
dials[0].dispatch('pointerdown', { clientY: 200, pointerId: 1, preventDefault() {} });
dials[0].dispatch('pointermove', { clientY: 130, shiftKey: false });
await tick(30);
const preview = calls.rawApply[beforeDial];
check('dragging previews the value without committing', preview?.opts.commit === false && preview.body.settings?.brightness === 88, JSON.stringify(preview));
check('the arc fills for the drag value', dialArc(dials[0]) === '33 100', dialArc(dials[0]));
dials[0].dispatch('pointerup', { pointerId: 1 });
await tick(30);
check(
  'releasing commits the setting',
  calls.rawApply.at(-1)?.body?.settings?.brightness === 88 && calls.rawApply.at(-1)?.opts.commit === true,
  JSON.stringify(calls.rawApply.at(-1)),
);
check('the dial STAYS at its value', dialValue(dials[0]) === '+88', dialValue(dials[0]));

/* Fine-tune the same dial down: the value is replaced, never stacked. */
dials[0].dispatch('pointerdown', { clientY: 200, pointerId: 1, preventDefault() {} });
dials[0].dispatch('pointermove', { clientY: 216, shiftKey: false });
await tick(30);
check('fine-tuning sends the new value, not a delta', calls.rawApply.at(-1)?.body?.settings?.brightness === 68, JSON.stringify(calls.rawApply.at(-1)));
dials[0].dispatch('pointerup', { pointerId: 1 });
await tick(30);
check('the dial follows the fine-tune', dialValue(dials[0]) === '+68', dialValue(dials[0]));

/* Arrow keys nudge the stored setting; double-click resets one dial. */
dials[1].dispatch('keydown', { key: 'ArrowUp', shiftKey: false, preventDefault() {} });
await tick(30);
check(
  'an arrow key nudges the setting',
  calls.rawApply.at(-1)?.body?.settings?.contrast === 1 && calls.rawApply.at(-1)?.opts.commit === true,
  JSON.stringify(calls.rawApply.at(-1)),
);
check('the nudged dial stays', dialValue(dials[1]) === '+1', dialValue(dials[1]));
dials[1].dispatch('dblclick', {});
await tick(30);
check('double-click resets one dial', calls.rawApply.at(-1)?.body?.settings?.contrast === 0 && dialValue(dials[1]) === '0', dialValue(dials[1]));

/* A settings commit refreshes the strip previews (trailing debounce). */
const looksAfterDial = calls.looks;
await tick(650);
check('committing refreshes the previews', calls.looks > looksAfterDial, String(calls.looks));

/* One-clicks, including revert. A destructive edit clears the live dials. */
const oneClicks = findByClass(tile, 'image-oneclick');
check('the one-click row has five actions plus revert', oneClicks.length === 6, String(oneClicks.length));
oneClicks.find((c) => c.textContent === 'Invert').dispatch('click', {});
await tick(30);
check('one-click invert commits', calls.rawApply.at(-1)?.body?.operations?.[0]?.op === 'invert', JSON.stringify(calls.rawApply.at(-1)));
check('a destructive edit clears the dials', dialValue(dials[0]) === '0' && dialValue(dials[1]) === '0', `${dialValue(dials[0])} / ${dialValue(dials[1])}`);
calls.rawApply.length = 0;
oneClicks.find((c) => c.textContent === 'Revert').dispatch('click', {});
await tick(30);
check('revert sends the reset op', calls.rawApply[0]?.body?.operations?.[0]?.op === 'reset', JSON.stringify(calls.rawApply[0]));

/* The AI refresh path: `agent:actions` reloads the open document. */
const rendersBefore = calls.renders;
const actionListeners = windowListeners['agent:actions'] || [];
check('the window listens for agent actions', actionListeners.length === 1);
if (actionListeners[0]) {
  actionListeners[0]({ detail: [{ action: 'image_edit', result: 'ok', data: { image_id: 'doc1' } }] });
  await tick(80);
  check('an AI edit refreshes the pixels', calls.renders > rendersBefore, String(calls.renders));
}

/* Files → window import (full metadata is fetched so looks survive). */
await mod.namespace.importFromFiles('Pictures/x.png', 'x.png');
await tick(60);
check('importing from Files uploads it as a new document', calls.createImage.length === 1, String(calls.createImage.length));
check('the imported document becomes the open one', title.textContent === 'new.png', title.textContent);
check('the filmstrip rebuilt for the imported document', chips().length === LOOKS.length, String(chips().length));

/* The menu offers the core actions. */
const menu = mod.namespace.editorContextMenu();
check('the context menu offers revert + delete', menu.some((e) => e.label === 'Revert to original') && menu.some((e) => e.label === 'Delete image'), JSON.stringify(menu.map((e) => e.label)));
calls.contextMenu.length = 0;
mod.namespace.openMainMenu(tile);
const entries = calls.contextMenu.at(-1) || [];
check('the main menu offers rotate + flip', entries.some((e) => e.label === 'Rotate right') && entries.some((e) => e.label === 'Flip horizontal'), JSON.stringify(entries.map((e) => e.label)));

/* Rotate goes through the operations engine. */
calls.rawApply.length = 0;
await mod.namespace.rotateCurrent('right');
check('rotating sends the rotate op', calls.rawApply[0]?.body?.operations?.[0]?.op === 'rotate' && calls.rawApply[0]?.body?.operations?.[0]?.angle === 90, JSON.stringify(calls.rawApply[0]));

/* Unmount is clean: listeners dropped, preview URL revoked. */
const revokesBefore = calls.revokes;
mod.namespace.unmountEditor();
check('unmount drops the key listener', (windowListeners['keydown'] || []).length === 0, String((windowListeners['keydown'] || []).length));
check('unmount revokes the preview strip URL', calls.revokes > revokesBefore);

if (failures.length) {
  console.error(`\n${failures.length} failure(s):`);
  for (const f of failures) console.error(` - ${f}`);
  process.exit(1);
}
console.log('\nall image window smoke checks passed');
