/**
 * The Browser's address-bar completion (address-suggest.js): the inline tail
 * keeps the user's own letters, deleting never completes, a stale reply is
 * dropped, arrows + Enter pick a row, Escape drops the tail.
 *
 * Run:  node plugins/browser/web/address-suggest.test.mjs
 */
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import vm from 'node:vm';

const here = dirname(fileURLToPath(import.meta.url));
const source = readFileSync(join(here, 'address-suggest.js'), 'utf8')
  .replace(/^import\s+[\s\S]*?;[ \t]*\n?/gm, '')
  .replace(/^export\s+/gm, '');

let failures = 0;
function check(name, condition, detail = '') {
  if (condition) {
    console.log(`  ok  ${name}`);
  } else {
    failures += 1;
    console.error(`FAIL  ${name}${detail ? ` — ${detail}` : ''}`);
  }
}

function makeElement(tag) {
  const el = {
    tagName: tag.toUpperCase(), children: [], attrs: {}, listeners: {}, textContent: '', className: '',
    type: '', style: {},
    classList: {
      contains: (c) => el.className.split(/\s+/).includes(c),
      add: (c) => { if (!el.classList.contains(c)) el.className = `${el.className} ${c}`.trim(); },
      remove: (c) => { el.className = el.className.split(/\s+/).filter((x) => x && x !== c).join(' '); },
      toggle: (c, on) => {
        const want = on === undefined ? !el.classList.contains(c) : !!on;
        if (want) el.classList.add(c); else el.classList.remove(c);
      },
    },
    setAttribute: (k, v) => { el.attrs[k] = String(v); },
    append: (...kids) => el.children.push(...kids),
    replaceChildren: (...kids) => { el.children = kids; },
    addEventListener: (type, fn) => { (el.listeners[type] ||= []).push(fn); },
    dispatch: (type, event = {}) => {
      for (const fn of el.listeners[type] || []) fn({ preventDefault() {}, stopPropagation() {}, ...event });
    },
    querySelectorAll: (selector) => {
      const cls = selector.replace(/^\./, '');
      const out = [];
      const walk = (n) => { for (const k of n.children || []) { if (k.classList?.contains(cls)) out.push(k); walk(k); } };
      walk(el);
      return out;
    },
  };
  return el;
}

/* The bar: a text input with a caret/selection. */
const input = makeElement('input');
input.value = '';
input.selectionStart = 0;
input.selectionEnd = 0;
input.setSelectionRange = (a, b) => { input.selectionStart = a; input.selectionEnd = b; };
input.blur = () => { document.activeElement = null; input.dispatch('blur'); };
const document = { createElement: makeElement, activeElement: input };

/** Type `text` as the user would: value, caret at the end, an input event. */
function type(text, inputType = 'insertText') {
  input.value = text;
  input.setSelectionRange(text.length, text.length);
  input.dispatch('input', { inputType });
}

let reply = { completion: 'youtube.com', items: [] };
let gate = null;
const asked = [];
const fetchSuggestions = (q) => {
  asked.push(q);
  return gate ? gate.then(() => reply) : Promise.resolve(reply);
};
const picked = [];
let changes = 0;

const context = vm.createContext({
  document, console, Promise, String, Array, encodeURIComponent,
  apiFetch: async () => ({}),
  t: (key, params, fallback) => fallback ?? key,
  controlButton: () => makeElement('button'),
  icon: (name) => { const node = makeElement('span'); node.className = 'ui-icon'; node.attrs.icon = name; return node; },
});
vm.runInContext(`${source}\nthis.api = { createAddressSuggest, inlineCompletion, addressBody };`, context);
const { createAddressSuggest, inlineCompletion } = context.api;

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

/* Pure helpers. */
check('completes the tail after the typed letters',
  JSON.stringify(inlineCompletion('you', 'youtube.com')) === JSON.stringify({ value: 'youtube.com', start: 3, end: 11 }));
