/**
 * "Import from another browser" (import-panel.js): the profiles the server
 * found are listed and imported by id, a file's text is sent as is, a
 * password file waits for a locked vault and finishes once it opens.
 *
 * Run:  node plugins/browser/web/import-panel.test.mjs
 */
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import vm from 'node:vm';

const here = dirname(fileURLToPath(import.meta.url));
const source = readFileSync(join(here, 'import-panel.js'), 'utf8')
  .replace(/^import\s+[\s\S]*?;[ \t]*\n?/gm, '')
  .replace(/^export\s+/gm, '');

let failures = 0;
function check(name, condition, detail = '') {
  if (condition) console.log(`  ok  ${name}`);
  else {
    failures += 1;
    console.error(`FAIL  ${name}${detail ? ` — ${detail}` : ''}`);
  }
}

function makeElement(tag) {
  const el = {
    tagName: tag.toUpperCase(), children: [], attrs: {}, dataset: {}, listeners: {}, textContent: '',
    className: '', disabled: false,
    setAttribute: (k, v) => { el.attrs[k] = String(v); },
    append: (...kids) => el.children.push(...kids),
    appendChild: (kid) => el.children.push(kid),
    replaceChildren: (...kids) => { el.children = kids; },
    addEventListener: (type, fn) => { (el.listeners[type] ||= []).push(fn); },
    click: () => { for (const fn of el.listeners.click || []) fn({}); },
    querySelectorAll: (sel) => (sel === 'button' ? findAll(el, (n) => n.tagName === 'BUTTON' && n !== el) : []),
    focus() {},
  };
  return el;
}
function findAll(root, predicate, out = []) {
  if (predicate(root)) out.push(root);
  for (const kid of root.children || []) findAll(kid, predicate, out);
  return out;
}
const textOf = (root) => findAll(root, () => true).map((n) => n.textContent).join(' ');
const rowTitled = (root, title) => findAll(root, (n) => n.tagName === 'BUTTON'
  && findAll(n, (k) => k.textContent === title).length)[0];
const tick = () => new Promise((r) => setTimeout(r, 0));
const settle = async () => { for (let i = 0; i < 6; i += 1) await tick(); };

const calls = [];
const server = { locked: false, sources: [] };
async function api(url, opts = {}) {
  calls.push({ url, body: opts.body ? JSON.parse(opts.body) : null });
  if (url === '/api/browser/import/sources') return { data: { sources: server.sources } };
  if (url === '/api/browser/import/profile') return { data: { added: { history: 40, bookmarks: 3 } } };
  if (url === '/api/browser/import/file') {
    const { text } = JSON.parse(opts.body);
    if (text.startsWith('<!DOCTYPE')) return { data: { kind: 'bookmarks', added: 7 } };
    if (!text.startsWith('url,')) throw new Error('not_an_import_file');
    if (server.locked) throw new Error('vault_locked');
    return { data: { kind: 'passwords', counts: { added: 5, updated: 1 } } };
  }
  throw new Error(`unexpected ${url}`);
}

let picked = null;
const context = vm.createContext({
  document: { createElement: makeElement },
  JSON, String, Promise, setTimeout,
  apiFetch: api,
  pickFiles: async () => picked,
  t: (key, params, fallback) => fallback ?? key,
  controlButton: () => makeElement('button'),
});
vm.runInContext(`${source}\nthis.createImportPanel = createImportPanel;`, context);

let imported = 0;
let passwordsOpened = 0;
const panel = context.createImportPanel({
  popover: (className) => { const el = makeElement('div'); el.className = className; return el; },
  onLayout: () => {},
  onImported: () => { imported += 1; },
  openPasswords: () => { passwordsOpened += 1; },
  api,
  pick: async () => picked,
});
const el = panel.element;

/* Nothing installed. */
await panel.open();
check('with no other browser it says so', textOf(el).includes('No other browser found'));
check('a file can still be imported', !!rowTitled(el, 'From a file…'));

/* Two profiles. */
server.sources = [
  { id: 'firefox-0', browser: 'Firefox', profile: 'default-release', history: true, bookmarks: true },
  { id: 'chromium-1', browser: 'Google Chrome', profile: 'Work', history: false, bookmarks: true },
];
await panel.open();
check('each profile is a row named by browser and profile',
  !!rowTitled(el, 'Firefox · default-release') && !!rowTitled(el, 'Google Chrome · Work'));
check('a row says what it brings', textOf(el).includes('History and bookmarks'));
rowTitled(el, 'Firefox · default-release').click();
await settle();
check('a profile is imported by its id, never a path',
  JSON.stringify(calls.at(-1)) === JSON.stringify({ url: '/api/browser/import/profile', body: { id: 'firefox-0' } }));
check('the result is reported', textOf(el).includes('Bookmarks imported: 3 · history pages: 40.'));
check('the window reloads its bookmarks', imported === 1);

/* Files. */
picked = [{ text: async () => '<!DOCTYPE NETSCAPE-Bookmark-file-1>' }];
rowTitled(el, 'From a file…').click();
await settle();
check('a bookmarks file is imported', textOf(el).includes('Bookmarks imported: 7.') && imported === 2);

picked = [{ text: async () => 'url,username,password\nhttps://a,b,c' }];
rowTitled(el, 'From a file…').click();
await settle();
check('a password file is imported, with a reminder to delete it',
  textOf(el).includes('Passwords imported: 5 · updated: 1.') && textOf(el).includes('delete the CSV'));

picked = [{ text: async () => 'hello' }];
rowTitled(el, 'From a file…').click();
await settle();
check('another file is refused plainly', textOf(el).includes('not a password CSV'));

picked = [];
rowTitled(el, 'From a file…').click();
await settle();
check('a cancelled pick does nothing', calls.at(-1).body?.text === 'hello');

/* A locked vault: unlock, then the import finishes by itself. */
server.locked = true;
picked = [{ text: async () => 'url,username,password\nhttps://a,b,c' }];
rowTitled(el, 'From a file…').click();
await settle();
check('a locked vault opens the unlock prompt', passwordsOpened === 1 && panel.hasPending());
server.locked = false;
panel.vaultUnlocked();
await settle();
check('once open, the import finishes', !panel.hasPending() && textOf(el).includes('Passwords imported: 5'));

if (failures) {
  console.error(`\n${failures} import-panel check(s) failed`);
  process.exit(1);
}
console.log('\nall import-panel checks passed');
