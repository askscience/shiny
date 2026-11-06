/**
 * Password-manager page scripts.
 *
 * The page scripts (`fillScript`, `readScript`, `watchScript`, `takeScript`,
 * `suggestScript`) run inside a web page, so they are checked here against a
 * tiny DOM stand-in: the origin guard (a page that navigated away never
 * receives the password), which fields are picked, the events a site's
 * framework listens for, that hostile values stay inert data, and that a
 * submitted login is handed over once. `generatePassword` is checked for
 * length, character classes and look-alikes.
 *
 * Run:  node plugins/browser/web/passwords.test.mjs
 */
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import vm from 'node:vm';

const here = dirname(fileURLToPath(import.meta.url));
// The module's imports reach the app; the two page-script builders need none
// of them, so the file is evaluated as a plain script with imports dropped.
const source = readFileSync(join(here, 'passwords.js'), 'utf8')
  .replace(/^import\s+[\s\S]*?;[ \t]*\n?/gm, '')
  .replace(/^export\s+/gm, '');
const sandbox = vm.createContext({ crypto: globalThis.crypto, Uint32Array });
vm.runInContext(`${source}
this.fillScript = fillScript; this.readScript = readScript; this.watchScript = watchScript;
this.takeScript = takeScript; this.suggestScript = suggestScript; this.generatePassword = generatePassword;`, sandbox);
const { fillScript, readScript, watchScript, takeScript, suggestScript, generatePassword } = sandbox;

let failures = 0;
function check(name, condition, detail = '') {
  if (condition) {
    console.log(`  ok  ${name}`);
  } else {
    failures += 1;
    console.error(`FAIL  ${name}${detail ? ` — ${detail}` : ''}`);
  }
}

/** A login page: a hidden search box, an email field, a password field. */
function page(origin, { withPassword = true, passwords = withPassword ? 1 : 0, newPassword = false } = {}) {
  const events = [];
  const order = [];
  class Input {
    constructor(type, name, shown = true) {
      this.type = type;
      this.name = name;
      this.id = name;
      this.autocomplete = '';
      this.value = '';
      this.disabled = false;
      this.readOnly = false;
      this.form = null;
      this.offsetWidth = shown ? 100 : 0;
      this.offsetHeight = shown ? 20 : 0;
      order.push(this);
    }
    getClientRects() { return this.offsetWidth ? [{}] : []; }
    focus() {}
    dispatchEvent(event) { events.push(`${this.name}:${event.type}`); }
    compareDocumentPosition(other) {
      return order.indexOf(other) > order.indexOf(this) ? 4 : 2;
    }
  }
  const search = new Input('search', 'q', false);
  const email = new Input('email', 'email');
  const passes = Array.from({ length: passwords }, (_, i) => new Input('password', i ? `password${i + 1}` : 'password'));
  if (newPassword) for (const p of passes) p.autocomplete = 'new-password';
  const pass = passes[0] || null;
  const all = [search, email, ...passes];
  const listeners = {};
  const document = {
    querySelectorAll(selector) {
      if (selector === 'input[type=password]') return all.filter((i) => i.type === 'password');
      return all.filter((i) => ['text', 'email', 'tel'].includes(i.type));
    },
    addEventListener(type, fn) { (listeners[type] ||= []).push(fn); },
  };
  const store = new Map();
  const sessionStorage = {
    getItem: (k) => (store.has(k) ? store.get(k) : null),
    setItem: (k, v) => store.set(k, String(v)),
    removeItem: (k) => store.delete(k),
  };
  // The page script calls the native `value` setter (frameworks shadow the
  // instance one); here it writes the stand-in's own field.
  const HTMLInputElement = function HTMLInputElement() {};
  Object.defineProperty(HTMLInputElement.prototype, 'value', {
    set(v) { this.value = v; },
  });
  const context = vm.createContext({
    location: { origin },
    document,
    sessionStorage,
    HTMLInputElement,
    Node: { DOCUMENT_POSITION_FOLLOWING: 4 },
    Event: class { constructor(type) { this.type = type; } },
    JSON,
  });
  context.window = context;
  context.addEventListener = (type, fn) => { (listeners[type] ||= []).push(fn); };
  const fire = (type, event = {}) => { for (const fn of listeners[type] || []) fn(event); };
  return { run: (script) => vm.runInContext(script, context), email, pass, passes, events, fire, store, listeners };
}

const tricky = `p"w'</script>\${x}\`;alert(1)//`;

const login = page('https://example.com');
check('a matching page is filled', login.run(fillScript('https://example.com', 'me@x.com', tricky)) === 'ok');
check('the username lands in the visible field before the password', login.email.value === 'me@x.com');
check('the password arrives byte for byte', login.pass.value === tricky, login.pass.value);
check(
  'the page hears input and change, as typing would raise',
  login.events.includes('password:input') && login.events.includes('password:change'),
  login.events.join(' '),
);

