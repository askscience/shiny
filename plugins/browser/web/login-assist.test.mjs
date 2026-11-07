/**
 * The Browser's automatic password help (login-assist.js): when a submitted
 * login is offered for saving or updating, when it is not (same password,
 * incognito, another origin, "Never"), what "Save" does with the vault open
 * or locked, filling the one saved login, and the strong-password offer.
 *
 * The page is a stand-in: `runPageJs` answers each page script by its kind.
 *
 * Run:  node plugins/browser/web/login-assist.test.mjs
 */
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import vm from 'node:vm';

const here = dirname(fileURLToPath(import.meta.url));
const source = readFileSync(join(here, 'login-assist.js'), 'utf8')
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

/* A small DOM: enough for the offer popover. */
function makeElement(tag) {
  const el = {
    tagName: tag.toUpperCase(), children: [], attrs: {}, listeners: {}, textContent: '', className: '',
    type: '',
    classList: {
      contains: (c) => el.className.split(/\s+/).includes(c),
      add: (c) => { if (!el.classList.contains(c)) el.className = `${el.className} ${c}`.trim(); },
      remove: (c) => { el.className = el.className.split(/\s+/).filter((x) => x && x !== c).join(' '); },
    },
    setAttribute: (k, v) => { el.attrs[k] = String(v); },
    append: (...kids) => el.children.push(...kids),
    replaceChildren: (...kids) => { el.children = kids; },
    addEventListener: (type, fn) => { (el.listeners[type] ||= []).push(fn); },
    click: () => { for (const fn of el.listeners.click || []) fn({ stopPropagation() {} }); },
  };
  return el;
}
const document = { createElement: makeElement };
function findAll(root, predicate, out = []) {
  if (predicate(root)) out.push(root);
  for (const kid of root.children || []) findAll(kid, predicate, out);
  return out;
}
const textOf = (root) => findAll(root, () => true).map((n) => n.textContent).join(' ');
// A button is named by its text, or by its label when it also carries an icon.
const buttonNamed = (root, name) => findAll(root, (n) => n.tagName === 'BUTTON'
  && (n.textContent === name || n.attrs['aria-label'] === name))[0];

const storage = new Map();
const localStorage = {
  getItem: (k) => (storage.has(k) ? storage.get(k) : null),
  setItem: (k, v) => storage.set(k, String(v)),
};

/** The vault the stand-in API serves. */
const vault = { exists: true, unlocked: true, entries: [], secrets: {} };
const calls = [];
async function api(url, opts = {}) {
  calls.push({ url, body: opts.body ? JSON.parse(opts.body) : null });
  if (url === '/api/browser/vault') return { data: { exists: vault.exists, unlocked: vault.unlocked } };
  if (!vault.unlocked) throw new Error('vault_locked');
  if (url.startsWith('/api/browser/passwords?url=')) return { data: { entries: vault.entries } };
  if (url === '/api/browser/passwords/reveal') return { data: vault.secrets[JSON.parse(opts.body).id] };
  if (url === '/api/browser/passwords/save') return { data: { id: 'new' } };
  throw new Error(`unexpected ${url}`);
}

/** What the page answers. */
const pageState = { kept: null, kind: 'none', scripts: [] };
async function runPageJs(_tab, script) {
  pageState.scripts.push(script);
  if (script.includes('sessionStorage.removeItem')) {
    const kept = pageState.kept;
    pageState.kept = null;
    return kept ? JSON.stringify(kept) : '';
  }
  if (script.includes('__liteauraLoginWatch')) return JSON.stringify({ kind: pageState.kind });
  if (script.startsWith('SUGGEST')) return 'ok';
  if (script.startsWith('FILL')) return 'ok';
  return '';
}

