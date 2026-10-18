import './helpers/i18n.mjs';
import assert from 'node:assert/strict';
import { apiFetch, apiRequest, getToken, setAuth } from '../api.js';

let expired = 0;
globalThis.CustomEvent = class { constructor(type) { this.type = type; } };
window.dispatchEvent = (event) => { if (event.type === 'auth:expired') expired++; };
const response = (status, body = {}) => new Response(JSON.stringify(body), {
  status, headers: { 'Content-Type': 'application/json' },
});
const signIn = () => setAuth('bearer', { id: 'one' });

signIn();
globalThis.fetch = async (path) => {
  if (path === '/api/travelers/me') throw new TypeError('offline');
  return response(401);
};
await assert.rejects(apiFetch('/api/preferences'), { status: 401 });
assert.equal(getToken(), 'bearer');
assert.equal(expired, 0, 'network failure must not expire a login');

for (const status of [500, 503]) {
  globalThis.fetch = async (path) => response(path === '/api/travelers/me' ? status : 401);
  await assert.rejects(apiFetch('/api/preferences'), { status: 401 });
  assert.equal(getToken(), 'bearer');
}

signIn();
let attempts = 0;
globalThis.fetch = async (path, options) => {
  if (path === '/api/travelers/me') return response(200, { data: { id: 'one' } });
  attempts++;
  if (options.headers.has('Authorization')) return response(401);
  return response(200, { success: true, data: ['preserved'] });
};
assert.deepEqual(await apiFetch('/api/preferences'), { success: true, data: ['preserved'] });
assert.equal(attempts, 2);
assert.equal(getToken(), null, 'stale bearer is discarded after cookie validation');

signIn();
let checks = 0;
let completeCheck;
globalThis.fetch = async (path) => {
  if (path === '/api/travelers/me') {
    checks++;
    await new Promise((resolve) => { completeCheck = resolve; });
    return response(401);
  }
  return response(401);
};
const requests = [apiFetch('/api/one'), apiRequest('/api/events')];
await new Promise((resolve) => setImmediate(resolve));
assert.equal(checks, 1, 'concurrent JSON and stream failures share a session probe');
completeCheck();
await Promise.allSettled(requests);
assert.equal(expired, 1);
assert.equal(getToken(), null);

signIn();
globalThis.fetch = async () => new Response('<!doctype html><title>Shell fallback</title>', { status: 200 });
await assert.rejects(apiFetch('/api/missing'), { name: 'ApiError', status: 502 });
assert.equal(getToken(), 'bearer');
globalThis.fetch = async () => new Response('broken JSON', { status: 500 });
await assert.rejects(apiFetch('/api/failure'), { status: 500 });

signIn();
globalThis.fetch = async (path) => {
  if (path === '/api/travelers/me') {
    setAuth('new-login', { id: 'two' });
    return response(401);
  }
  return response(401);
};
await assert.rejects(apiFetch('/api/preferences'), { status: 401 });
assert.equal(getToken(), 'new-login', 'old request cannot clear a newer login');
console.log('API recovery checks passed');
