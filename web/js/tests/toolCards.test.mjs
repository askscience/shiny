/**
 * Tool cards: every family renders from a sample payload, an unknown tool
 * falls back to the generic card, a failed call is an error card, and the
 * housekeeping tools get none.
 *
 * The repo's JS tests have no DOM, so this file brings the minimum element
 * stub the renderers use (class names, text, attributes, children, clicks).
 *
 * Run:  node web/js/tests/toolCards.test.mjs
 */

import './helpers/i18n.mjs';   // catalogs, so t() resolves the English labels

let failures = 0;
function check(name, condition, detail = '') {
  if (condition) {
    console.log(`  ok  ${name}`);
  } else {
    failures += 1;
    console.error(`FAIL  ${name}${detail ? ` — ${detail}` : ''}`);
  }
}

/* ── minimal DOM ────────────────────────────────────────────── */

class FakeText {
  constructor(text) { this.nodeType = 3; this.data = String(text); }
  get textContent() { return this.data; }
}

class FakeElement {
  constructor(tag) {
    this.tagName = tag.toUpperCase();
    this.nodeType = 1;
    this.children = [];
    this.attributes = {};
    this.dataset = {};
    this.style = {};
    this.listeners = {};
    this.className = '';
    this.parentElement = null;
    this._html = '';
    const self = this;
    this.classList = {
      add: (...names) => { self.className = [...new Set([...self.classes(), ...names])].join(' '); },
      remove: (...names) => { self.className = self.classes().filter((c) => !names.includes(c)).join(' '); },
      contains: (name) => self.classes().includes(name),
      toggle: (name, force) => {
        const on = force === undefined ? !self.classes().includes(name) : force;
        if (on) self.classList.add(name); else self.classList.remove(name);
        return on;
      },
    };
  }

  classes() { return this.className.split(/\s+/).filter(Boolean); }
  append(...nodes) {
    for (const n of nodes) {
      const node = typeof n === 'string' ? new FakeText(n) : n;
      if (node instanceof FakeElement) node.parentElement = this;
      this.children.push(node);
    }
  }
  appendChild(node) { this.append(node); return node; }
  setAttribute(k, v) { this.attributes[k] = String(v); }
  getAttribute(k) { return this.attributes[k] ?? null; }
  addEventListener(type, fn) { (this.listeners[type] ||= []).push(fn); }
  removeEventListener() {}
  click() { for (const fn of this.listeners.click || []) fn({ stopPropagation() {}, preventDefault() {} }); }
  set innerHTML(v) { this._html = String(v); }
  get innerHTML() { return this._html; }
  set textContent(v) { this.children = v === '' || v == null ? [] : [new FakeText(v)]; }
  get textContent() { return this.children.map((c) => c.textContent).join(''); }

  /** All descendants (depth first) matching a single `.class` or `tag`. */
  findAll(selector) {
    const out = [];
    const match = (el) => (selector.startsWith('.')
      ? el.classes().includes(selector.slice(1))
      : el.tagName === selector.toUpperCase());
    const walk = (el) => {
      for (const c of el.children) {
        if (!(c instanceof FakeElement)) continue;
        if (match(c)) out.push(c);
        walk(c);
      }
    };
    walk(this);
    return out;
  }
  find(selector) { return this.findAll(selector)[0] || null; }
}

const dispatched = [];
Object.defineProperty(globalThis, 'document', {
  configurable: true,
  writable: true,
  value: {
    ...globalThis.document,
    documentElement: { lang: 'en', dir: 'ltr', setAttribute() {}, style: {} },
    createElement: (tag) => new FakeElement(tag),
    createTextNode: (text) => new FakeText(text),
    querySelectorAll: () => [],
  },
});
Object.defineProperty(globalThis, 'window', {
  configurable: true,
  writable: true,
  value: {
    ...globalThis.window,
    addEventListener() {},
    removeEventListener() {},
    dispatchEvent: (event) => { dispatched.push(event); return true; },
  },
});
if (typeof globalThis.CustomEvent === 'undefined') {
  globalThis.CustomEvent = class CustomEvent {
    constructor(type, init = {}) { this.type = type; this.detail = init.detail; }
  };
}
globalThis.getComputedStyle = () => ({ getPropertyValue: () => '' });