const opened = [];
let active = 'a';
const context = vm.createContext({
  document, localStorage, JSON, URL, Set, Map, Promise, String, Array, setTimeout, clearTimeout,
  apiFetch: api,
  t: (key, params, fallback) => fallback ?? key,
  controlButton: () => makeElement('button'),
  icon: () => makeElement('span'),
  fillScript: (origin, user, pass, opts) => `FILL ${origin} ${user} ${pass} ${opts?.onlyEmpty ? 'empty' : ''}`,
  suggestScript: (origin, pass) => `SUGGEST ${origin} ${pass}`,
  takeScript: () => 'TAKE sessionStorage.removeItem',
  watchScript: () => 'WATCH __liteauraLoginWatch',
  generatePassword: () => 'Strong-Pass-123456789',
});
vm.runInContext(`${source}\nthis.createLoginAssist = createLoginAssist;`, context);

const assist = context.createLoginAssist({
  popover: (className) => { const el = makeElement('div'); el.className = `browser-popover ${className} hidden`; return el; },
  runPageJs,
  originOf: (tab) => { try { return new URL(tab.url).origin; } catch (_) { return ''; } },
  isActive: (tab) => tab.id === active,
  openPanel: (values) => opened.push(values),
  onShow: () => {},
  api,
});
const offer = assist.element;
const shown = () => !offer.classList.contains('hidden');
const tick = () => new Promise((r) => setTimeout(r, 0));

function reset({ unlocked = true, exists = true, entries = [], secrets = {} } = {}) {
  Object.assign(vault, { unlocked, exists, entries, secrets });
  pageState.kept = null;
  pageState.kind = 'none';
  pageState.scripts = [];
  calls.length = 0;
  opened.length = 0;
  assist.hide();
}
const tab = (extra = {}) => ({ id: 'a', url: 'https://shop.example/account', ...extra });

/* A new login is offered, and "Save" saves it. */
reset();
let t1 = tab();
pageState.kept = { origin: 'https://shop.example', username: 'me@x.com', password: 'pw1' };
await assist.pageLoaded(t1);
check('a submitted login is offered for saving', shown() && textOf(offer).includes('Save the password for shop.example?'),
  textOf(offer));
check('the offer names the account', textOf(offer).includes('me@x.com'));
buttonNamed(offer, 'Save').click();
await tick(); await tick();
const saved = calls.find((c) => c.url === '/api/browser/passwords/save');
check('"Save" stores it for the origin the window loaded',
  saved?.body?.site === 'https://shop.example' && saved.body.username === 'me@x.com' && saved.body.password === 'pw1',
  JSON.stringify(saved));
check('and the offer goes away', !shown());

/* The same password: nothing to say. */
reset({ entries: [{ id: 'e1', origin: 'https://shop.example', username: 'me@x.com' }],
  secrets: { e1: { username: 'me@x.com', password: 'pw1' } } });
pageState.kept = { origin: 'https://shop.example', username: 'me@x.com', password: 'pw1' };
await assist.pageLoaded(tab());
check('a login already saved as it is is not offered', !shown());

/* A changed password: offer to update that entry. */
reset({ entries: [{ id: 'e1', origin: 'https://shop.example', username: 'me@x.com' }],
  secrets: { e1: { username: 'me@x.com', password: 'old' } } });
pageState.kept = { origin: 'https://shop.example', username: 'me@x.com', password: 'new' };
await assist.pageLoaded(tab());
check('a changed password is offered as an update', shown() && textOf(offer).includes('Update the saved password'));
buttonNamed(offer, 'Update').click();
await tick(); await tick();
check('"Update" rewrites that entry',
  calls.find((c) => c.url === '/api/browser/passwords/save')?.body?.id === 'e1');

/* Never offered: incognito, another origin. */
reset();
pageState.kept = { origin: 'https://shop.example', username: 'me', password: 'pw' };
await assist.pageLoaded(tab({ incognito: true }));
check('an incognito tab is never offered to save', !shown());
reset();
pageState.kept = { origin: 'https://bank.example', username: 'me', password: 'pw' };
await assist.pageLoaded(tab());
check('a login a page claims for another origin is ignored', !shown());

