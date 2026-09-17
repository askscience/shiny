/**
 * Browser-window smoke test.
 *
 * There is no headless browser in this environment, so this exercises the real
 * plugin surface (`plugins/browser/web/plugin.js`) against a small DOM shim:
 * mount the window, drive tab create/close, navigation and the message bridge,
 * and prove the generated sandboxed home document renders and navigates.
 *
 * It is not a substitute for a browser; it is a substitute for *no* test. The
 * value is in the seams a Rust test cannot reach: the tab model, the
 * postMessage contract (including which tab a message belongs to), the link
 * menu, the hover preview, and the srcdoc string.
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
    // A real `classList` and `className` are two views of one attribute, so
    // `classList.add` must be visible in `className`. The window relies on it
    // (it tags its nav buttons and active tab through `classList`).
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
    // The real DOM gives an iframe a `contentWindow` WindowProxy; the plugin
    // compares `event.source` against each tab's, so the shim must provide one.
    contentWindow: null,
    _listeners: {},
    _parent: null,
    append(...kids) {
      for (const kid of kids) { kid._parent = el; el.children.push(kid); }
    },
    appendChild(kid) { kid._parent = el; el.children.push(kid); return kid; },
    remove() {
      const parent = el._parent;
      if (parent) {
        parent.children = parent.children.filter((c) => c !== el);
        el._parent = null;
      }
    },
    replaceChildren(...kids) { el.children = [...kids]; },
    removeAttribute(name) { delete el.attrs[name]; },
    setAttribute(name, value) { el.attrs[name] = String(value); },
    getAttribute(name) { return el.attrs[name]; },
    getBoundingClientRect() { return { left: 0, top: 0, width: 0, height: 0 }; },
    addEventListener(type, fn) { (el._listeners[type] ||= []).push(fn); },
    removeEventListener(type, fn) {
      el._listeners[type] = (el._listeners[type] || []).filter((f) => f !== fn);
    },
    dispatch(type, event) {
      for (const fn of el._listeners[type] || []) fn(event);
    },
    blur() {},
    focus() {},
    closest() { return null; },
  };
  el.classList._owner = el;
  // `dataset` is an attribute view, as in the real DOM: `el.dataset.plugin = x`
  // must produce `data-plugin="x"`, which is what core reads to route the tile.
  el.dataset = new Proxy(
    {},
    {
      set(_t, key, value) {
        const name = `data-${String(key).replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`)}`;
        el.attrs[name] = String(value);
        return true;
      },
      get(_t, key) {
        const name = `data-${String(key).replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`)}`;
        return el.attrs[name];
      },
    },
  );
  return el;
}

const head = makeElement('head');
const body = makeElement('body');
const documentStub = {
  head,
  body,
  activeElement: null,
  createElement: makeElement,
  getElementById: () => null,
  addEventListener: () => {},
  querySelector: () => null,
};

const windowListeners = {};
const windowStub = {
  addEventListener: (type, fn) => { (windowListeners[type] ||= []).push(fn); },
  removeEventListener: (type, fn) => {
    windowListeners[type] = (windowListeners[type] || []).filter((f) => f !== fn);
  },
  document: documentStub,
};

/* ── Module stubs for the core imports ────────────────────────── */

const NEWS_PAYLOAD = {
  cards: [
    {
      title: 'Aurora alert: solar storm could light up northern skies',
      url: 'https://example.com/aurora-alert',
      source: 'Example News',
      snippet: 'NOAA forecasts a G2 storm.',
      image: 'https://imgs.search.brave.com/thumb/aurora.jpg',
      topic: 'aurora',
      age: '2 hours ago',
      score: 7.1,
    },
  ],
  topics: ['aurora'],
  personalized: true,
};

const PREVIEW_PAYLOAD = {
  url: 'https://example.com/a',
  title: 'Example article',
  description: 'What the page is about.',
  image: 'https://example.com/img.png',
  site: 'example.com',
};

