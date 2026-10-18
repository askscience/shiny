/**
 * Fullscreen state machine test.
 *
 * A fullscreen window owns a freshly created workspace that remembers where the
 * window came from (`home`); several windows can hold fullscreen at once, one
 * workspace each; and the chrome takeover follows the workspace on view. The
 * browser's real fullscreen is only an enhancement — and the two halves must
 * never fight:
 *
 *   • entering a window while another is already fullscreen leaves nothing and
 *     requests nothing. Leaving and re-requesting is a race the browser always
 *     wins (the exit lands *after* the request), and the late `fullscreenchange`
 *     used to tear the new window straight back out of its workspace;
 *   • an exit we asked for is never mistaken for Escape, and Escape ends only
 *     the fullscreen of the window on screen.
 *
 * The repo's JS tests have no DOM, so this file brings a stub of
 * document / window / localStorage / matchMedia — enough for desktop.js and
 * fullscreen.js, which never touch tiles.js.
 *
 * Run:  node web/js/tests/fullscreen.test.mjs
 */

import { readFileSync } from 'node:fs';

let failures = 0;
function check(name, condition, detail = '') {
  if (condition) {
    console.log(`  ok  ${name}`);
  } else {
    failures += 1;
    console.error(`FAIL  ${name}${detail ? ` — ${detail}` : ''}`);
  }
}

/* ── A browser, just enough of one ───────────────────────────── */

const listeners = new Map(); // target -> type -> [fn]

function fire(target, event) {
  for (const fn of [...(listeners.get(target)?.get(event.type) || [])]) fn(event);
}

/** Event/listener plumbing, mixed into the fake window and document. */
function emitter(target) {
  target.addEventListener = (type, fn) => {
    if (typeof fn !== 'function') return;
    if (!listeners.has(target)) listeners.set(target, new Map());
    const byType = listeners.get(target);
    byType.set(type, [...(byType.get(type) || []), fn]);
  };
  target.removeEventListener = (type, fn) => {
    const fns = listeners.get(target)?.get(type);
    if (!fns) return;
    const i = fns.indexOf(fn);
    if (i >= 0) fns.splice(i, 1);
  };
  target.dispatchEvent = (event) => { fire(target, event); return true; };
  return target;
}

class FakeEvent {
  constructor(type, init = {}) {
    this.type = String(type);
    this.detail = init.detail ?? null;
  }

  preventDefault() {}
}

function classListOf() {
  const set = new Set();
  return {
    add: (...names) => names.forEach((n) => set.add(n)),
    remove: (...names) => names.forEach((n) => set.delete(n)),
    contains: (name) => set.has(name),
    toggle: (name, on) => {
      const want = on === undefined ? !set.has(name) : !!on;
      if (want) set.add(name);
      else set.delete(name);
      return want;
    },
    toString: () => [...set].join(' '),
  };
}

function element(tag = 'div') {
  return emitter({
    tagName: String(tag).toUpperCase(),
    className: '',
    style: (() => {
      const props = new Map();
      return {
        setProperty: (name, value) => props.set(name, String(value)),
        removeProperty: (name) => props.delete(name),
        getPropertyValue: (name) => props.get(name) ?? '',
      };
    })(),
    dataset: {},
    children: [],
    textContent: '',
    innerHTML: '',
    classList: classListOf(),
    setAttribute() {},
    getAttribute: () => null,
    removeAttribute() {},
    appendChild(child) { this.children.push(child); return child; },
    append(...nodes) { nodes.forEach((n) => this.appendChild(n)); },
    remove() {},
    querySelector: () => null,
    querySelectorAll: () => [],
    getBoundingClientRect: () => ({ width: 0, height: 0, top: 0, left: 0 }),
  });
}

/** The fake fullscreen transport: `requestFullscreen` rejects while a
 *  fullscreen element exists, `exitFullscreen` clears it in a *later* task and
 *  fires the event then — exactly the asynchrony that used to break the second
 *  window. Both event names are fired for one transition, like the browsers that
 *  keep the webkit alias alive. */
const transport = { requests: 0, exits: 0, element: null, pending: [] };

