/** Native page views outlive a reload of the app; the next document closes them. */
import assert from 'node:assert/strict';

const store = new Map();
globalThis.sessionStorage = {
  getItem: (k) => (store.has(k) ? store.get(k) : null),
  setItem: (k, v) => store.set(k, String(v)),
  removeItem: (k) => store.delete(k),
};
const posted = [];
globalThis.window = { ipc: { postMessage: (m) => posted.push(m) } };

const { rememberNativeView, forgetNativeView, closeOrphanNativeViews } = await import('../nativeViews.js');

assert.equal(closeOrphanNativeViews(), 0, 'nothing open, nothing sent');
assert.equal(posted.length, 0);

rememberNativeView('t1');
rememberNativeView('t2');
rememberNativeView('t2');
rememberNativeView('t3');
forgetNativeView('t3');

// The app reloads: a new document starts and closes what the old one left.
assert.equal(closeOrphanNativeViews(), 2);
assert.deepEqual(posted, [
  'liteaura:view:{"op":"close","id":"t1"}',
  'liteaura:view:{"op":"close","id":"t2"}',
]);
assert.equal(closeOrphanNativeViews(), 0, 'closed once');

console.log('native view checks passed');
