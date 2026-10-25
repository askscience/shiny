/**
 * Craft store connectivity regression.
 *
 * A live ISO can boot before its network is up. The catalog must say "waiting"
 * and retry by itself — not cache a GitHub failure or show a raw error — and it
 * must load as soon as a retry succeeds.
 */
import assert from 'node:assert/strict';

function stub(name, value) {
  Object.defineProperty(globalThis, name, { value, configurable: true, writable: true });
}

const store = new Map();
stub('localStorage', {
  getItem: (k) => (store.has(k) ? store.get(k) : null),
  setItem: (k, v) => store.set(k, String(v)),
  removeItem: (k) => store.delete(k),
});
stub('navigator', { language: 'en' });
stub('CustomEvent', class { constructor(type, init = {}) { this.type = type; this.detail = init.detail; } });
stub('window', {
  addEventListener() {},
  removeEventListener() {},
  dispatchEvent() {},
  matchMedia: () => ({ matches: false, addEventListener() {}, removeEventListener() {} }),
  innerWidth: 1280,
  innerHeight: 800,
});

function elementStub() {
  return {
    className: '', textContent: '', title: '', dataset: {}, style: { setProperty() {} },
    classList: { add() {}, remove() {}, toggle() {} },
    setAttribute() {}, appendChild() {}, addEventListener() {}, removeEventListener() {},
    querySelectorAll: () => [],
  };
}
stub('document', {
  documentElement: {
    lang: '', dir: '', dataset: {}, style: { setProperty() {} }, setAttribute() {},
  },
  body: { appendChild() {} },
  querySelectorAll: () => [],
  createElement: elementStub,
  addEventListener() {},
  removeEventListener() {},
});

const json = (body, status = 200) => new Response(JSON.stringify(body), {
  status, headers: { 'Content-Type': 'application/json' },
});
const catalog = (waiting) => json({
  success: true,
  data: {
    refreshing: false,
    waiting_for_network: waiting,
    apps: [{
      id: 'photocraft',
      name: 'PhotoCraft',
      waiting_for_network: waiting,
      latest_version: waiting ? null : '0.10.0',
      installed_version: null,
    }],
  },
});

// Import-time icon loader asks for its index; nothing else should fetch during
// import. Any unknown request is answered with an empty 404.
let handler = async () => json({}, 404);
stub('fetch', async (url, options) => handler(String(url), options));

const {
  loadStore, retryStoreNow, storeSnapshot, storeWaitingForNetwork, storeLoadFailed,
} = await import('../craftStore.js');

let storeAttempts = 0;
handler = async (url) => {
  if (url === '/api/plugins/store') {
    storeAttempts += 1;
    throw new TypeError('offline');
  }
  return json({}, 404);
};

await assert.rejects(loadStore(), /offline/);
assert.equal(storeSnapshot(), null);
assert.equal(storeWaitingForNetwork(), true, 'a failed catalog read waits for the network');
assert.equal(storeAttempts, 1);

// The core is reachable but GitHub is not: the catalog is a valid answer with
// `waiting_for_network`, not an error.
handler = async (url) => {
  if (url === '/api/plugins/store/refresh') return json({ success: true });
  if (url === '/api/plugins/store') return catalog(true);
  return json({}, 404);
};
await retryStoreNow();
assert.equal(storeWaitingForNetwork(), true);
assert.equal(storeSnapshot()?.apps?.[0]?.waiting_for_network, true);
assert.equal(storeSnapshot()?.apps?.[0]?.latest_version, null);

// The next successful read clears the waiting state and exposes the release.
handler = async (url) => (url === '/api/plugins/store' ? catalog(false) : json({}, 404));
await loadStore();
assert.equal(storeWaitingForNetwork(), false);
assert.equal(storeSnapshot()?.apps?.[0]?.latest_version, '0.10.0');

// A permanent client error is not retried as connectivity trouble.
handler = async () => json({ error: 'Unauthorized' }, 401);
await assert.rejects(loadStore(), { status: 401 });
assert.equal(storeWaitingForNetwork(), false, 'a 401 is not a network wait');
assert.equal(storeLoadFailed(), true, 'a permanent failure is still reported');

// The retry is scheduled by the failed read itself: simulate the timer firing
// once the network is back, without waiting the real backoff.
const realSetTimeout = globalThis.setTimeout;
let retry = null;
globalThis.setTimeout = (fn) => {
  retry = fn;
  return { unref() {} };
};
handler = async () => { throw new TypeError('offline'); };
await assert.rejects(loadStore(), /offline/);
assert.equal(typeof retry, 'function', 'a failed read schedules its own retry');
handler = async (url) => (url === '/api/plugins/store' ? catalog(false) : json({}, 404));
retry();
await new Promise((resolve) => realSetTimeout(resolve, 10));
globalThis.setTimeout = realSetTimeout;
assert.equal(storeWaitingForNetwork(), false);
assert.equal(storeLoadFailed(), false);
assert.equal(storeSnapshot()?.apps?.[0]?.latest_version, '0.10.0');

console.log('craft store waiting/retry checks passed');