const { renderToolCard, toolCardFamily, isHousekeepingTool } = await import('../toolCards.js');

const label = (card) => card?.find('.tool-card-label')?.textContent || '';
const bigValue = (card) => card?.find('.tool-card-value')?.textContent || '';
const hasClass = (card, name) => !!card?.classList.contains(name);
const allText = (card) => card?.textContent || '';

/* ── families ───────────────────────────────────────────────── */

{
  const card = renderToolCard({
    action: 'calculator_eval', result: 'ok', plugin: 'calculator',
    data: { expression: '18% * 2340', result: 421.2, result_text: '421.2' },
  });
  check('calculator: card with family class', hasClass(card, 'tool-card') && hasClass(card, 'tool-card--calculator'));
  check('calculator: big result', bigValue(card) === '421.2', bigValue(card));
  check('calculator: expression line', card.find('.tool-card-expr')?.textContent === '18% * 2340');
  check('calculator: label', label(card) === 'Calculator', label(card));
  const open = card.find('.tool-card-open');
  check('calculator: open arrow present', !!open);
  dispatched.length = 0;
  open.click();
  check('open arrow focuses the plugin window',
    dispatched[0]?.type === 'plugin:focus' && dispatched[0]?.detail?.name === 'calculator');
}

{
  const card = renderToolCard(JSON.stringify({
    action: 'radio_play', result: 'ok', plugin: 'radio',
    data: { station: 'Lo-fi FM', stationuuid: 'x', stream_url: 'https://s' },
  }));
  check('radio_play from a stored JSON row', hasClass(card, 'tool-card--radio'));
  check('radio_play: now playing label', label(card) === 'Radio · Now playing', label(card));
  check('radio_play: station name', allText(card).includes('Lo-fi FM'));
}

{
  const card = renderToolCard({
    action: 'radio_search', result: 'ok',
    data: { count: 6, stations: Array.from({ length: 6 }, (_, i) => ({ name: `S${i}`, country: 'Italy', bitrate: 128 })) },
  });
  check('radio_search: count label', label(card) === 'Radio · 6 stations', label(card));
  check('radio_search: 4 rows + more line',
    card.findAll('.tool-card-row').length === 4 && card.find('.tool-card-more')?.textContent === '+2 more');
  check('radio: plugin inferred from the tool prefix', !!card.find('.tool-card-open'));
}

{
  const card = renderToolCard({ action: 'radio_stop', result: 'ok', data: { stopped: true } });
  check('radio_stop: one-line card', label(card) === 'Radio · Stopped' && card.children.length === 1);
}

{
  const card = renderToolCard({
    action: 'calendar_create', result: 'ok', plugin: 'calendar',
    data: { event_id: 'e1', title: 'Dinner with Marta', date: '2026-10-30', start_time: '20:30', end_time: '22:30', location: 'Trattoria', all_day: false },
  });
  check('calendar_create: event added', label(card) === 'Calendar · Event added', label(card));
  check('calendar_create: date block', card.find('.tool-card-date')?.find('b')?.textContent === '30');
  check('calendar_create: time and place', allText(card).includes('20:30 – 22:30 · Trattoria'));
}

{
  const card = renderToolCard({
    action: 'calendar_list', result: 'ok',
    data: { count: 2, from: '2026-10-01', to: '2026-10-31', events: [
      { title: 'A', date: '2026-10-02', start_time: '09:00', end_time: '10:00', all_day: false },
      { title: 'B', date: '2026-10-03', all_day: true },
    ] },
  });
  check('calendar_list: rows', card.findAll('.tool-card-row').length === 2 && label(card) === 'Calendar · 2 events');
  check('calendar_list: all-day text', allText(card).includes('All day'));
}