function queueTransition(fn) { transport.pending.push(fn); }

function fireFullscreenChange() {
  fire(document, new FakeEvent('fullscreenchange'));
  fire(document, new FakeEvent('webkitfullscreenchange'));
}

/** Run every queued fullscreen transition, then let promises settle. */
async function flushBrowser() {
  while (transport.pending.length) transport.pending.shift()();
  await new Promise((resolve) => setTimeout(resolve, 0));
}

const documentElement = emitter(element('html'));
documentElement.requestFullscreen = () => {
  transport.requests += 1;
  if (transport.element) {
    return Promise.reject(new TypeError('Document already in fullscreen'));
  }
  transport.element = documentElement;
  queueTransition(fireFullscreenChange);
  return Promise.resolve();
};

const body = emitter(element('body'));

const document = emitter({
  documentElement,
  body,
  createElement: (tag) => element(tag),
  createElementNS: () => element('svg'),
  getElementById: () => null,
  querySelector: () => null,
  querySelectorAll: () => [],
});

// The fullscreen element is the transport's state, read the way the browser
// reports it (both names — the webkit alias included).
Object.defineProperty(document, 'fullscreenElement', {
  get: () => transport.element,
  configurable: true,
});
Object.defineProperty(document, 'webkitFullscreenElement', {
  get: () => transport.element,
  configurable: true,
});

document.exitFullscreen = () => {
  transport.exits += 1;
  if (!transport.element) return Promise.reject(new TypeError('Document not in fullscreen'));
  queueTransition(() => {
    transport.element = null;
    fireFullscreenChange();
  });
  return Promise.resolve();
};

const mediaQuery = (query) => ({
  media: query,
  matches: false,
  addEventListener() {},
  removeEventListener() {},
});

const window = emitter({
  innerWidth: 1440,
  innerHeight: 900,
  matchMedia: mediaQuery,
  requestAnimationFrame: (cb) => setTimeout(cb, 0),
  location: { href: 'http://localhost/' },
});

const storage = new Map();
const localStorage = {
  getItem: (key) => (storage.has(key) ? storage.get(key) : null),
  setItem: (key, value) => { storage.set(key, String(value)); },
  removeItem: (key) => { storage.delete(key); },
  clear: () => storage.clear(),
};

function defineGlobal(name, value) {
  Object.defineProperty(globalThis, name, { value, writable: true, configurable: true });
}

defineGlobal('window', window);
defineGlobal('document', document);
defineGlobal('localStorage', localStorage);
defineGlobal('Event', FakeEvent);
defineGlobal('CustomEvent', FakeEvent);
defineGlobal('requestAnimationFrame', window.requestAnimationFrame);
defineGlobal('fetch', () => Promise.reject(new Error('no network in tests')));

// The desktop state is persisted only with "remember" on (preferences.js).
localStorage.setItem('session.remember', 'true');

/* ── The modules under test (imported after the stubs) ───────── */

const {
  initDesktop, ensureWindows, getWorkspacesList, switchWorkspace,
  activeWorkspaceIndex, workspaceLock,
} = await import('../desktop.js');
const {
  initFullscreen, toggleWindowFullscreen, exitWindowFullscreen,
  fullscreenWindow, isAppFullscreen,
} = await import('../fullscreen.js');

initDesktop();
ensureWindows(['alpha', 'beta', 'gamma']);
initFullscreen();

const list = () => getWorkspacesList();
/** The dedicated workspace of `name` — the only workspaces that carry a name. */
const dedicated = (name) => list().find((ws) => ws.name && ws.windows.join() === name) || null;
const idOf = (index) => list()[index].id;
const onWorkspaceOf = (name) => switchWorkspace(list().findIndex((ws) => ws.name === name));

console.log('fullscreen: per-window workspaces');

/* ── 1. The first fullscreen window ──────────────────────────── */

