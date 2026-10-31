/**
 * Password-manager page scripts.
 *
 * `fillScript` / `readScript` run inside a web page, so they are checked here
 * against a tiny DOM stand-in: the origin guard (a page that navigated away
 * never receives the password), which fields are picked, the events a site's
 * framework listens for, and that hostile values stay inert data.
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
const sandbox = vm.createContext({});
vm.runInContext(`${source}\nthis.fillScript = fillScript; this.readScript = readScript;`, sandbox);
const { fillScript, readScript } = sandbox;

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
function page(origin, { withPassword = true } = {}) {
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
  const pass = withPassword ? new Input('password', 'password') : null;
  const all = [search, email, pass].filter(Boolean);
  const document = {
    querySelectorAll(selector) {
      if (selector === 'input[type=password]') return all.filter((i) => i.type === 'password');
      return all.filter((i) => ['text', 'email', 'tel'].includes(i.type));
    },
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
    HTMLInputElement,
    Node: { DOCUMENT_POSITION_FOLLOWING: 4 },
    Event: class { constructor(type) { this.type = type; } },
    JSON,
  });
  return { run: (script) => vm.runInContext(script, context), email, pass, events };
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

if (failures) {
  console.error(`\n${failures} password check(s) failed`);
  process.exit(1);
}
console.log('\nall password page-script checks passed');
