import './helpers/i18n.mjs';
import assert from 'node:assert/strict';

// The harness has no sessionStorage, timers are real: keep the poll out of
// the way by driving `check()` by hand.
const session = new Map();
Object.defineProperty(globalThis, 'sessionStorage', {
  value: {
    getItem: (k) => (session.has(k) ? session.get(k) : null),
    setItem: (k, v) => session.set(k, String(v)),
    removeItem: (k) => session.delete(k),
  },
  configurable: true,
});
globalThis.CustomEvent = class { constructor(type, init) { this.type = type; this.detail = init?.detail; } };
const events = [];
const listeners = new Map();
window.addEventListener = (type, fn) => listeners.set(type, fn);
window.removeEventListener = () => {};
window.dispatchEvent = (event) => { events.push(event); listeners.get(event.type)?.(event); };
window.location = { reload: () => { throw new Error('the real reload must not run in tests'); } };
window.matchMedia = () => ({ matches: false, addEventListener() {}, removeEventListener() {} });

const { setAuth } = await import('../api.js');
// Import preferences without applying a desktop surface (no DOM here).
delete globalThis.document;
const prefs = await import('../preferences.js');
globalThis.document = { hidden: false };
const { initLiveUpdate, check, knownVersion } = await import('../liveUpdate.js');
setAuth('bearer', { id: 'one' });

let version = { build: 'b1', web: 'w1', plugins: { word: 'p1', mail: 'm1' } };
let fail = null;
globalThis.fetch = async (path) => {
  if (path !== '/api/system/version') throw new Error(`unexpected fetch ${path}`);
  if (fail) throw fail;
  return new Response(JSON.stringify({ success: true, data: version }), {
    status: 200, headers: { 'Content-Type': 'application/json' },
  });
};

const toasts = [];
let reloads = 0;
initLiveUpdate({
  toast: async (message) => { toasts.push(message); return { remove() { toasts.push('(dismissed)'); } }; },
  reload: () => { reloads++; },
  snapshot: async () => ({ workspaces: [{ id: 'ws1', windows: ['word'] }], active: 0 }),
});

// The first answer is only remembered.
assert.deepEqual(await check(), { first: true });
assert.equal(knownVersion().web, 'w1');
assert.equal(events.length, 0);

// Nothing changed: nothing happens.
assert.deepEqual(await check(), { updated: [] });
assert.equal(events.length, 0);

// A plugin's fingerprint moved: the catalog is invalidated and the desktop told.
version = { ...version, plugins: { word: 'p2', mail: 'm1' } };
assert.deepEqual(await check(), { updated: ['word'] });
const changed = events.find((e) => e.type === 'plugins:changed');
assert.deepEqual(changed.detail, { updated: ['word'] });
assert.ok(toasts.some((m) => m.includes('word')), 'the user is told which plugin changed');

// A new build alone (the server swapped) changes nothing on the page.
version = { ...version, build: 'b2' };
assert.deepEqual(await check(), { updated: [] });
assert.equal(reloads, 0);

// The server goes away: one notice, then it comes back and streams are told.
events.length = 0;
fail = new TypeError('Failed to fetch');
assert.equal(await check(), null);
const notices = toasts.filter((m) => m === 'Reconnecting to the server…').length;
assert.equal(notices, 1);
assert.equal(await check(), null);
assert.equal(toasts.filter((m) => m === 'Reconnecting to the server…').length, 1, 'one notice per outage');
fail = null;
assert.deepEqual(await check(), { updated: [] });
assert.ok(events.some((e) => e.type === 'server:reconnected'));
assert.ok(toasts.includes('(dismissed)'), 'the notice goes away when the server is back');

// The core web tree changed: the desktop is stashed and the page reloads once.
prefs.setRemember(false);
version = { ...version, web: 'w2' };
assert.deepEqual(await check(), { reload: true });
assert.equal(reloads, 1);
assert.ok(prefs.isUpdateReload());
assert.deepEqual(prefs.getWorkspaces(), [{ id: 'ws1', windows: ['word'] }], 'fresh mode restores the stashed desktop once');
assert.equal(prefs.getActiveWorkspaceId(), 'ws1');
prefs.finishUpdateReload();
assert.equal(prefs.getWorkspaces(), null, 'after the reload fresh mode is fresh again');
assert.equal(await check(), null, 'nothing runs after the reload was asked');
assert.equal(reloads, 1);

console.log('liveUpdate: all checks passed');