toggleWindowFullscreen('alpha');
check('alpha owns the screen', fullscreenWindow() === 'alpha');
check('the chrome follows the workspace on view', body.classList.contains('fs-active'));
check('alpha got a dedicated workspace named after it', dedicated('alpha')?.name === 'Alpha');
check('it remembers where alpha came from', dedicated('alpha')?.home === idOf(0));
check('the workspace is sealed around alpha', workspaceLock(dedicated('alpha')) === 'alpha');
check('real fullscreen was requested once', transport.requests === 1, `requests=${transport.requests}`);
check('nothing was left', transport.exits === 0, `exits=${transport.exits}`);
check('the home is persisted with the workspace',
  [...storage.values()].some((v) => String(v).includes('"Alpha"') && String(v).includes('"home"')));

await flushBrowser();
check('the browser granted it', isAppFullscreen() === true);
check('the duplicate fullscreenchange did not disturb it', fullscreenWindow() === 'alpha');
check('the request was not repeated', transport.requests === 1, `requests=${transport.requests}`);

/* ── 2. Looking at another workspace ─────────────────────────── */

switchWorkspace(0);
check('a workspace with no fullscreen window reports none', fullscreenWindow() === null);
check('…and gets its chrome back', !body.classList.contains('fs-active'));
check('alpha keeps its own workspace', dedicated('alpha')?.fullscreen === 'alpha');
check('real fullscreen was not dropped', transport.exits === 0 && isAppFullscreen() === true);

/* ── 3. The second window: its own workspace, no fighting ────── */

toggleWindowFullscreen('beta');
check('beta got its own workspace', dedicated('beta')?.name === 'Beta');
check('…holding only beta, fullscreen', dedicated('beta')?.windows.join() === 'beta'
  && dedicated('beta')?.fullscreen === 'beta');
check('beta remembers its home', dedicated('beta')?.home === idOf(0));
check('alpha is untouched', dedicated('alpha')?.fullscreen === 'alpha'
  && dedicated('alpha')?.windows.join() === 'alpha');
check('the browser was not made to leave', transport.exits === 0, `exits=${transport.exits}`);
check('no second request was made', transport.requests === 1, `requests=${transport.requests}`);

await flushBrowser();
check('beta is still fullscreen after every browser event', fullscreenWindow() === 'beta');
check('…in its own workspace', dedicated('beta')?.fullscreen === 'beta');
check('the chrome is on beta', body.classList.contains('fs-active'));

/* ── 4. A third window, and three screens at once ────────────── */

switchWorkspace(0);
toggleWindowFullscreen('gamma');
await flushBrowser();
check('three windows hold three screens at once',
  list().filter((ws) => ws.fullscreen).length === 3);
check('each of them is alone in its workspace',
  ['alpha', 'beta', 'gamma'].every((n) => dedicated(n)?.windows.length === 1));
check('and the browser never blinked', transport.exits === 0 && transport.requests === 1);

/* ── 5. Leaving one of several ───────────────────────────────── */

onWorkspaceOf('Alpha');
check('the takeover moved to the workspace on view',
  fullscreenWindow() === 'alpha' && body.classList.contains('fs-active'));

exitWindowFullscreen();
check('alpha is home again', list()[0].windows.includes('alpha'));
check('its dedicated workspace is gone', dedicated('alpha') === null);
check('the view is back on the home workspace', activeWorkspaceIndex() === 0);
check('…with its chrome', !body.classList.contains('fs-active'));
check('the browser keeps fullscreen for the others',
  transport.exits === 0 && isAppFullscreen() === true);
check('the others kept their screens',
  dedicated('beta')?.fullscreen === 'beta' && dedicated('gamma')?.fullscreen === 'gamma');

/* ── 6. Escape ends only the fullscreen of the window on screen ─ */

onWorkspaceOf('Beta');
check('the takeover followed us', fullscreenWindow() === 'beta');
// The browser's own exit (Escape, F11) — not one we asked for.
document.exitFullscreen();
await flushBrowser();
check('escape ends the fullscreen of the window on screen',
  fullscreenWindow() === null && list()[0].windows.includes('beta'));
check('its workspace is dropped', dedicated('beta') === null);
check('the other fullscreen window keeps its screen', dedicated('gamma')?.fullscreen === 'gamma');
check('the browser is out of real fullscreen', isAppFullscreen() === false);

/* ── 7. A window fullscreened while the browser is out ───────── */

