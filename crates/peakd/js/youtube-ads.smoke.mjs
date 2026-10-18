/**
 * YouTube scriptlet smoke test.
 *
 * There is no browser in this environment, so this exercises the real injected
 * script (`crates/peakd/js/youtube-ads.js`) against a small DOM shim: it must
 * no-op on every other host, rewrite the player responses, prune the inlined
 * player response, get a running ad out of the way, and never let a throwing
 * page surface an error. It is not a substitute for a browser; it is a
 * substitute for *no* test — the seams a Rust test cannot reach.
 *
 * Run:  node crates/peakd/js/youtube-ads.smoke.mjs
 */
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import vm from 'node:vm';

const here = dirname(fileURLToPath(import.meta.url));
const source = readFileSync(join(here, 'youtube-ads.js'), 'utf8');

const failures = [];
function check(label, condition, detail) {
  if (condition) {
    console.log(`  ok   ${label}`);
  } else {
    failures.push(label);
    console.log(`  FAIL ${label}${detail === undefined ? '' : ` — ${detail}`}`);
  }
}

/* ── The DOM shim ─────────────────────────────────────────────── */

function makeElement(tag) {
  return {
    tagName: String(tag).toUpperCase(),
    children: [],
    textContent: '',
    clicked: false,
    appendChild(child) { this.children.push(child); return child; },
    click() { this.clicked = true; },
  };
}

/** One page under test: a fresh global with the host it should load from. */
function makePage(hostname, { brokenHead = false, unparsed = false } = {}) {
  const elements = new Map();   // selector -> element the script asks for
  const listeners = new Map();  // event name -> handlers
  const observers = [];         // live MutationObservers, for driving callbacks
  const timers = new Map();     // interval id -> callback
  const state = { body: '', fetched: [], appendedToBareDocument: false };
  let nextTimer = 1;

  const head = makeElement('head');
  if (brokenHead) head.appendChild = () => { throw new Error('no head for you'); };
  const documentElement = makeElement('html');

  const doc = {
    // `unparsed` is the document a DocumentCreation script actually sees: no
    // root element, no head. `parse()` is the parser arriving with them.
    documentElement: unparsed ? null : documentElement,
    head: unparsed ? null : head,
    createElement: (tag) => makeElement(tag),
    querySelector: (selector) => elements.get(selector) || null,
    addEventListener: (type, fn) => {
      if (!listeners.has(type)) listeners.set(type, []);
      listeners.get(type).push(fn);
    },
    emit(type) { for (const fn of listeners.get(type) || []) fn({ type }); },
    // The document node itself. Appending an element here at creation is what
    // blanked youtube.com in the real engine: the parser never builds
    // html/head/body afterwards. The script must never do it.
    appendChild(child) { state.appendedToBareDocument = true; this.kids.push(child); return child; },
    kids: [],
    parse() {
      doc.documentElement = documentElement;
      doc.head = head;
    },
  };

  const window = {
    document: doc,
    location: { hostname },
    ytInitialPlayerResponse: {
      adPlacements: [{ ad: true }],
      adSlots: [{ slot: true }],
      playerAds: [{ ad: true }],
      videoDetails: { videoId: 'abc' },
    },
    addEventListener: (type, fn) => {
      const key = `window:${type}`;
      if (!listeners.has(key)) listeners.set(key, []);
      listeners.get(key).push(fn);
    },
    emit(type) { for (const fn of listeners.get(`window:${type}`) || []) fn({ type }); },
  };

  class FakeXHR {
    constructor() {
      this.readyState = 0;
      this.responseType = '';
      this._text = '';
      this._handlers = new Map();
    }
    open(method, url) { this._url = String(url); this.readyState = 1; }
    send() {}
    addEventListener(type, fn) { this._handlers.set(type, fn); }
    /** Deliver a response the way the engine would: state 4, then the handler. */
    respond(text) {
      this.readyState = 4;
      this._text = text;
      const handler = this._handlers.get('readystatechange');
      if (handler) handler.call(this);
    }
  }
  Object.defineProperty(FakeXHR.prototype, 'responseText', {
    get() { return this._text; },
    configurable: true,
  });

  class FakeObserver {
    constructor(callback) { this.callback = callback; this.target = null; }
    observe(target) { this.target = target; observers.push(this); }
    disconnect() { this.target = null; }
  }

  const nativeFetch = (input) => {
    state.fetched.push(typeof input === 'string' ? input : (input && input.url) || '');
    const response = {
      status: 200,
      statusText: 'OK',
      // What the site sent, so a pass-through can be told from a rewrite.
      body: state.body,
      headers: { 'content-type': 'application/json' },
      clone() { return { text: () => Promise.resolve(state.body) }; },
    };
    return Promise.resolve(response);
  };
  window.fetch = nativeFetch;

  const sandbox = {
    window,
    document: doc,
    location: window.location,
    XMLHttpRequest: FakeXHR,
    MutationObserver: FakeObserver,
    Response: class Response {
      constructor(body, init) { this.body = body; this.status = init && init.status; }
    },
    setInterval: (fn) => { const id = nextTimer++; timers.set(id, fn); return id; },
    clearInterval: (id) => timers.delete(id),
    console,
  };
  const context = vm.createContext(sandbox);
  vm.runInContext(source, context, { filename: 'youtube-ads.js' });

  return {
    window,
    doc,
    elements,
    observers,
    timers,
    state,
    nativeFetch,
    FakeXHR,
    /** Run every live interval callback, `times` times, like time passing. */
    runTimers(times) {
      for (let i = 0; i < times; i++) for (const fn of [...timers.values()]) fn();
    },
  };
}