const calls = { navigate: [], clicks: [], sessionClose: [], preview: [], state: 0 };
let proxyPortValue = '1234';
const proxyPort = () => proxyPortValue;

const apiFetch = async (path, options = {}) => {
  if (path.startsWith('/api/browser/news/click')) {
    calls.clicks.push(JSON.parse(options.body));
    return { success: true, data: { recorded: true } };
  }
  if (path.startsWith('/api/browser/news')) {
    return { success: true, data: NEWS_PAYLOAD };
  }
  if (path === '/api/browser/state') {
    calls.state += 1;
    return {
      success: true,
      data: {
        proxy_base: `http://127.0.0.1:${proxyPort()}`,
        ready: true,
        metrics: { blocked: 3 },
        rules: 100,
      },
    };
  }
  if (path === '/api/browser/metrics') {
    return { success: true, data: { metrics: { blocked: 3 }, rules: 100, filtering_paused: false } };
  }
  if (path === '/api/browser/session/close') {
    calls.sessionClose.push(JSON.parse(options.body));
    return { success: true, data: { closed: true } };
  }
  if (path === '/api/browser/preview') {
    const body = JSON.parse(options.body);
    calls.preview.push(body);
    return { success: true, data: PREVIEW_PAYLOAD };
  }
  if (path === '/api/browser/navigate') {
    const input = JSON.parse(options.body).input;
    calls.navigate.push({ input });
    return {
      success: true,
      data: {
        session: { id: `s${calls.navigate.length}`, url: input },
        url: input,
        proxy_base: `http://127.0.0.1:${proxyPort()}`,
        view_url: `http://127.0.0.1:${proxyPort()}/p/https/${input.replace(/^https?:\/\//, '')}`,
      },
    };
  }
  return { success: true, data: {} };
};

/* The core UI components the window builds its toolbar from. The shim mirrors
   each one's real shape (class names included), because that shape is what the
   assertions below check: the window must use the library, not roll its own. */
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

/** Spy for the core menu the window opens on a link's right-click. */
const menus = [];
const openContextMenu = (entries, x, y) => {
  menus.push({ entries, x, y });
};

/** Depth-first search of the shim DOM. */
function findAll(root, predicate, out = []) {
  if (predicate(root)) out.push(root);
  for (const kid of root.children || []) findAll(kid, predicate, out);
  return out;
}

const moduleSource = source
  .replace(/^import .*$/gm, '')
  // Re-export whatever the file defines, minus anything it already exports
  // itself (a duplicate export is a SyntaxError).
  .concat(
    `\nexport { ${[
      'mountBrowserTile',
      'unmountBrowserTile',
      'getBrowserTileElement',
      'wireBrowserEvents',
      'browserContextMenu',
    ]
      .filter((name) => !new RegExp(`export\\s+(function|const|let|var)\\s+${name}\\b`).test(source))
      .join(', ')} };\n`,
  );

const context = vm.createContext({
  console,
  document: documentStub,
  window: windowStub,
  apiFetch,
  icon,
  iconButton,
  searchBar,
  openContextMenu,
  navigator: { clipboard: { writeText: () => Promise.resolve() } },
  URL,
  setTimeout,
  clearTimeout,
  setInterval,
  clearInterval,
  JSON,
  String,
  Number,
  Array,
  Object,
  Math,
  RegExp,
  parseInt,
  parseFloat,
  Error,
  Promise,
});

const mod = new vm.SourceTextModule(moduleSource, { context });
await mod.link(() => { throw new Error('unexpected import'); });
await mod.evaluate();

/* ── Assertions ───────────────────────────────────────────────── */

const failures = [];
const check = (name, condition, detail = '') => {
  if (condition) {
    console.log(`  ok   ${name}`);
  } else {
    failures.push(`${name}${detail ? ` — ${detail}` : ''}`);
    console.log(`  FAIL ${name}${detail ? ` — ${detail}` : ''}`);
  }
};
const tick = (ms = 40) => new Promise((resolve) => setTimeout(resolve, ms));