check('keeps the scheme and www the user typed',
  inlineCompletion('https://www.you', 'youtube.com')?.value === 'https://www.youtube.com');
check('keeps the user\'s capitals', inlineCompletion('You', 'youtube.com')?.value === 'Youtube.com');
check('never completes a phrase', inlineCompletion('you tube', 'youtube.com') === null);
check('nothing to add to a finished address', inlineCompletion('youtube.com', 'youtube.com') === null);
check('a completion that does not extend the text is ignored', inlineCompletion('git', 'youtube.com') === null);

const suggest = createAddressSuggest({
  input,
  onPick: (value) => picked.push(value),
  onChange: () => { changes += 1; },
  fetchSuggestions,
});

/* Typing fills in the tail, selected. */
reply = {
  completion: 'youtube.com',
  items: [
    { kind: 'history', value: 'https://www.youtube.com/', display: 'youtube.com', title: null },
    { kind: 'bookmark', value: 'https://yourls.org/', display: 'yourls.org', title: 'YOURLS' },
    { kind: 'search', value: 'youtube music', display: '', title: null },
  ],
};
type('you');
await tick();
check('asks with what was typed', asked.at(-1) === 'you', asked.at(-1));
check('fills in the address', input.value === 'youtube.com', input.value);
check('selects only the added tail', input.selectionStart === 3 && input.selectionEnd === 11,
  `${input.selectionStart}-${input.selectionEnd}`);
check('opens the list', suggest.isOpen() && suggest.panel.children.length === 3);
check('the window hears the list open (its mask follows)', changes > 0);
check('a bookmark row shows its title and address', JSON.stringify(suggest.panel.children[1].children[1].children.map((c) => c.textContent)) === '["YOURLS","yourls.org"]');

/* Deleting the selected tail must stick. */
type('you', 'deleteContentBackward');
await tick();
check('deleting does not complete again', input.value === 'you', input.value);

/* A reply for older text is dropped. */
let release;
gate = new Promise((resolve) => { release = resolve; });
type('yo');
type('yout');
gate = null;
release();
await tick();
await tick();
check('a stale reply does not overwrite newer typing', input.value === 'youtube.com' && input.selectionStart === 4,
  `${input.value} ${input.selectionStart}`);

/* Arrows + Enter pick a row. */
input.dispatch('keydown', { key: 'ArrowDown' });
input.dispatch('keydown', { key: 'ArrowDown' });
let prevented = false;
input.dispatch('keydown', { key: 'Enter', preventDefault() { prevented = true; } });
check('Enter on a highlighted row opens it', picked.at(-1) === 'https://yourls.org/', JSON.stringify(picked));
check('and takes Enter from the form', prevented);
check('the list closes after a pick', !suggest.isOpen());

/* Escape drops the inline tail and keeps what was typed. */
document.activeElement = input;
type('you');
await tick();
check('completed again on new typing', input.value === 'youtube.com');
input.dispatch('keydown', { key: 'Escape' });
check('Escape drops the tail', input.value === 'you', input.value);
check('Escape closes the list', !suggest.isOpen());

/* A past search is searched again: its value is the phrase. */
type('you');
await tick();
input.dispatch('keydown', { key: 'ArrowUp' });
input.dispatch('keydown', { key: 'Enter' });
check('ArrowUp wraps to the last row (a past search)', picked.at(-1) === 'youtube music', JSON.stringify(picked));

/* Leaving the bar closes the list. */
document.activeElement = input;
type('you');
await tick();
input.blur();
check('blur closes the list', !suggest.isOpen());

/* An empty bar asks nothing. */
document.activeElement = input;
const before = asked.length;
type('', 'deleteContentBackward');
await tick();
check('an empty bar asks nothing and shows nothing', asked.length === before && !suggest.isOpen());

if (failures) {
  console.error(`\n${failures} address suggestion check(s) failed`);
  process.exit(1);
}
console.log('\nall address suggestion checks passed');