onWorkspaceOf('Gamma');
exitWindowFullscreen();
check('leaving the last screen with the browser already out asks for nothing',
  transport.exits === 1, `exits=${transport.exits}`);

toggleWindowFullscreen('gamma');
check('entering again requests real fullscreen', transport.requests === 2, `requests=${transport.requests}`);
await flushBrowser();
check('and the browser grants it', isAppFullscreen() === true);

/* ── 8. The race: a stale exit must not tear down a new window ─ */

exitWindowFullscreen();
check('the last window leaving asks the browser to leave', transport.exits === 2, `exits=${transport.exits}`);

// …and a window enters in the same tick, before that exit lands.
toggleWindowFullscreen('alpha');
check('alpha owns a screen immediately', fullscreenWindow() === 'alpha');
await flushBrowser();
check('the late exit did not tear the new window down', fullscreenWindow() === 'alpha');
check('…which still owns its own workspace',
  dedicated('alpha')?.fullscreen === 'alpha' && dedicated('alpha')?.windows.join() === 'alpha');
check('the effect stands even though the browser ended up out of fullscreen',
  isAppFullscreen() === false && body.classList.contains('fs-active'));

/* ── 9. The way out ──────────────────────────────────────────── */

exitWindowFullscreen();
check('exiting while the browser is out asks for nothing',
  transport.exits === 2, `exits=${transport.exits}`);
check('the desktop is exactly as it started',
  list().length === 1 && [...list()[0].windows].sort().join() === 'alpha,beta,gamma');
check('no workspace is left sealed', !list().some((ws) => ws.fullscreen));

/* ── 10. The chrome contract in CSS ──────────────────────────────
   Two states exist and they are not the same: `body.fs-active` is the workspace
   on view holding a fullscreen window (fullscreen.js), while
   `body.fullscreen-active` is the browser being in real fullscreen — set for the
   whole document, so it stays on while you visit every other workspace. Only the
   first may take the chrome away: keying the grid's inset off the second left the
   windows of every other workspace sitting under their (visible) top bar. */

