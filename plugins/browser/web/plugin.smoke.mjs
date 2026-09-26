/**
 * Browser-window smoke test.
 *
 * There is no headless browser in this environment, so this exercises the real
 * plugin surface (`plugins/browser/web/plugin.js`) against a small DOM shim:
 * mount the window, drive tab create/close, navigation, the native IPC
 * contract, the shield toggle, incognito tabs and the downloads panel.
 *
 * The window renders pages in a native child webview owned by the shell, so
 * this test asserts the *commands* the window sends (`window.ipc.postMessage`)
 * rather than iframe URLs. It is not a substitute for a browser; it is a
 * substitute for *no* test. The value is in the seams a Rust test cannot reach:
 * the tab model, the checksum of the IPC contract, the shield POST + IPC, the
 * incognito flag, the home `srcdoc` and the downloads panel.
 *
 * Run:  node --experimental-vm-modules plugins/browser/web/plugin.smoke.mjs
 */
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import vm from 'node:vm';

const here = dirname(fileURLToPath(import.meta.url));
const source = readFileSync(join(here, 'plugin.js'), 'utf8');

/* ── The DOM shim ─────────────────────────────────────────────── */

function makeElement(tag) {
  const el = {
    tagName: String(tag).toUpperCase(),
    children: [],
    attrs: {},
    style: {},
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
    textContent: '',
    innerHTML: '',
    className: '',
    type: '',
    title: '',
    disabled: false,
    src: '',
    value: '',
    placeholder: '',
    contentWindow: null,
    _listeners: {},
    _parent: null,
    _queryOne: null,
    append(...kids) { for (const kid of kids) { kid._parent = el; el.children.push(kid); } },
    appendChild(kid) { kid._parent = el; el.children.push(kid); return kid; },
    remove() {
      const parent = el._parent;
      if (parent) { parent.children = parent.children.filter((c) => c !== el); el._parent = null; }
    },
    replaceChildren(...kids) { el.children = [...kids]; for (const k of kids) k._parent = el; },
    removeAttribute(name) { delete el.attrs[name]; },
    setAttribute(name, value) { el.attrs[name] = String(value); },
    getAttribute(name) { return el.attrs[name]; },
    querySelector(sel) {
      // Only the badge lookup uses this; return the first matching descendant.
      const want = String(sel).replace(/^\./, '');
      const stack = [...(el.children || [])];
      while (stack.length) {
        const node = stack.shift();
        if (String(node.className || '').split(/\s+/).includes(want)) return node;
        stack.push(...(node.children || []));
      }
      return null;
    },
    getBoundingClientRect() { return { left: 0, top: 0, width: 0, height: 0 }; },
    addEventListener(type, fn) { (el._listeners[type] ||= []).push(fn); },
    removeEventListener(type, fn) {
      el._listeners[type] = (el._listeners[type] || []).filter((f) => f !== fn);
    },
    dispatch(type, event) { for (const fn of el._listeners[type] || []) fn(event); },
    blur() {},
    focus() {},
    closest() { return null; },
  };
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

const head = makeElement('head');
const body = makeElement('body');
const documentStub = {
  head, body,
  activeElement: null,
  createElement: makeElement,
  getElementById: () => null,
  addEventListener: () => {},
  querySelector: () => null,
};

/** Capture everything the window posts to the shell's IPC bridge. */
const ipcMessages = [];
const windowListeners = {};
const windowStub = {
  addEventListener: (type, fn) => { (windowListeners[type] ||= []).push(fn); },
  removeEventListener: (type, fn) => {
    windowListeners[type] = (windowListeners[type] || []).filter((f) => f !== fn);
  },
  document: documentStub,
  // Present at import time, so `nativeAvailable` is true.
  ipc: { postMessage: (message) => ipcMessages.push(String(message)) },
};

/* ── Module stubs for the core imports ────────────────────────── */

const NEWS_PAYLOAD = {
  cards: [{
    title: 'Aurora alert: solar storm could light up northern skies',
    url: 'https://example.com/aurora-alert',
    source: 'Example News',
    snippet: 'NOAA forecasts a G2 storm.',
    image: 'https://imgs.search.brave.com/thumb/aurora.jpg',
    topic: 'aurora',
    age: '2 hours ago',
    score: 7.1,
  }],
  topics: ['aurora'],
  personalized: true,
};

const calls = { navigate: [], settings: [], downloadEvents: [], downloadRemove: [], sessionClose: [], state: 0, clears: 0 };

const apiFetch = async (path, options = {}) => {
  if (path === '/api/browser/settings' && (!options.method || options.method === 'GET')) {
    return { success: true, data: { adblock: true, incognito: false, downloads_dir: '/home/u/Downloads', rules: 12345 } };
  }
  if (path === '/api/browser/settings') {
    calls.settings.push(JSON.parse(options.body));
    return { success: true, data: JSON.parse(options.body) };
  }
  if (path.startsWith('/api/browser/downloads/event')) {
    calls.downloadEvents.push(JSON.parse(options.body));
    return { success: true, data: { recorded: true } };
  }
  if (path.startsWith('/api/browser/downloads/remove')) {
    calls.downloadRemove.push(JSON.parse(options.body));
    return { success: true, data: { removed: true } };
  }
  if (path.startsWith('/api/browser/downloads/clear')) {
    calls.clears += 1;
    return { success: true, data: { cleared: true } };
  }
  if (path.startsWith('/api/browser/downloads')) {
    return { success: true, data: { downloads: [{ id: 'h1', file: 'older.pdf', host: 'x', state: 'completed' }] } };
  }
  if (path.startsWith('/api/browser/news/click')) return { success: true, data: { recorded: true } };
  if (path.startsWith('/api/browser/news')) return { success: true, data: NEWS_PAYLOAD };
  if (path === '/api/browser/session/close') {
    calls.sessionClose.push(JSON.parse(options.body));
    return { success: true, data: { closed: true } };
  }
  if (path === '/api/browser/navigate') {
    const body = JSON.parse(options.body);
    calls.navigate.push(body);
    return { success: true, data: { session: { id: `s${calls.navigate.length}` }, url: body.input } };
  }
  return { success: true, data: {} };
};

const icon = (name, { size = 18, label = null } = {}) => {
  const el = makeElement('span');
  el.className = 'ui-icon';
  el.dataset.icon = name;
  el.attrs['data-icon'] = name;
  if (label) el.attrs['aria-label'] = label;
  el.style.width = `${size}px`;
  el.style.height = `${size}px`;
  return el;
};

const iconButton = ({ icon: name = 'ui/info', label = '', size, onClick, disabled } = {}) => {
  const el = makeElement('button');
  el.className = `ui-btn ui-btn--ghost ui-btn--icon${size && size !== 'md' ? ` ui-btn--${size}` : ''}`;
  el.type = 'button';
  el.title = label;
  el.setAttribute('aria-label', label);
  el.setAttribute('title', label);
  if (disabled) el.disabled = true;
  el.appendChild(icon(name, { size: size === 'sm' ? 16 : 18 }));
  if (onClick) el.addEventListener('click', onClick);
  return el;
};

const searchBar = ({ placeholder = 'Search…', value } = {}) => {
  const wrap = makeElement('div');
  wrap.className = 'ui-search';
  wrap.appendChild(icon('ui/search'));
  const el = makeElement('input');
  el.className = 'ui-input';
  el.type = 'text';
  el.placeholder = placeholder;
  if (value != null) el.value = value;
  wrap.appendChild(el);
  wrap.input = el;
  return wrap;
};

/** Files-plugin helpers imported by the window. */
const openWithPlugin = async () => {};
const pluginForFile = () => null;
const revealInFiles = () => {};

function findAll(root, predicate, out = []) {
  if (predicate(root)) out.push(root);
  for (const kid of root.children || []) findAll(kid, predicate, out);
  return out;
}

/* The transform strips `import` lines, then re-exports what the file defines. */
const moduleSource = source
  .replace(/^import .*$/gm, '')
  .concat(
    `\nexport { ${['mountBrowserTile', 'unmountBrowserTile', 'getBrowserTileElement', 'wireBrowserEvents', 'browserContextMenu']
      .filter((name) => !new RegExp(`export\\s+(function|const|let|var)\\s+${name}\\b`).test(source))
      .join(', ')} };\n`,
  );

const context = vm.createContext({
  console, document: documentStub, window: windowStub,
  apiFetch, icon, iconButton, searchBar, openWithPlugin, pluginForFile, revealInFiles,
  navigator: { clipboard: { writeText: () => Promise.resolve() } },
  URL, setTimeout, clearTimeout, setInterval, clearInterval,
  JSON, String, Number, Array, Object, Math, RegExp, parseInt, parseFloat, Error, Promise, Set, Map,
});

const mod = new vm.SourceTextModule(moduleSource, { context });
await mod.link(() => { throw new Error('unexpected import'); });
await mod.evaluate();

/* ── Assertions ───────────────────────────────────────────────── */

const failures = [];
const check = (name, condition, detail = '') => {
  if (condition) console.log(`  ok   ${name}`);
  else { failures.push(`${name}${detail ? ` — ${detail}` : ''}`); console.log(`  FAIL ${name}${detail ? ` — ${detail}` : ''}`); }
};
const tick = (ms = 50) => new Promise((resolve) => setTimeout(resolve, ms));

const viewCommands = () => ipcMessages
  .filter((m) => m.startsWith('peakd:view:'))
  .map((m) => JSON.parse(m.slice('peakd:view:'.length)));
const rawCommands = () => ipcMessages.filter((m) => !m.startsWith('peakd:view:'));
const lastView = (op) => viewCommands().filter((c) => c.op === op).pop();

console.log('browser window smoke test');

check('module exports the window surface', typeof mod.namespace.mountBrowserTile === 'function');

const tile = mod.namespace.mountBrowserTile();
mod.namespace.wireBrowserEvents();
check('mount returns a tile element', !!tile);
check('tile is tagged for its plugin', tile.attrs['data-plugin'] === 'browser', tile.attrs['data-plugin']);

await tick(160);

const bar = findAll(tile, (el) => String(el.className).includes('ui-btn--icon'));
const byLabel = (label) => bar.find((b) => b.attrs['aria-label'] === label);
check('there is a Back button', !!byLabel('Back'));
check('there is a Forward button', !!byLabel('Forward'));
check('there is a Reload control', !!byLabel('Reload'));
check('there is a Home control', !!byLabel('Home'));
check('there is a New tab control', !!byLabel('New tab'));
const shieldControl = findAll(tile, (el) => String(el.className).includes('browser-shield'))[0];
check('there is an Ad blocking (shield) control', !!shieldControl);
check(
  'the shield reflects ad blocking in its label',
  /Ad blocking/.test(shieldControl?.attrs?.['aria-label'] || ''),
  shieldControl?.attrs?.['aria-label'],
);
check('there is a Downloads control', !!byLabel('Downloads'));
check('there is a New incognito tab control', !!byLabel('New incognito tab'));

/* The shell was told the settings + where downloads go. */
const settingsMsg = rawCommands().find((m) => m.startsWith('peakd:settings:'));
check('mount pushes settings to the shell', !!settingsMsg, rawCommands().join(' | ').slice(0, 120));
if (settingsMsg) {
  const parsed = JSON.parse(settingsMsg.slice('peakd:settings:'.length));
  check('settings carry adblock + downloads dir', parsed.adblock === true && parsed.downloadsDir === '/home/u/Downloads', JSON.stringify(parsed));
}

/* Home surface renders in the sandboxed local frame. */
const frameWrap = findAll(tile, (el) => String(el.className) === 'browser-frames')[0];
const frames = () => (frameWrap?.children || []).filter((c) => String(c.className).includes('browser-frame'));
check('viewport has a frames container', !!frameWrap);
check('one tab is open on mount', frames().length === 1, String(frames().length));
check('the home surface is sandboxed', frames()[0]?.attrs?.sandbox === 'allow-scripts');
check('the home surface lists news', String(frames()[0]?.srcdoc || '').includes('https://example.com/aurora-alert'));

/* Navigation: a URL submit reaches the server and opens a native view. */
const addressForm = findAll(tile, (el) => String(el.className).includes('browser-address'))[0];
addressForm.input.value = 'https://example.com/story';
addressForm.input.blur();
addressForm.dispatch('submit', { preventDefault() {} });
await tick(80);
check('navigating tells the server', calls.navigate.some((n) => n.input === 'https://example.com/story'));
const navOpen = lastView('open');
check('navigating opens a native view', navOpen?.op === 'open' && navOpen.url === 'https://example.com/story', JSON.stringify(navOpen));
check('a normal tab is not incognito', navOpen?.incognito === false);
check('the address bar reflects the navigation', addressForm.input.value === 'https://example.com/story');

/* The shield toggles ad blocking and tells the shell. */
const shield = shieldControl;
shield.dispatch('click', {});
await tick(30);
check('the shield persists the choice', calls.settings.some((s) => s.adblock === false), JSON.stringify(calls.settings));
check('the shield tells the shell to disable', rawCommands().some((m) => m === 'peakd:filter:{"enabled":false}'), rawCommands().join(' | ').slice(0, 120));
check('the shield reflects its off state', String(shield.className).includes('is-off'));
shield.dispatch('click', {});
await tick(30);
check('the shield can be turned back on', rawCommands().some((m) => m === 'peakd:filter:{"enabled":true}'));

/* Incognito: the tab is private, and navigating it asks for the OTR profile. */
const beforeIncognito = viewCommands().filter((c) => c.op === 'open').length;
byLabel('New incognito tab').dispatch('click', {});
await tick(40);
check(
  'the incognito button opens a private tab',
  !!findAll(tile, (el) => String(el.className).includes('browser-tab') && String(el.className).includes('is-incognito'))[0],
);
addressForm.input.value = 'https://private.example/';
addressForm.input.blur();
addressForm.dispatch('submit', { preventDefault() {} });
await tick(80);
const incognitoOpen = lastView('open');
check('a private navigation opens a view', viewCommands().filter((c) => c.op === 'open').length > beforeIncognito);
check('the incognito tab asks for the OTR profile', incognitoOpen?.incognito === true, JSON.stringify(incognitoOpen));
check(
  'the context menu offers incognito',
  (mod.namespace.browserContextMenu?.() || []).some((e) => e.label === 'New incognito tab'),
);

/* Downloads: shell events feed the panel and get persisted. */
windowStub.__peakdViewEvent({
  type: 'download',
  kind: 'completed',
  download: { id: 'd1', file: 'pexels-cat.jpg', host: 'images.pexels.com', state: 'completed', total: 2048, received: 2048 },
});
await tick(30);
check('a download event reaches the panel', !!findAll(tile, (el) => String(el.className).includes('browser-download is-completed'))[0]);
check('a download event is persisted', calls.downloadEvents.some((d) => d.id === 'd1'));

windowStub.__peakdViewEvent({
  type: 'download',
  kind: 'progress',
  download: { id: 'd2', file: 'big.iso', host: 'example.com', state: 'downloading', total: 100, received: 50 },
});
await tick(30);
const badge = findAll(tile, (el) => String(el.className).includes('browser-downloads-badge'))[0];
check('the badge counts active downloads', badge?.textContent === '1', badge?.textContent);

const panel = findAll(tile, (el) => String(el.className).includes('browser-downloads-panel'))[0];
byLabel('Downloads').dispatch('click', {});
await tick(60);
check('the downloads button opens the panel', panel && !panel.classList.contains('hidden'));
check('the panel lists history + live downloads', panel?.children?.[1]?.children?.length >= 2, String(panel?.children?.[1]?.children?.length));

/* Closing a tab tells the server. */
const firstTabClose = findAll(tile, (el) => String(el.className).includes('browser-tab-close'))[0];
firstTabClose.dispatch('click', { stopPropagation() {} });
await tick(40);
check('closing a tab tells the server', calls.sessionClose.length >= 1);

/* AI-driven navigation. */
const savedListeners = windowListeners['artifact:saved'] || [];
check('the window listens for saved artifacts', savedListeners.length === 1);
if (savedListeners[0]) {
  const before = calls.navigate.length;
  savedListeners[0]({ detail: { type: 'browser_page', title: 'x', narrative: 'https://www.ilfattoquotidiano.it' } });
  await tick(60);
  check('an AI-opened page loads in the active tab', calls.navigate.length > before && calls.navigate.some((n) => n.input === 'https://www.ilfattoquotidiano.it'));
}

mod.namespace.unmountBrowserTile?.();
check('unmount is safe', true);

if (failures.length) {
  console.error(`\n${failures.length} failure(s):`);
  for (const f of failures) console.error(` - ${f}`);
  process.exit(1);
}
console.log('\nall browser window smoke checks passed');