{
  const card = renderToolCard({
    action: 'mail_send', result: 'ok', plugin: 'mail',
    data: { to: ['luca@example.com'], sent: true },
    params: { to: ['luca@example.com'], subject: 'Q3 report', body: 'Hi Luca, here it is.' },
  });
  check('mail_send: sent label', label(card) === 'Mail · Sent', label(card));
  check('mail_send: recipient, subject and body',
    allText(card).includes('luca@example.com') && allText(card).includes('Q3 report') && allText(card).includes('Hi Luca'));
}

{
  const card = renderToolCard({
    action: 'mail_list', result: 'ok',
    data: { folder: 'INBOX', total: 42, page: 0, messages: [{ id: '1', subject: 'Hello', from: 'a@b.c', seen: false, date: '2026-10-29T10:00:00Z' }] },
  });
  check('mail_list: big total', bigValue(card) === '42');
  check('mail_list: message row', card.findAll('.tool-card-row').length === 1 && allText(card).includes('Hello'));
}

{
  const card = renderToolCard({
    action: 'mail_read', result: 'ok',
    data: { subject: 'Invoice', from: ['Shop <s@x.y>'], text: 'Your invoice is attached.', folder: 'INBOX', attachments: [{ filename: 'inv.pdf', size: 2048 }] },
  });
  check('mail_read: subject, sender, excerpt, attachment',
    ['Invoice', 'Shop <s@x.y>', 'Your invoice', 'inv.pdf', '2 KB'].every((s) => allText(card).includes(s)), allText(card));
}

{
  const card = renderToolCard({
    action: 'file_search', result: 'ok', plugin: 'files',
    data: { query: 'report', count: 2, results: [
      { name: 'report-q3.pdf', path: 'Documents/report-q3.pdf', kind: 'file', size: 2516582 },
      { name: 'Reports', path: 'Documents/Reports', kind: 'dir', size: 0 },
    ] },
  });
  check('file_search: found label', label(card) === 'Files · 2 found', label(card));
  const rowsEls = card.findAll('.tool-card-row');
  check('file_search: rows are buttons', rowsEls.length === 2 && rowsEls[0].tagName === 'BUTTON');
  check('file_search: size shown', allText(card).includes('2.4 MB'));
  dispatched.length = 0;
  rowsEls[0].click();
  check('file row opens Files at its folder',
    dispatched[0]?.detail?.name === 'files' && dispatched[0]?.detail?.path === 'Documents');
}

{
  const card = renderToolCard({
    action: 'file_read', result: 'ok', data: { path: 'notes/todo.md', content: 'line 1\nline 2', size: 13, truncated: false },
  });
  check('file_read: content in a pre block', card.find('.tool-card-pre')?.textContent === 'line 1\nline 2');
}

{
  const card = renderToolCard({
    action: 'terminal_exec', result: 'ok', plugin: 'terminal',
    data: { command: 'df -h /home', output: '/dev/nvme0n1p3  476G  412G  64G  87%\n', timed_out: false },
  });
  const pre = card.find('.tool-card-pre');
  check('terminal: prompt + command + output',
    pre?.find('.tool-card-prompt')?.textContent === '$' && pre.textContent.includes('df -h /home') && pre.textContent.includes('87%'));
  check('terminal: label', label(card) === 'Terminal', label(card));
}

{
  const card = renderToolCard({ action: 'terminal_exec', result: 'ok', data: { command: 'sleep 99', output: '', timed_out: true } });
  check('terminal: timed out label', label(card) === 'Terminal · Timed out', label(card));
}

{
  const card = renderToolCard({ action: 'browser_open', result: 'ok', plugin: 'browser', data: { url: 'https://www.giallozafferano.it/ricette' } });
  check('browser_open: host label', label(card) === 'Browser · giallozafferano.it', label(card));
  check('browser_open: url row', card.findAll('.tool-card-row').length === 1);
}