/* Locked vault: "Save" opens the unlock prompt with the login. */
reset({ unlocked: false });
pageState.kept = { origin: 'https://shop.example', username: 'me', password: 'pw' };
await assist.pageLoaded(tab());
check('with the vault locked the login is still offered', shown());
buttonNamed(offer, 'Save').click();
await tick(); await tick();
check('"Save" then opens the passwords panel prefilled',
  opened[0]?.site === 'https://shop.example' && opened[0]?.password === 'pw', JSON.stringify(opened));

/* "Never" is remembered. */
reset();
pageState.kept = { origin: 'https://forum.example', username: 'me', password: 'pw' };
await assist.pageLoaded({ id: 'a', url: 'https://forum.example/' });
buttonNamed(offer, 'Never').click();
reset();
pageState.kept = { origin: 'https://forum.example', username: 'me', password: 'pw2' };
await assist.pageLoaded({ id: 'a', url: 'https://forum.example/' });
check('"Never" is remembered for the site', !shown());

/* Filling the one saved login. */
reset({ entries: [{ id: 'e1', origin: 'https://shop.example', username: 'me' }],
  secrets: { e1: { username: 'me', password: 'pw' } } });
pageState.kind = 'login';
await assist.pageLoaded(tab());
check('the one saved login fills empty fields on its own',
  pageState.scripts.some((s) => s === 'FILL https://shop.example me pw empty'), pageState.scripts.join(' | '));
reset({ entries: [{ id: 'p1', origin: 'https://example', username: 'parent' }],
  secrets: { p1: { username: 'parent', password: 'pw' } } });
pageState.kind = 'login';
await assist.pageLoaded({ id: 'a', url: 'https://sub.example/login' });
check('a parent domain\'s login is never filled unasked', !pageState.scripts.some((s) => s.startsWith('FILL')));
reset({ entries: [{ id: 'e1', origin: 'https://shop.example', username: 'a' }, { id: 'e2', origin: 'https://shop.example', username: 'b' }] });
pageState.kind = 'login';
await assist.pageLoaded(tab());
check('with two saved logins nothing is guessed', !pageState.scripts.some((s) => s.startsWith('FILL')));
check('with two saved logins the user is pointed to the list', shown() && textOf(offer).includes('Several logins'));
buttonNamed(offer, 'Fill password…').click();
check('"Fill password…" opens the panel', opened.length === 1 && opened[0] === null);
reset({ entries: [{ id: 'e1', origin: 'https://shop.example', username: 'a' }, { id: 'e2', origin: 'https://shop.example', username: 'b' }] });
pageState.kind = 'login';
await assist.pageLoaded(tab());
check('the list is pointed to once per site', !shown());

reset({ unlocked: false });
pageState.kind = 'login';
await assist.pageLoaded(tab());
check('a locked vault on a login page offers to unlock', shown() && textOf(offer).includes('Unlock'));
buttonNamed(offer, 'Unlock').click();
check('"Unlock" opens the panel', opened.length === 1 && opened[0] === null);
reset({ unlocked: false });
pageState.kind = 'login';
await assist.pageLoaded(tab());
check('the unlock hint is shown once per site', !shown());

/* Sign-up: a strong password. */
reset();
pageState.kind = 'signup';
await assist.pageLoaded({ id: 'a', url: 'https://new.example/join' });
check('a sign-up form is offered a strong password', shown() && textOf(offer).includes('strong password'));
buttonNamed(offer, 'Use it').click();
await tick(); await tick();
check('"Use it" puts a generated password into the form',
  pageState.scripts.includes('SUGGEST https://new.example Strong-Pass-123456789'));

/* A background tab keeps its offer until the user comes back to it. */
reset();
active = 'b';
const background = { id: 'a', url: 'https://shop.example/' };
pageState.kept = { origin: 'https://shop.example', username: 'me', password: 'pw' };
await assist.pageLoaded(background);
check('a background tab does not cover the active one', !shown());
active = 'a';
assist.tabActivated(background);
check('its offer shows when the user switches to it', shown());

if (failures) {
  console.error(`\n${failures} login-assist check(s) failed`);
  process.exit(1);
}
console.log('\nall login-assist checks passed');