const css = readFileSync(new URL('../../css/fullscreen.css', import.meta.url), 'utf8')
  .replace(/\/\*[\s\S]*?\*\//g, ''); // comments explain the rule; they are not the rule

check('the workspace state is what gives a window the screen',
  /body\.fs-active #tile-grid\s*\{/.test(css));
check('the browser-wide fullscreen state never pins the grid',
  !/body\.fullscreen-active[^{}]*#tile-grid/.test(css));
check('no layout rule keys off the browser-wide state',
  !/body:not\(\.fullscreen-active\)/.test(css));

/* ── 11. Bar and voice-bar docking ────────────────────────────
   A side dock has to fill the edge (it used to be content-sized, leaving the
   lower column empty), a bottom top bar takes the bottom band, and the voice bar
   can sit in a corner or become the top-left pill. */

check('a side dock fills the edge instead of hugging its content',
  /html\[data-bar-pos='left'\] \.hud-header-main[\s\S]{0,400}?flex:\s*1 1 auto/.test(css));
check('…with the controls pushed to the far end',
  /html\[data-bar-pos='left'\] #hud-top[\s\S]{0,200}?margin-top:\s*auto/.test(css));
check('the bottom bar docks to the lower edge',
  /html\[data-bar-pos='bottom'\] #hud-header\s*\{[^}]*bottom:\s*0/.test(css));
check('a bottom bar clears the desktop',
  /html\[data-bar-pos='bottom'\] body:not\(\.fs-active\) #tile-grid\s*\{[^}]*bottom:/.test(css));

const sphereCss = readFileSync(new URL('../../css/sphere.css', import.meta.url), 'utf8')
  .replace(/\/\*[\s\S]*?\*\//g, '');
check('the bar positions itself — no wrapper element',
  !/#chrome-bottom/.test(sphereCss) && /#sphere-container \{\n  position: fixed;/.test(sphereCss));
check('the voice bar docks in either bottom corner',
  /data-orb-pos='bottom-left'\] #sphere-container/.test(sphereCss)
  && /data-orb-pos='bottom-right'\] #sphere-container/.test(sphereCss));
/* The bar IS the element: one div, its own body, its own shadow. */
const barMarkup = /id="sphere-container"[\s\S]*?<\/div>/.exec(
  readFileSync(new URL('../../index.html', import.meta.url), 'utf8'))?.[0] || '';
check('the bar is one element — the only thing inside it is the step text',
  (barMarkup.match(/<span/g) || []).length === 1 && /<span id="voice-step"/.test(barMarkup));
check('the bar takes its own size from the settings',
  /width: var\(--voice-bar-width\)/.test(sphereCss)
  && /height: var\(--voice-bar-height\)/.test(sphereCss));
check('the corners are the top bar\'s own, not a setting',
  /#sphere-container \{[\s\S]{0,400}?border-radius: var\(--voice-radius/.test(sphereCss)
  && /--voice-radius: 999px/.test(sphereCss)
  && !/data-orb-radius/.test(sphereCss));
check('the bar\'s colour is the bar\'s own body',
  /#sphere-container \{[\s\S]{0,400}?background: var\(--voice-color\)/.test(sphereCss)
  && !/data-orb-border='none'\]\s*#sphere-container\s*\{[^}]*background:/.test(sphereCss));
check('the voice-reactive shadow is the bar\'s own shadow',
  /--voice-glow:[\s\S]{0,400}?--voice-level/.test(sphereCss)
  && /box-shadow: var\(--voice-glow\)/.test(sphereCss)
  && !/#sphere-container::after/.test(sphereCss));
check('the dock is a sibling of the bar',
  /#artifact-dock \{\n  position: fixed;/.test(sphereCss));
check('“all accents” fills the bar itself with the focus palette',
  /html\[data-orb-color='palette'\]\:not\(\[data-orb-style='bar'\]\) #sphere-container \{[\s\S]{0,120}?var\(--focus-bar/.test(sphereCss));
check('the top-bar look swaps the body for the rail\'s plate, glow kept',
  /data-orb-style='bar'\] #sphere-container \{[\s\S]{0,400}?background: var\(--surface\)/.test(sphereCss)
  && !/data-orb-style='bar'\] #sphere-container \{[\s\S]{0,400}?box-shadow:/.test(sphereCss));
check('the palette border reuses the focus-mode ring',
  /data-orb-border='palette'\] #sphere-container::before/.test(sphereCss)
  && /--focus-ring/.test(sphereCss));
check('the step line has a home inside the bar',
  /id="voice-step"/.test(readFileSync(new URL('../../index.html', import.meta.url), 'utf8')));
check('in the top bar the bar rides the rail\'s height and rounding',
  /data-orb-pos='top-left'\] #sphere-container\s*\{[^}]*height: var\(--hud-bar-height\)/.test(sphereCss));
check('the bar follows the top bar down to the bottom edge',
  /data-orb-pos='top-left'\]\[data-bar-pos='bottom'\] #sphere-container\s*\{[^}]*bottom:/.test(sphereCss));
check('the row gives up the bar\'s slot plus its own gap',
  /var\(--voice-slot\) \+ var\(--desktop-gap/.test(sphereCss));

/* ── 12. The position preferences ───────────────────────────── */

const prefs = await import('../preferences.js');
check('the top bar can dock at the bottom', prefs.BAR_POSITIONS.includes('bottom'));
check('the voice bar has four docking spots',
  prefs.ORB_POSITIONS.join() === 'bottom-center,bottom-left,bottom-right,top-left');
check('a bottom voice bar rides the bar\'s row when the top bar owns that edge',
  prefs.effectiveOrbPosition({ bar_position: 'bottom', orb_position: 'bottom-center' }) === 'top-left'
  && prefs.orbIsAttached({ bar_position: 'bottom', orb_position: 'bottom-center' }) === true);
check('…while the top bar is elsewhere the corner is kept',
  prefs.effectiveOrbPosition({ bar_position: 'top', orb_position: 'bottom-left' }) === 'bottom-left'
  && prefs.orbIsAttached({ bar_position: 'top', orb_position: 'bottom-left' }) === false);
check('a top-left voice bar never moves because of a bottom bar',
  prefs.effectiveOrbPosition({ bar_position: 'bottom', orb_position: 'top-left' }) === 'top-left');

localStorage.setItem('desktop.immersive',
  JSON.stringify({ bar_position: 'bottom', orb_position: 'bottom-right' }));
prefs.applyImmersive();
check('the immersion settings are published to CSS',
  documentElement.dataset.barPos === 'bottom' && documentElement.dataset.orbPos === 'top-left'
  && documentElement.dataset.orbAttached === '1');
localStorage.setItem('desktop.immersive',
  JSON.stringify({ bar_position: 'top', orb_position: 'bottom-right' }));
prefs.applyImmersive();
check('moving the top bar back restores the chosen corner',
  documentElement.dataset.orbPos === 'bottom-right'
  && documentElement.dataset.orbAttached === '0');

/* ── 13. Voice-bar appearance ───────────────────────────────── */

/* Riding the top bar's row the bar hides and comes back with the top bar —
   its own autohide switch has nothing to add there. */
const fullscreenCss = readFileSync(new URL('../../css/fullscreen.css', import.meta.url), 'utf8')
  .replace(/\/\*[\s\S]*?\*\//g, '');
check('an attached voice bar hides with the top bar',
  /data-orb-attached='1'\]\[data-autohide-bar='1'\][\s\S]{0,200}?#sphere-container/.test(fullscreenCss)
  && /data-orb-attached='1'\] body\.fs-active[\s\S]{0,200}?#sphere-container/.test(fullscreenCss));
check('…and comes back with it',
  /data-orb-attached='1'\] body\.fs-top[\s\S]{0,200}?#sphere-container/.test(fullscreenCss));
check('a fullscreen window hides the bar at every dock',
  /body\.fs-active #hud-header \{[\s\S]{0,400}?visibility: hidden/.test(fullscreenCss)
  && /data-bar-pos='bottom'\] body\.fs-active #hud-header\s*\{\s*transform: translateY\(calc\(100% \+ 16px\)\)/.test(fullscreenCss)
  && /data-bar-pos='left'\] body\.fs-active #hud-header\s*\{\s*transform: translateX\(calc\(-100% - 16px\)\)/.test(fullscreenCss));
check('…and comes back from that edge',
  /html\[data-bar-pos\] body\.fs-active\.fs-top #hud-header/.test(fullscreenCss));

/* The bar's own body is not a setting: the border choices shape it, so none of
   them may take it away — a borderless bar is a bar without a border. */
check('the border setting never decides whether the bar exists',
  !/data-orb-border='none'\]\s*#sphere-container\s*\{[^}]*background:/.test(sphereCss)
  && !/data-orb-plate/.test(sphereCss));

localStorage.setItem('desktop.immersive', JSON.stringify({
  bar_position: 'top',
  orb_position: 'bottom-center',
  orb_color: 'palette',
  orb_width: 320,
  orb_height: 56,
  orb_border: 'palette',
  orb_status: 'icons',
}));
prefs.applyImmersive();
check('the appearance choices reach CSS',
  documentElement.dataset.orbColor === 'palette'
  && documentElement.dataset.orbBorder === 'palette'
  && documentElement.dataset.orbStatus === 'icons');
check('the bar size lands on the bar element',
  documentElement.style.getPropertyValue('--voice-bar-width') === '320px'
  && documentElement.style.getPropertyValue('--voice-bar-height') === '56px',
  `${documentElement.style.getPropertyValue('--voice-bar-width')} x ${documentElement.style.getPropertyValue('--voice-bar-height')}`);

localStorage.setItem('desktop.immersive', JSON.stringify({
  orb_width: 999, orb_height: 1, orb_color: 'nope', orb_border: 'nope',
}));
const clamped = prefs.getImmersive();
check('out-of-range sizes are clamped and unknown values fall back',
  clamped.orb_width === 400 && clamped.orb_height === 24
  && clamped.orb_color === 'accent' && clamped.orb_border === 'hairline',
  JSON.stringify(clamped));

if (failures) {
  console.error(`\n${failures} failure(s)`);
  process.exit(1);
}
console.log('\nall fullscreen checks passed');
