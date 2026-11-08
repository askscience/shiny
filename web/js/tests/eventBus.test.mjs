/**
 * Event bus test.
 *
 * Every live stream the shell needs rides one `/api/events` connection
 * (Chromium allows six per origin; one per stream filled the pool at login).
 * Covers: subscriptions made together share one connection, a topic added
 * later moves to a new connection that replaces the old one only once it
 * answers, the last unsubscribe closes it, frames go to their topic only,
 * and a dropped stream reports down and reconnects.
 *
 * Run:  node web/js/tests/eventBus.test.mjs
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

globalThis.localStorage = { getItem: () => null };

const encoder = new TextEncoder();
const streams = [];
globalThis.fetch = async (url, init = {}) => {
  const queue = [];
  let wake = null;
  const stream = {
    topics: new URL(String(url)).searchParams.get('topics'),
    aborted: false,
    ended: false,
    push(text) { queue.push(encoder.encode(text)); wake?.(); },
    end() { stream.ended = true; wake?.(); },
  };
  init.signal?.addEventListener('abort', () => { stream.aborted = true; wake?.(); });
  streams.push(stream);
  const reader = {
    async read() {
      for (;;) {
        if (stream.aborted) throw new DOMException('aborted', 'AbortError');
        if (queue.length) return { done: false, value: queue.shift() };
        if (stream.ended) return { done: true };
        await new Promise((resolve) => { wake = resolve; });
        wake = null;
      }
    },
  };
  return { ok: true, status: 200, body: { getReader: () => reader } };
};

const { subscribe, currentTopics } = await import('../eventBus.js');
const wait = (ms = 60) => new Promise((resolve) => setTimeout(resolve, ms));
const open = () => streams.filter((s) => !s.aborted && !s.ended);

const got = { a: [], b: [], c: [] };
const states = [];
const offA = subscribe('a', (data) => got.a.push(data), (up) => states.push(up));
const offB = subscribe('b', (data) => got.b.push(data));
await wait();
check('subscriptions in one tick share one connection',
  streams.length === 1 && streams[0].topics === 'a,b', `${streams.length} ${streams[0]?.topics}`);
check('a subscriber hears the stream come up', states.at(-1) === true, JSON.stringify(states));

streams[0].push('event: a\ndata: 1\n\nevent: b\ndata: 2\n\n: keep-alive\n\nevent: a\ndata: x\ndata: y\n\n');
await wait(10);
check('frames reach their topic only', got.a.join('|') === '1|x\ny' && got.b.join('|') === '2',
  JSON.stringify(got));

const offC = subscribe('c', (data) => got.c.push(data));
await wait();
check('a new topic opens a new connection', streams.length === 2 && streams[1].topics === 'a,b,c',
  streams.map((s) => s.topics).join(' / '));
check('the old connection goes once the new one answers', streams[0].aborted && open().length === 1);
streams[1].push('event: c\ndata: 3\n\n');
await wait(10);
check('the new connection delivers', got.c.join() === '3');

offC();
offB();
await wait();
check('unsubscribing narrows the topics', currentTopics() === 'a', currentTopics());

// The server drops the stream: down, then a reconnect after the delay.
open()[0].end();
await wait(10);
check('a drop reports down', states.at(-1) === false, JSON.stringify(states));
const before = streams.length;
await wait(5200);
check('it reconnects by itself', streams.length === before + 1 && open().length === 1,
  `${streams.length} vs ${before}`);
check('and reports up again', states.at(-1) === true, JSON.stringify(states));

offA();
await wait();
check('the last unsubscribe closes the connection', open().length === 0 && currentTopics() === '');

if (failures) {
  console.error(`\n${failures} event bus check(s) failed`);
  process.exit(1);
}
console.log('\nall event bus checks passed');