console.log('browser window smoke test');

check('module exports the window surface', typeof mod.namespace.mountBrowserTile === 'function');

const tile = mod.namespace.mountBrowserTile();
// Core calls this once when it wires plugin windows; the window registers its
// global message/artifact listeners there.
mod.namespace.wireBrowserEvents();
check('mount returns a tile element', !!tile);
check('tile is tagged for its plugin', tile.attrs['data-plugin'] === 'browser', tile.attrs['data-plugin']);
check('tile carries the plugin class', tile.className.includes('browser-tile'), tile.className);

await tick(140);

/* Toolbar built from the core UI library. */
const bar = findAll(tile, (el) => el.className === 'browser-bar')[0];
check('the window has a toolbar', !!bar);
const tabstrip = findAll(tile, (el) => el.className === 'browser-tabstrip')[0];
check('the window has a tab strip', !!tabstrip);

const search = findAll(tile, (el) => String(el.className).includes('ui-search'))[0];
check('the address field is the core searchBar', !!search);

const buttons = findAll(tile, (el) => String(el.className).includes('ui-btn--icon'));
const byLabel = (label) => buttons.find((b) => b.attrs['aria-label'] === label);
check('there is a Back button', !!byLabel('Back'));
check('there is a Forward button', !!byLabel('Forward'));
check('there is a Reload control', !!byLabel('Reload'));
check('there is a Home control', !!byLabel('Home'));
check('there is a New tab control', !!byLabel('New tab'));

const frameWrap = findAll(tile, (el) => el.className === 'browser-frames')[0];
check('viewport has a frames container', !!frameWrap);
const frames = () => frameWrap.children.filter((c) => String(c.className).includes('browser-frame'));
const tabEls = () => tabstrip.children.filter((c) => String(c.className).includes('browser-tab') && !String(c.className).includes('browser-tabstrip'));

check('one tab is open on mount', tabEls().length === 1 && frames().length === 1, `${tabEls().length}/${frames().length}`);
check('the open tab is active', frames()[0]?.classList.contains('is-active'));
check('the home surface was rendered', !!frames()[0]?.srcdoc && frames()[0].srcdoc.includes('https://example.com/aurora-alert'));
check('home frame is sandboxed without same-origin', frames()[0]?.attrs?.sandbox === 'allow-scripts');

// Give each frame the WindowProxy a real DOM would, for source attribution.
let fakeWindows = 0;
const wireFrame = (frame) => {
  frame.contentWindow = { postMessage: () => {}, _id: ++fakeWindows };
  return frame;
};
wireFrame(frames()[0]);

/* ── Navigation in the active tab ─────────────────────────────── */

const addressForm = findAll(tile, (el) => el.className === 'browser-address')[0];
addressForm.input.value = 'https://example.com/story';
addressForm.input.blur();
addressForm.dispatch('submit', { preventDefault() {} });
await tick();

check(
  'navigating points the frame at the proxy',
  frames()[0].src === `http://127.0.0.1:1234/p/https/example.com/story`,
  frames()[0].src,
);
check('the address bar reflects the navigation', addressForm.input.value === 'https://example.com/story');

/* ── Message attribution across tabs ──────────────────────────── */

const onMessage = (windowListeners.message || [])[0];
check('the window listens for frame messages', typeof onMessage === 'function');

// Open a second tab via the strip's "+".
byLabel('New tab').dispatch('click', {});
await tick(60);
check('the new-tab control opens a tab', tabEls().length === 2 && frames().length === 2, `${tabEls().length}/${frames().length}`);
wireFrame(frames()[1]);
check('the new tab becomes active', frames()[1].classList.contains('is-active') && !frames()[0].classList.contains('is-active'));

// A report from the active tab updates the address bar.
onMessage({ source: frames()[1].contentWindow, data: { type: 'shiny:location', url: 'https://example.com/two', title: 'Two' } });
check("the active tab's location updates the address bar", addressForm.input.value === 'https://example.com/two');

