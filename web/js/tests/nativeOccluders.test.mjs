/**
 * Native-page occluder test.
 *
 * The Browser renders pages in a native scene item above the app webview, so
 * these rects are the only way core HTML can stay visible over a page. This
 * covers the core registry: the chat panel (the reported regression), viewport
 * clipping and the geometry the shell expects.
 *
 * Run:  node web/js/tests/nativeOccluders.test.mjs
 */

import {
  NATIVE_OCCLUDER_SELECTOR,
  nativeOccluderHoles,
  watchNativeOccluders,
} from '../nativeOccluders.js';

let failures = 0;
function check(name, condition, detail = '') {
  if (condition) {
    console.log(`  ok  ${name}`);
  } else {
    failures += 1;
    console.error(`FAIL  ${name}${detail ? ` — ${detail}` : ''}`);
  }
}

check(
  'the registry includes the chat panel, not its transparent wrapper',
  NATIVE_OCCLUDER_SELECTOR.includes('#compose-mode:not(.hidden) .compose-conversation'),
);

const viewport = { x: 100, y: 100, w: 400, h: 300, dpr: 2 };
const chat = {
  getBoundingClientRect: () => ({
    left: 120, top: 140, width: 100, height: 80, right: 220, bottom: 220,
  }),
};
const offscreen = {
  getBoundingClientRect: () => ({
    left: 600, top: 140, width: 50, height: 50, right: 650, bottom: 190,
  }),
};
const zeroSize = {
  getBoundingClientRect: () => ({
    left: 120, top: 140, width: 0, height: 80, right: 120, bottom: 220,
  }),
};

let queried = null;
globalThis.document = {
  querySelectorAll: (selector) => {
    queried = selector;
    return [chat, offscreen, zeroSize];
  },
};

const holes = nativeOccluderHoles(viewport);
check('the core selector is what is queried', queried === NATIVE_OCCLUDER_SELECTOR);
check('only the overlapping overlay becomes a hole', holes.length === 1, JSON.stringify(holes));
check(
  'the chat hole keeps client coordinates and the viewport dpr',
  JSON.stringify(holes[0]) === JSON.stringify({
    x: 120, y: 140, w: 100, h: 80, dpr: 2,
  }),
  JSON.stringify(holes[0]),
);
check('no viewport means no work', nativeOccluderHoles(null).length === 0);

/* The watcher observes overlay roots and body children, then disposes. */
const body = {};
const root = {};
let observerInstance = null;
class FakeMutationObserver {
  constructor(callback) {
    this.callback = callback;
    observerInstance = this;
  }
  observe(target, options) {
    if (target === body) this.bodyOptions = options;
    else (this.rootOptions ||= []).push(options);
  }
  disconnect() { this.disposed = true; }
  trigger() { this.callback([]); }
}

globalThis.MutationObserver = FakeMutationObserver;
globalThis.document = { body, querySelectorAll: () => [root] };

let changes = 0;
const stopWatching = watchNativeOccluders(() => { changes += 1; });
check('the watcher observes the static overlay roots', (observerInstance?.rootOptions || []).length === 1);
check('the watcher observes body children', observerInstance?.bodyOptions?.childList === true);
observerInstance.trigger();
check('a watcher change notifies the caller', changes === 1, String(changes));
stopWatching();
check('the watcher disposes', observerInstance?.disposed === true);

delete globalThis.document;
delete globalThis.MutationObserver;

if (failures) {
  console.error(`\n${failures} native occluder check(s) failed`);
  process.exit(1);
}
console.log('\nall native occluder checks passed');