/* ── Another site: the script must be a no-op ─────────────────── */

const other = makePage('news.example.com');
check('does not arm itself on other hosts', other.window.__peakdYoutubeAds === undefined);
check('leaves fetch alone on other hosts', other.window.fetch === other.nativeFetch);
check('injects nothing on other hosts', other.doc.head.children.length === 0);
check('observes nothing on other hosts', other.observers.length === 0);
check('starts no timers on other hosts', other.timers.size === 0);
other.doc.emit('DOMContentLoaded');
check('a DOMContentLoaded elsewhere stays harmless', other.window.fetch === other.nativeFetch);

/* ── youtube.com: the whole surface ──────────────────────────── */

const page = makePage('www.youtube.com', { unparsed: true });
check('arms itself on youtube.com', page.window.__peakdYoutubeAds === true);
// At document creation the parser has no root yet. Touching the document node
// itself there is what left youtube.com as a lone <style> — a blank page.
check(
  'never appends to the bare document at creation',
  page.state.appendedToBareDocument === false,
  String(page.state.appendedToBareDocument),
);
check(
  'leaves the document alone until the parser has a root',
  page.doc.head === null && page.doc.documentElement === null,
);

/* Player responses through fetch. */
page.state.body = '{"adPlacements":[{"ad":1}],"videoDetails":{"videoId":"abc"}}';
const playerResponse = await page.window.fetch('https://www.youtube.com/youtubei/v1/player?key=x');
page.state.body = '{"adPlacements":[{"ad":1}]}';
const otherResponse = await page.window.fetch('https://www.youtube.com/api/stats/watchtime');
check(
  'fetch: the player response loses its ad keys',
  playerResponse.body === '{"no_ads":[{"ad":1}],"videoDetails":{"videoId":"abc"}}',
  playerResponse.body,
);
check(
  'fetch: any other response is passed through untouched',
  otherResponse.body === '{"adPlacements":[{"ad":1}]}',
  otherResponse.body,
);

/* Player responses through XHR, both response types. */
const textXhr = new page.FakeXHR();
textXhr.open('POST', 'https://www.youtube.com/youtubei/v1/get_watch?key=x');
textXhr.send();
textXhr.respond('{"adSlots":[{"slot":1}],"playerAds":[]}');
check(
  'xhr: the player response loses its ad keys',
  textXhr.responseText === '{"no_ads":[{"slot":1}],"no_ads":[]}',
  textXhr.responseText,
);

const otherXhr = new page.FakeXHR();
otherXhr.open('GET', 'https://www.youtube.com/api/stats/qoe');
otherXhr.send();
otherXhr.respond('{"adPlacements":[{"ad":1}]}');
check(
  'xhr: any other response is passed through untouched',
  otherXhr.responseText === '{"adPlacements":[{"ad":1}]}',
  otherXhr.responseText,
);

