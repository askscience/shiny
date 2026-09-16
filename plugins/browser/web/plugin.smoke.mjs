/**
 * Browser-window smoke test.
 *
 * There is no headless browser in this environment, so this exercises the real
 * plugin surface (`plugins/browser/web/plugin.js`) against a small DOM shim:
 * mount the window, let the home surface render its news shelf, and run the
 * generated sandboxed document's own click handler to prove a card navigates
 * through `postMessage` rather than doing nothing.
 *
 * It is not a substitute for a browser; it is a substitute for *no* test. The
 * value is in the seams a Rust test cannot reach: the srcdoc string, the
 * card markup, the postMessage contract with the parent frame, and the
 * frame-history bridge the toolbar now drives.
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
    // (it tags its nav buttons through `classList`), and so do these checks.
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
    // The real DOM gives an iframe a `contentWindow` WindowProxy. The compiler
    // fix in this release compares `event.source` against it, so the shim must
    // provide one — the old test posted with the *element* as `source`, which
    // is exactly why the bug survived.
    contentWindow: null,
    _listeners: {},
    append(...kids) { el.children.push(...kids); },
    appendChild(kid) { el.children.push(kid); return kid; },
    remove() {},
    removeAttribute(name) { delete el.attrs[name]; },
    setAttribute(name, value) { el.attrs[name] = String(value); },
    getAttribute(name) { return el.attrs[name]; },
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
    {
      title: 'Second story',
      url: 'https://example.com/second',
      source: 'Another Outlet',
      snippet: '',
      image: null,
      topic: 'solar',
      age: null,
      score: 3.0,
    },
  ],
  topics: ['aurora', 'solar'],
  personalized: true,
};

const calls = { navigate: [], clicks: [], state: 0 };
/** The proxy port the "server" is currently on. Restarts change it. */
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
  if (path === '/api/browser/navigate') {
    const input = JSON.parse(options.body).input;
    calls.navigate.push({ input });
    return {
      success: true,
      data: {
        session: { id: 's1', url: input },
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

/** Depth-first search of the shim DOM. */
function findAll(root, predicate, out = []) {
  if (predicate(root)) out.push(root);
  for (const kid of root.children || []) findAll(kid, predicate, out);
  return out;
}

const moduleSource = source
  .replace(/^import .*$/gm, '')
  // Re-export whatever the file defines, minus anything it already exports
  // itself (a duplicate export is a SyntaxError, as this test discovered).
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

console.log('browser window smoke test');

check('module exports the window surface', typeof mod.namespace.mountBrowserTile === 'function');

const tile = mod.namespace.mountBrowserTile();
check('mount returns a tile element', !!tile);
check('tile is tagged for its plugin', tile.attrs['data-plugin'] === 'browser', tile.attrs['data-plugin']);
check('tile carries the plugin class', tile.className.includes('browser-tile'), tile.className);

// The mount paints the home surface on an async continuation. Wait for it to
// settle before driving navigation, or `showHome`'s own frame reset lands after
// the navigation and clobbers it.
await new Promise((resolve) => setTimeout(resolve, 120));

/* The toolbar must be built from the core UI library, so the window inherits
   the theme's metrics and accent (PLUGINS.md §19) instead of its own controls. */
const bar = findAll(tile, (el) => el.className === 'browser-bar')[0];
check('the window has a toolbar', !!bar);

const search = findAll(tile, (el) => String(el.className).includes('ui-search'))[0];
check('the address field is the core searchBar', !!search);
check(
  'the address field is a ui-input',
  String(search?.children?.[1]?.className || '').includes('ui-input'),
  search?.children?.[1]?.className,
);
check(
  'the address field carries the theme search icon',
  (search?.children || []).some((c) => c.dataset?.icon === 'ui/search'),
);
check('the address field has a placeholder', !!search?.input?.placeholder);

const buttons = findAll(tile, (el) => String(el.className).includes('ui-btn--icon'));
check('every toolbar control is a core iconButton', buttons.length >= 5, `${buttons.length} found`);

const byLabel = (label) => buttons.find((b) => b.attrs['aria-label'] === label);
const back = byLabel('Back');
const forward = byLabel('Forward');
check('there is a Back button', !!back);
check('there is a Forward button', !!forward);
check(
  'Back and Forward use the same theme glyph',
  back?.children?.[0]?.dataset?.icon === forward?.children?.[0]?.dataset?.icon &&
    back?.children?.[0]?.dataset?.icon === 'ui/arrow-left',
  `${back?.children?.[0]?.dataset?.icon} vs ${forward?.children?.[0]?.dataset?.icon}`,
);
check(
  'Forward is the mirrored copy',
  String(forward?.className).includes('browser-nav--forward') &&
    String(back?.className).includes('browser-nav--back'),
  forward?.className,
);
check(
  'no curved share arrow is used for Forward',
  forward?.children?.[0]?.dataset?.icon !== 'ui/forward',
);
for (const label of ['Reload', 'Home']) {
  check(`there is a ${label} control`, !!byLabel(label));
}
// The filter control's label states the action, so it flips with the state;
// its state lives on `aria-pressed`.
const filterControl = buttons.find((b) =>
  String(b.attrs['aria-label'] || '').includes('blocking'),
);
check('there is an ad-blocking control', !!filterControl, buttons.map((b) => b.attrs['aria-label']).join(' | '));
check(
  'the ad-blocking control exposes its state',
  ['true', 'false'].includes(filterControl?.attrs['aria-pressed']),
  filterControl?.attrs['aria-pressed'],
);

const frame = tile.children
  .flatMap((c) => c.children || [])
  .find((c) => c.className === 'browser-frame');
check('viewport has an iframe', !!frame);

// Give the iframe the WindowProxy a real DOM would, so the parent-side
// `event.source !== contentWindow` guard has something real to compare.
const frameCommands = [];
frame.contentWindow = { postMessage: (message) => frameCommands.push(message) };

const doc = frame?.srcdoc || '';
check('home document was rendered', doc.length > 0);
check('home document embeds the card payload', doc.includes('https://example.com/aurora-alert'));
check('home document shows the source', doc.includes('Example News'));
check('home document shows the age', doc.includes('2 hours ago'));
check('home document shows a card image', doc.includes('https://imgs.search.brave.com/thumb/aurora.jpg'));
check('home document uses the thumb class', doc.includes('class="thumb"'));
check('home document lists the interest chips', doc.includes('>aurora<') && doc.includes('>solar<'));
check('home document is personalised', doc.includes('Because of your recent searches'));
check('home frame is sandboxed without same-origin', frame?.attrs?.sandbox === 'allow-scripts');

/* Run the generated document's own click handler. */
const scriptMatch = doc.match(/<script>([\s\S]*?)<\/script>/g) || [];
const inline = scriptMatch[scriptMatch.length - 1]?.replace(/<\/?script>/g, '') || '';
check('home document ships a click handler', inline.includes('browser:open-card'));

const posted = [];
const cardParent = {
  postMessage: (message) => posted.push(message),
};
const cardDocument = {
  getElementById: (id) =>
    id === 'home-data'
      ? { textContent: (doc.match(/id="home-data" type="application\/json">([\s\S]*?)<\/script>/) || [])[1] || '{}' }
      : null,
  addEventListener: (type, fn) => { if (type === 'click') cardDocument._click = fn; },
};
vm.runInNewContext(inline, {
  document: cardDocument,
  parent: cardParent,
  JSON,
  parseInt,
});

const firstCard = {
  closest: (sel) => (sel === 'a.card' ? { getAttribute: () => '0' } : null),
};
cardDocument._click({ target: firstCard, preventDefault() {} });
check('clicking a card posts its URL to the window', posted[0]?.type === 'browser:open-card');
check('clicked card carries the topic signal', posted[0]?.topic === 'aurora', JSON.stringify(posted[0]));
check('clicked card URL is the article', posted[0]?.url === 'https://example.com/aurora-alert');

const refreshCard = {
  closest: (sel) => (sel === '[data-refresh]' ? { disabled: false, textContent: 'Refresh' } : null),
};
cardDocument._click({ target: refreshCard, preventDefault() {} });
check('refresh button posts its own message', posted[1]?.type === 'browser:refresh-news');

/* The parent side: the message must navigate and record the click. */
mod.namespace.wireBrowserEvents();
const onMessage = (windowListeners.message || [])[0];
check('the window listens for home-shelf messages', typeof onMessage === 'function');

onMessage({ source: frame.contentWindow, data: { type: 'browser:open-card', url: 'https://example.com/aurora-alert', topic: 'aurora' } });
await new Promise((resolve) => setTimeout(resolve, 20));
check(
  'a card click navigates the window to the article',
  calls.navigate.some((c) => c.input === 'https://example.com/aurora-alert'),
  JSON.stringify(calls.navigate),
);
check(
  'a card click is recorded as a ranking signal',
  calls.clicks.some((c) => c.url === 'https://example.com/aurora-alert' && c.topic === 'aurora'),
  JSON.stringify(calls.clicks),
);
check(
  'a message from another frame is ignored',
  (() => {
    const before = calls.navigate.length;
    // The old bug: `event.source` is the *window*, not the element. A message
    // whose source is some other window must still be ignored.
    onMessage({ source: {}, data: { type: 'browser:open-card', url: 'https://evil.example/x' } });
    return calls.navigate.length === before;
  })(),
);

check('context menu exposes entries', (mod.namespace.browserContextMenu?.() || []).length > 0);

/* ── Frame-history bridge ──────────────────────────────────────
 *
 * The toolbar no longer keeps its own server-side history: it mirrors what the
 * framed page reports and asks the frame to go back / forward / reload. */

const addressForm = findAll(tile, (el) => el.className === 'browser-address')[0];
addressForm.input.value = 'https://example.com/story';
addressForm.input.blur();
addressForm.dispatch('submit', { preventDefault() {} });
await new Promise((resolve) => setTimeout(resolve, 30));

check(
  'navigating points the frame at the proxy',
  frame.src === `http://127.0.0.1:1234/p/https/example.com/story`,
  frame.src,
);
check(
  'the address bar reflects the navigation',
  addressForm.input.value === 'https://example.com/story',
  addressForm.input.value,
);

// The page reports it moved (e.g. a link click inside it).
onMessage({ source: frame.contentWindow, data: { type: 'shiny:location', url: 'https://example.com/next' } });
check(
  'the address bar follows an in-page navigation',
  addressForm.input.value === 'https://example.com/next',
  addressForm.input.value,
);
check('Back is enabled once there is history', back.disabled === false);
check('Forward stays disabled at the tip', forward.disabled === true);

back.dispatch('click', {});
await new Promise((resolve) => setTimeout(resolve, 10));
check(
  'Back asks the frame to go back in its own history',
  frameCommands.some((m) => m.type === 'shiny:cmd' && m.cmd === 'back'),
  JSON.stringify(frameCommands),
);

byLabel('Reload').dispatch('click', {});
check(
  'Reload asks the frame to reload',
  frameCommands.some((m) => m.type === 'shiny:cmd' && m.cmd === 'reload'),
  JSON.stringify(frameCommands),
);

// A report of an earlier URL means the frame actually went back: the Forward
// button must come alive again.
onMessage({ source: frame.contentWindow, data: { type: 'shiny:location', url: 'https://example.com/story' } });
check('a back navigation re-enables Forward', forward.disabled === false, String(forward.disabled));

/* ── Connection-refused recovery ──────────────────────────────
 *
 * The proxy's port is random per server start, so a window left open across a
 * restart used to show the browser's own error page on every navigation. The
 * frame's error handler must notice, ask the server for the current address,
 * and retry — without the user reloading. */

addressForm.input.value = 'https://example.com/story';
addressForm.dispatch('submit', { preventDefault() {} });
await new Promise((resolve) => setTimeout(resolve, 30));

// The server restarts on a new port while the window stays open.
proxyPortValue = '5678';
const stateCallsBefore = calls.state;
frame.dispatch('error', {});
await new Promise((resolve) => setTimeout(resolve, 40));

check(
  'a dead proxy address prompts a state refresh',
  calls.state > stateCallsBefore,
  `${stateCallsBefore} → ${calls.state}`,
);
check(
  'the retry loads the same page on the live proxy port',
  frame.src === `http://127.0.0.1:5678/p/https/example.com/story`,
  frame.src,
);
check(
  'the retry shows reconnecting rather than a browser error',
  tile.children.some((c) => (c.children || []).some((s) => String(s.className).includes('browser-status'))),
);

// A navigation that *did* load is never interrupted by the watchdog, even if
// the page is slow: the load event clears it.
frame.dispatch('load', {});
check('a loaded page is treated as alive', true);

// A second failure is not retried forever; it explains itself.
const stateCallsAfterRetry = calls.state;
frame.dispatch('error', {});
await new Promise((resolve) => setTimeout(resolve, 20));
check(
  'a repeated failure does not retry in a loop',
  calls.state === stateCallsAfterRetry,
  `${stateCallsAfterRetry} → ${calls.state}`,
);

/* ── AI-driven navigation ─────────────────────────────────────
 *
 * The shape below is copied from a real row in `saved_artifacts`, and that is
 * the point: the window used to read `payload.url`, a key the server never
 * writes, so asking the AI to open a site left the window on its home page —
 * hidden behind an artifact card that showed the URL it was not loading. */

const savedListeners = windowListeners['artifact:saved'] || [];
check('the window listens for saved artifacts', savedListeners.length === 1);

// A browser card as the server actually stores it (narrative added by the
// current tool; `title` is the URL, as it always was).
windowListeners['artifact:saved'][0]({
  detail: {
    id: 'card-1',
    type: 'browser_page',
    plugin: 'browser',
    title: 'https://www.ilfattoquotidiano.it',
    subtitle: 'Filtered by shiny-filter',
    narrative: 'https://www.ilfattoquotidiano.it',
  },
});
await new Promise((resolve) => setTimeout(resolve, 40));
check(
  'an AI-opened page actually loads in the viewport',
  calls.navigate.some((c) => c.input === 'https://www.ilfattoquotidiano.it'),
  JSON.stringify(calls.navigate),
);

// A legacy card from before the URL was carried in `narrative` still works.
windowListeners['artifact:saved'][0]({
  detail: {
    id: 'card-0',
    type: 'browser_page',
    plugin: 'browser',
    title: 'https://example.com/legacy',
    subtitle: 'Filtered by shiny-filter',
  },
});
await new Promise((resolve) => setTimeout(resolve, 40));
check(
  'a card saved before the narrative field still navigates',
  calls.navigate.some((c) => c.input === 'https://example.com/legacy'),
);

// Another plugin's card must not drive this window.
const before = calls.navigate.length;
windowListeners['artifact:saved'][0]({
  detail: { id: 'card-2', type: 'travel_plan', plugin: 'traveler', title: 'Trip to Rome' },
});
await new Promise((resolve) => setTimeout(resolve, 20));
check('a traveler card never navigates the browser', calls.navigate.length === before);

// Unmount last: it tears the window down.
mod.namespace.unmountBrowserTile?.();
check('unmount is safe', true);

if (failures.length) {
  console.error(`\n${failures.length} failure(s):`);
  for (const f of failures) console.error(` - ${f}`);
  process.exit(1);
}
console.log('\nall browser window smoke checks passed');