// A report from the *inactive* tab must not steal the address bar.
onMessage({ source: frames()[0].contentWindow, data: { type: 'shiny:location', url: 'https://example.com/one', title: 'One' } });
check('an inactive tab does not change the address bar', addressForm.input.value === 'https://example.com/two');

// A message from an unknown window is ignored.
const navBefore = calls.navigate.length;
onMessage({ source: {}, data: { type: 'shiny:new-tab', url: 'https://evil.example/x' } });
check('a message from another frame is ignored', calls.navigate.length === navBefore);

/* ── Link affordances from the frame ──────────────────────────── */

onMessage({ source: frames()[1].contentWindow, data: { type: 'shiny:new-tab', url: 'https://example.com/link' } });
await tick(60);
check('a target=_blank link opens a tab', tabEls().length === 3 && calls.navigate.some((c) => c.input === 'https://example.com/link'));
wireFrame(frames()[2]);
// The tab the new-tab action opened is active; hover/leave are active-only.
const activeWindow = () => frames().find((f) => f.classList.contains('is-active'))?.contentWindow;

onMessage({ source: frames()[1].contentWindow, data: { type: 'shiny:link-menu', url: 'https://example.com/ctx', x: 12, y: 20 } });
check('right-click opens the window menu', menus.length === 1);
check(
  'the link menu offers open-in-new-tab',
  (menus[0]?.entries || []).some((e) => e.label === 'Open in new tab'),
  JSON.stringify(menus[0]?.entries?.map((e) => e.label)),
);

/* ── Hover preview ────────────────────────────────────────────── */

onMessage({ source: activeWindow(), data: { type: 'shiny:hover-link', url: 'https://example.com/preview', rect: { x: 5, y: 6, w: 10, h: 12 } } });
await tick(320);
check('hovering a link fetches a preview', calls.preview.some((p) => p.url === 'https://example.com/preview'));
const previewEl = findAll(tile, (el) => String(el.className).includes('browser-preview'))[0];
check('the preview card is shown', !!previewEl && !previewEl.classList.contains('hidden'));
check('the preview card carries the title', String(previewEl?.innerHTML || '').includes('Example article'));

onMessage({ source: activeWindow(), data: { type: 'shiny:leave-link' } });
check('leaving the link hides the preview', previewEl.classList.contains('hidden'));

/* ── Closing a tab ────────────────────────────────────────────── */

const closeBtn = findAll(tabEls()[2], (el) => String(el.className).includes('browser-tab-close'))[0];
check('a tab has a close control', !!closeBtn);
closeBtn.dispatch('click', { stopPropagation() {} });
await tick();
check('closing removes the tab', tabEls().length === 2 && frames().length === 2, `${tabEls().length}/${frames().length}`);
check('closing the tab tells the server', calls.sessionClose.length >= 1);

/* ── AI-driven navigation ─────────────────────────────────────── */

const savedListeners = windowListeners['artifact:saved'] || [];
check('the window listens for saved artifacts', savedListeners.length === 1);
const beforeAi = calls.navigate.length;
savedListeners[0]({
  detail: { id: 'card-1', type: 'browser_page', plugin: 'browser', title: 'x', narrative: 'https://www.ilfattoquotidiano.it' },
});
await tick();
check('an AI-opened page loads in the active tab', calls.navigate.length > beforeAi && calls.navigate.some((c) => c.input === 'https://www.ilfattoquotidiano.it'));

check('context menu exposes entries', (mod.namespace.browserContextMenu?.() || []).length > 0);

mod.namespace.unmountBrowserTile?.();
check('unmount is safe', true);

if (failures.length) {
  console.error(`\n${failures.length} failure(s):`);
  for (const f of failures) console.error(` - ${f}`);
  process.exit(1);
}
console.log('\nall browser window smoke checks passed');
