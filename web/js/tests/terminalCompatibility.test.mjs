// A hot-installed plugin must work with an older core API module already held
// in the page's module cache. Exercise the plugin's real transport adapter.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { Script } from 'node:vm';
const source = readFileSync(new URL('../../../plugins/terminal/web/plugin.js', import.meta.url), 'utf8');
const start = source.indexOf('function requestStream(');
const end = source.indexOf('/**\n * Read the window', start);
assert.ok(start >= 0 && end > start);
const coreApi = {};
const calls = [];
let token = 'legacy-token';
const request = new Script(`${source.slice(start, end)}; requestStream`).runInNewContext({
  coreApi, getToken: () => token,
  fetch: async (path, options) => { calls.push({ path, options }); return 'legacy response'; },
});
const signal = new AbortController().signal;
assert.equal(await request('/api/terminal/events', { signal, cache: 'no-store' }), 'legacy response');
assert.equal(calls[0].options.headers.Authorization, 'Bearer legacy-token');
assert.equal(calls[0].options.signal, signal);
token = null;
await request('/api/terminal/events', {});
assert.equal(Object.keys(calls[1].options.headers).length, 0);
coreApi.apiRequest = async (path, options) => {
  assert.equal(path, '/api/terminal/events');
  assert.equal(options.signal, signal);
  return 'shared recovery';
};
assert.equal(await request('/api/terminal/events', { signal }), 'shared recovery');
assert.equal(calls.length, 2, 'new cores use shared auth recovery rather than the legacy fetch path');
console.log('Terminal hot-install compatibility checks passed');
