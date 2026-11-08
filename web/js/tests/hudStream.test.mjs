/**
 * HUD stream test.
 *
 * The top-bar chips listen to their topic on the shell's single event stream
 * (`eventBus.js`). This covers what a chip relies on: all four chips share one
 * connection, each gets only its own topic's snapshots (split frames, several
 * events per chunk, keep-alive comments), the Authorization header is sent,
 * and the status poll stops once the stream is up (the drop and reconnect
 * path is covered by eventBus.test.mjs).
 *
 * Run:  node web/js/tests/hudStream.test.mjs
 */

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
  'event: audio\ndata: {"percent":42}\n\nevent: batt',
  'ery\ndata: {"percent":90}\n\n',
  'event: audio\ndata: {"percent":43}\n\nevent: audio\ndata: {"percent":44}\n\n',
];
let reads = 0;
const reader = {
  read: () => (reads < frames.length
    ? Promise.resolve({ done: false, value: encoder.encode(frames[reads++]) })
    : new Promise(() => {})),
};

const eventCalls = [];
const polls = [];
globalThis.fetch = async (url, init = {}) => {
  if (String(url).includes('/api/events?')) {
    eventCalls.push({ url: String(url), headers: init.headers });
    return { ok: true, status: 200, body: { getReader: () => reader } };
  }
  polls.push(String(url));
  return new Response('{"success":true,"data":{"percent":1}}', {
    status: 200,
    headers: { 'Content-Type': 'application/json' },
  });
};

const { createHudStream } = await import('../hudStream.js');

const received = { audio: [], battery: [], network: [], bluetooth: [] };
const chips = Object.keys(received).map((topic) => createHudStream({
  topic,
  statusUrl: `/api/${topic}/status`,
  pollMs: 15000,
  label: topic,
  apply: (status) => received[topic].push(status),
}));
for (const chip of chips) chip.start();
await new Promise((resolve) => setTimeout(resolve, 80));

const topics = new URL(eventCalls[0]?.url ?? 'http://x/').searchParams.get('topics');
check('four chips open one connection', eventCalls.length === 1, `${eventCalls.length}`);
check('the connection carries every chip topic',
  topics === 'audio,battery,bluetooth,network', topics);
check('sends the auth header', eventCalls[0]?.headers?.Authorization === 'Bearer test-token');

// Each chip polled once while the stream was still opening (a fast first
// paint); the stream's own snapshots follow.
const audio = received.audio.filter((s) => s.percent !== 1).map((s) => s.percent).join(',');
check('parses audio frames in order, across chunks', audio === '42,43,44', audio);
check('routes other topics to their own chip',
  received.battery.some((s) => s.percent === 90)
  && !received.network.some((s) => s.percent !== 1), JSON.stringify(received));
const pollsAtStart = polls.length;
await new Promise((resolve) => setTimeout(resolve, 50));
check('no extra poll while the stream is up', polls.length === pollsAtStart,
  `${polls.length} vs ${pollsAtStart}`);

for (const chip of chips) chip.stop();

if (failures) {
  console.error(`\n${failures} HUD stream check(s) failed`);
  process.exit(1);
}
console.log('\nall HUD stream checks passed');
process.exit(0);
