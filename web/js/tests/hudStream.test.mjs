/**
 * HUD stream test.
 *
 * The top-bar chips share one SSE reader. This covers the part that used to be
 * copied four times: split frames, multiple events per chunk, keep-alive
 * comments without data, and the Authorization header. The stream is left
 * pending (the helper reconnects forever by design), so the process exits
 * without waiting on it.
 *
 * Run:  node web/js/tests/hudStream.test.mjs
 */

import { createHudStream } from '../hudStream.js';

let failures = 0;
function check(name, condition, detail = '') {
  if (condition) {
    console.log(`  ok  ${name}`);
  } else {
    failures += 1;
    console.error(`FAIL  ${name}${detail ? ` — ${detail}` : ''}`);
  }
}

globalThis.localStorage = { getItem: () => 'test-token' };

const encoder = new TextEncoder();
const frames = [
  ': keep-alive\n\n',
  'data: {"percent":42}\n\n',
  'data: {"percent":43}\n\ndata: {"percent":44}\n\n',
];
let reads = 0;
const reader = {
  read: () => (reads < frames.length
    ? Promise.resolve({ done: false, value: encoder.encode(frames[reads++]) })
    : new Promise(() => {})),
};

let calledUrl = '';
let calledHeaders = null;
globalThis.fetch = async (url, init) => {
  calledUrl = url;
  calledHeaders = init.headers;
  return { ok: true, status: 200, body: { getReader: () => reader } };
};

const received = [];
const stream = createHudStream({
  eventsUrl: '/api/audio/events',
  statusUrl: '/api/audio/status',
  pollMs: 15000,
  label: 'audio',
  apply: (status) => received.push(status),
});

stream.start();
await new Promise((resolve) => setTimeout(resolve, 20));

check('subscribes to the events URL', calledUrl === '/api/audio/events');
check('sends the auth header', calledHeaders?.Authorization === 'Bearer test-token');
check('parses one event per frame', received.length === 3, JSON.stringify(received));
check('parses frames in order', received.map((s) => s.percent).join(',') === '42,43,44');
check('ignores keep-alive frames', received.every((s) => typeof s.percent === 'number'));

if (failures) {
  console.error(`\n${failures} HUD stream check(s) failed`);
  process.exit(1);
}
console.log('\nall HUD stream checks passed');