const moved = page('https://evil.example');
check('a page on another origin gets nothing',
  moved.run(fillScript('https://example.com', 'me', 'secret')) === 'origin'
  && moved.pass.value === '' && moved.email.value === '');

const stepOne = page('https://example.com', { withPassword: false });
check('a username-first sign-in fills the username only',
  stepOne.run(fillScript('https://example.com', 'me', 'secret')) === 'user'
  && stepOne.email.value === 'me');

const typed = page('https://example.com');
typed.email.value = 'typed@x.com';
typed.pass.value = tricky;
const read = JSON.parse(typed.run(readScript()));
check('reading returns what was typed', read.username === 'typed@x.com' && read.password === tricky
  && read.origin === 'https://example.com', JSON.stringify(read));

/* Filling on its own never overwrites what the user typed. */
const busy = page('https://example.com');
busy.pass.value = 'mine';
check('an automatic fill leaves a typed password alone',
  busy.run(fillScript('https://example.com', 'me', 'saved', { onlyEmpty: true })) === 'typed'
  && busy.pass.value === 'mine' && busy.email.value === '');
const empty = page('https://example.com');
check('an automatic fill fills empty fields',
  empty.run(fillScript('https://example.com', 'me', 'saved', { onlyEmpty: true })) === 'ok'
  && empty.pass.value === 'saved');

/* Watching a login form and handing over what was submitted. */
const watched = page('https://example.com');
check('a login form is recognised', JSON.parse(watched.run(watchScript())).kind === 'login');
watched.run(watchScript());
check('watching twice installs the listeners once', (watched.listeners.submit || []).length === 1);
watched.email.value = 'me@x.com';
watched.pass.value = tricky;
watched.fire('submit');
const taken = JSON.parse(watched.run(takeScript()) || 'null');
check('a submitted login is handed over',
  taken?.username === 'me@x.com' && taken?.password === tricky && taken?.origin === 'https://example.com',
  JSON.stringify(taken));
check('it is handed over once', watched.run(takeScript()) === '' && !watched.store.size);

const posted = page('https://example.com');
posted.run(watchScript());
posted.email.value = 'a';
posted.pass.value = 'b';
posted.fire('keydown', { key: 'Enter', target: { tagName: 'INPUT' } });
posted.run('window.__liteauraLogin = null');
check('the next page of the same site still finds it (sessionStorage)',
  JSON.parse(posted.run(takeScript()) || 'null')?.password === 'b');

const blank = page('https://example.com');
blank.run(watchScript());
blank.fire('submit');
check('an empty password is not kept', blank.run(takeScript()) === '');

const none = page('https://example.com', { withPassword: false });
check('a page without a password field is no form', JSON.parse(none.run(watchScript())).kind === 'none');

/* Sign-up and change-password forms. */
const signup = page('https://example.com', { passwords: 2 });
check('two password fields are a sign-up form', JSON.parse(signup.run(watchScript())).kind === 'signup');
check('a suggested password fills the password and its confirmation',
  signup.run(suggestScript('https://example.com', tricky)) === 'ok'
  && signup.passes.every((p) => p.value === tricky));
check('a suggestion never reaches another site',
  page('https://evil.example', { passwords: 2 }).run(suggestScript('https://example.com', 'x')) === 'origin');
const loginOnly = page('https://example.com');
check('a plain login form gets no suggestion', loginOnly.run(suggestScript('https://example.com', 'x')) === 'nofield'
  && loginOnly.pass.value === '');
const fresh = page('https://example.com', { newPassword: true });
check('autocomplete="new-password" marks a sign-up form', JSON.parse(fresh.run(watchScript())).kind === 'signup');

const change = page('https://example.com', { passwords: 3 });
change.run(watchScript());
change.passes[0].value = 'old';
change.passes[1].value = 'new';
change.passes[2].value = 'new';
change.fire('click', { target: { closest: () => ({}) } });
check('a change-password form keeps the new password',
  JSON.parse(change.run(takeScript()) || 'null')?.password === 'new');

/* Strong passwords. */
const samples = Array.from({ length: 200 }, () => generatePassword());
check('a generated password has 20 characters', samples.every((p) => p.length === 20));
check('every class is in every password', samples.every((p) =>
  /[a-z]/.test(p) && /[A-Z]/.test(p) && /[0-9]/.test(p) && /[^a-zA-Z0-9]/.test(p)));
check('no look-alike characters', samples.every((p) => !/[0O1lI]/.test(p)));
check('no two are the same', new Set(samples).size === samples.length);
check('never shorter than 12', generatePassword(4).length === 12);

if (failures) {
  console.error(`\n${failures} password check(s) failed`);
  process.exit(1);
}
console.log('\nall password page-script checks passed');