{
  const card = renderToolCard({ action: 'browser_read', result: 'ok', data: { url: 'https://example.com/a', text: 'Title line\nBody text here', chars: 24 } });
  check('browser_read: first line as title and excerpt',
    allText(card).includes('Title line') && allText(card).includes('Body text here'));
}

{
  const card = renderToolCard({
    action: 'web_search', result: 'ok', params: { query: 'risotto' },
    data: { results: [{ title: 'Risotto alla milanese', snippet: 'The classic method' }] },
  });
  check('web_search: family and query label', hasClass(card, 'tool-card--web') && label(card) === 'Web · “risotto”', label(card));
  check('web_search: core tool has no open arrow', !card.find('.tool-card-open'));
}

{
  const card = renderToolCard({
    action: 'youtube_play', result: 'ok', plugin: 'youtube',
    data: { video_id: 'v', title: 'Night driving', channel: 'Ch', thumbnail: 'https://i.ytimg.com/vi/v/hq.jpg' },
  });
  check('youtube_play: thumbnail + title', card.find('.tool-card-media')?.find('img')?.src === 'https://i.ytimg.com/vi/v/hq.jpg'
    && allText(card).includes('Night driving'));
}

{
  const card = renderToolCard({
    action: 'youtube_search', result: 'ok',
    data: { count: 1, results: [{ video_id: 'v', title: 'Lo-fi', channel: 'C', duration: '12:04', thumbnail: 'javascript:alert(1)' }] },
  });
  check('youtube_search: row, unsafe thumbnail dropped',
    card.findAll('.tool-card-row').length === 1 && !card.find('.tool-card-thumb') && allText(card).includes('12:04'));
}

{
  const card = renderToolCard({
    action: 'image_edit', result: 'ok', plugin: 'image',
    data: { image_id: 'i', title: 'Beach', width: 1024, height: 768, look: 'warm', operations_applied: 3 },
  });
  check('image_edit: size label and cells', label(card) === 'Image · 1024 × 768' && allText(card).includes('warm'), label(card));
}

{
  const card = renderToolCard({
    action: 'map_route', result: 'ok', plugin: 'traveler', params: { profile: 'car' },
    data: { route: { total_distance_meters: 52000, total_duration_seconds: 2880, steps: [{ distance: 100, duration: 10, instruction: 'Head north' }] } },
  });
  check('map_route: big minutes', bigValue(card) === '48' && card.find('.tool-card-unit')?.textContent === 'min');
  check('map_route: driving label, km meta', label(card) === 'Route · Driving' && allText(card).includes('52 km'), label(card));
  check('traveler has no window: no open arrow', !card.find('.tool-card-open'));
}

{
  const card = renderToolCard({
    action: 'map_route', result: 'ok', data: { route: { total_distance_meters: 120000, total_duration_seconds: 4320 } },
  });
  check('map_route: an hour and more reads h:mm', bigValue(card) === '1:12', bigValue(card));
}

{
  const card = renderToolCard({
    action: 'map_poi', result: 'ok', params: { category: 'museum' },
    data: { places: [{ display_name: 'Brera, Milano, Italia', name: 'Pinacoteca di Brera', lat: 1, lon: 2, place_type: 'museum' }] },
  });
  check('map_poi: places rows', hasClass(card, 'tool-card--places') && allText(card).includes('Pinacoteca di Brera'));
}

{
  const card = renderToolCard({
    action: 'plan_trip', result: 'ok',
    data: { destination: { name: 'Malpensa, Italia' }, guides_created: 3, route: { distance_km: 52.1, duration_min: 47.6 } },
  });
  check('plan_trip: trip card with minutes', hasClass(card, 'tool-card--trip') && bigValue(card) === '48');
}