const jsonXhr = new page.FakeXHR();
jsonXhr.open('POST', 'https://www.youtube.com/youtubei/v1/player?key=x');
jsonXhr.send();
jsonXhr.responseType = 'json';
Object.defineProperty(jsonXhr, 'response', {
  get() { return { adPlacements: [1], videoDetails: { videoId: 'abc' } }; },
  configurable: true,
});
jsonXhr.respond('');
check(
  'xhr: a json responseType is rewritten as an object',
  jsonXhr.response
    && jsonXhr.response.no_ads !== undefined
    && jsonXhr.response.adPlacements === undefined
    && jsonXhr.response.videoDetails.videoId === 'abc',
  JSON.stringify(jsonXhr.response),
);
check(
  'xhr: a json responseType is not double-rewritten',
  (() => {
    const xhr = new page.FakeXHR();
    xhr.open('GET', 'https://www.youtube.com/api/stats/qoe');
    xhr.send();
    xhr.responseType = 'json';
    Object.defineProperty(xhr, 'response', {
      get() { return { adPlacements: [1] }; },
      configurable: true,
    });
    xhr.respond('');
    // A response that is not the player's keeps its keys, even the ad ones.
    return xhr.response.adPlacements !== undefined;
  })(),
);
check(
  'xhr: nested ad keys inside a player response are rewritten too',
  (() => {
    const xhr = new page.FakeXHR();
    xhr.open('GET', 'https://www.youtube.com/playlist?list=PL1');
    xhr.send();
    xhr.responseType = 'json';
    Object.defineProperty(xhr, 'response', {
      get() { return { contents: [{ playerResponse: { adPlacements: [1], videoDetails: {} } }] }; },
      configurable: true,
    });
    xhr.respond('');
    const nested = xhr.response.contents[0].playerResponse;
    return nested.no_ads !== undefined && nested.adPlacements === undefined;
  })(),
);

/* The inlined response, at document creation and again on the SPA event. */
check(
  'the inlined player response is pruned immediately',
  page.window.ytInitialPlayerResponse.adPlacements === undefined
    && page.window.ytInitialPlayerResponse.playerAds === undefined
    && page.window.ytInitialPlayerResponse.videoDetails.videoId === 'abc',
  JSON.stringify(page.window.ytInitialPlayerResponse),
);
page.window.ytInitialPlayerResponse.adSlots = [{ slot: true }];
// The parser builds html/head/body, then the load events fire.
page.doc.parse();
page.doc.emit('DOMContentLoaded');
page.window.emit('yt-navigate-finish');
check(
  'a re-inlined response is pruned again on DOMContentLoaded and yt-navigate-finish',
  page.window.ytInitialPlayerResponse.adSlots === undefined,
  JSON.stringify(page.window.ytInitialPlayerResponse),
);

/* The ad boxes land in the head the parser built. */
check(
  'the cosmetic stylesheet is installed in the head',
  page.doc.head.children.some((el) => el.tagName === 'STYLE' && /display:none/.test(el.textContent)),
);
check(
  'and the document node itself stayed untouched',
  page.state.appendedToBareDocument === false,
);

/* The fallback: an ad that started anyway. */
const video = { muted: false, duration: 42, currentTime: 0 };
const skipButton = makeElement('button');
const closeButton = makeElement('button');
let adPlaying = true;
page.elements.set('#movie_player', {
  matches: (selector) => selector === '.ad-showing, .ad-interrupting' && adPlaying,
  querySelector: () => video,
});
page.elements.set('.ytp-ad-skip-button, .ytp-ad-skip-button-modern, .ytp-skip-ad-button', skipButton);
page.elements.set('.ytp-ad-overlay-close-button', closeButton);

/* One observer on the document until the player shows up, then a second one on
   the player itself — the cheap half. */
check('the document alone is observed at first', page.observers.length === 1, String(page.observers.length));
page.observers[0].callback();
check('the player is then watched for ad classes', page.observers.length === 2, String(page.observers.length));

page.runTimers(1);
check('an ad overlay is closed', closeButton.clicked);
check('a running ad is skipped through the button when there is one', skipButton.clicked);

page.elements.delete('.ytp-ad-skip-button, .ytp-ad-skip-button-modern, .ytp-skip-ad-button');
page.runTimers(1);
check(
  'without a button the ad is run to its end, muted',
  video.currentTime === video.duration && video.muted === true,
  JSON.stringify(video),
);

adPlaying = false;
page.runTimers(1);
check('the mute state is handed back when the ad ends', video.muted === false);

/* The prune interval is short-lived; the ad check keeps running. */
check('two intervals are running', page.timers.size === 2, String(page.timers.size));
page.runTimers(31);
check('the prune interval stops after its 30 ticks', page.timers.size === 1, String(page.timers.size));

/* ── A page that fights back must not break the script ───────── */

const hostile = makePage('m.youtube.com', { brokenHead: true });
check('a hostile page still gets the response patch', hostile.window.__peakdYoutubeAds === true);
hostile.state.body = '{"adPlacements":[1]}';
const hostileResponse = await hostile.window.fetch('https://m.youtube.com/youtubei/v1/player');
check(
  'and still gets its responses rewritten',
  hostileResponse.body === '{"no_ads":[1]}',
  hostileResponse.body,
);

if (failures.length) {
  console.error(`\n${failures.length} failure(s):`);
  for (const f of failures) console.error(` - ${f}`);
  process.exit(1);
}
console.log('\nall youtube scriptlet smoke checks passed');