{
  const card = renderToolCard({ action: 'doc_create', result: 'ok', plugin: 'word', data: { doc_id: 'd', title: 'Q3 report' } });
  check('doc_create: document card', hasClass(card, 'tool-card--document') && label(card) === 'Document · Created'
    && allText(card).includes('Q3 report'), label(card));
  const open = card.find('.tool-card-open');
  dispatched.length = 0;
  open?.click();
  check('doc card opens Word', dispatched[0]?.detail?.name === 'word');
}

{
  const card = renderToolCard({ action: 'pdf_list', result: 'ok', data: { count: 1, documents: [{ pdf_id: 'p', title: 'Lease', page_count: 4 }] } });
  check('pdf_list: list rows with pages', label(card) === 'PDF · 1 item' && allText(card).includes('4 pages'), label(card));
}

{
  const card = renderToolCard({ action: 'calc_write', result: 'ok', data: { sheet_id: 's', title: 'Budget', rows: 10, cols: 4 } });
  check('calc_write: sheet card with cells', label(card) === 'Sheet · Written' && !!card.find('.tool-card-cells'), label(card));
}

{
  const card = renderToolCard({
    action: 'insights_weather', result: 'ok',
    data: { place: 'Milano', current: { temperature: 16.4, weather_code: 2, temperature_max: 17, temperature_min: 11 } },
  });
  check('weather payload: weather card', hasClass(card, 'tool-card--weather') && bigValue(card) === '16'
    && label(card) === 'Weather · Milano', label(card));
}

/* ── fallback, error, housekeeping ──────────────────────────── */

{
  const card = renderToolCard({
    action: 'hello', result: 'ok', plugin: 'hello',
    data: { greeting: 'Hello there', session_id: 'hidden', nested: { a: 1 }, ok: true },
  });
  check('unknown tool: generic card', hasClass(card, 'tool-card--generic'));
  check('generic: key/value summary without ids or nested data',
    allText(card).includes('Greeting') && allText(card).includes('Hello there') && !allText(card).includes('hidden'), allText(card));
  check('generic: boolean as Yes', allText(card).includes('Yes'));
}

{
  const card = renderToolCard({ action: 'mail_send', result: 'error', plugin: 'mail', data: { error: 'SMTP refused the login' } });
  check('error: error card', hasClass(card, 'tool-card--error') && hasClass(card, 'tool-card--mail'));
  check('error: failed label + message', label(card).endsWith('Failed') && allText(card).includes('SMTP refused'), label(card));
}

{
  const card = renderToolCard({ action: 'show_plugin', result: 'error', data: { error: 'no such plugin' } });
  check('a failed housekeeping tool still shows its error', hasClass(card, 'tool-card--error'));
}

for (const action of ['show_plugin', 'plugin_activate', 'workspace_create', 'desktop_arrange', 'window_close',
  'list_plugins', 'show_artifact', 'generate_image', 'generate_song']) {
  check(`no card for ${action}`, renderToolCard({ action, result: 'ok', data: { plugin: 'x' } }) === null);
}
check('housekeeping helper', isHousekeepingTool('plugin_deactivate') && !isHousekeepingTool('radio_play'));
check('namespaced tool names resolve', toolCardFamily('word.doc_write') === 'document');
check('malformed record: no card', renderToolCard('not json') === null && renderToolCard({}) === null && renderToolCard(null) === null);

/* ── model text never becomes markup ────────────────────────── */

{
  const card = renderToolCard({
    action: 'web_search', result: 'ok', params: { query: '<img src=x onerror=alert(1)>' },
    data: { results: [{ title: '<script>alert(1)</script>', snippet: '<b>x</b>' }] },
  });
  const anyHtml = (el) => (el._html && el._html.includes('<script')) || el.children.some((c) => c instanceof FakeElement && anyHtml(c));
  check('untrusted strings land as text', allText(card).includes('<script>alert(1)</script>') && !anyHtml(card));
}

if (failures) {
  console.error(`\n${failures} check(s) failed`);
  process.exit(1);
}
console.log('\nall toolCards checks passed');
