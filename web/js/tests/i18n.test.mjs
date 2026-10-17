/**
 * i18n.js unit checks + the catalog/key consistency checker.
 * Run: node web/js/tests/i18n.test.mjs
 *
 * The unit half runs the real runtime against browser stubs (localStorage /
 * fetch / document / navigator) — i18n.js only touches them when called, so
 * stubbing is enough. The checker half parses the tree: every `t('…')` /
 * `tp('…')` key and every `data-i18n*` key must exist in web/i18n/en.json,
 * which is what keeps the reference catalog complete while surfaces are
 * translated one by one.
 */
import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs';
import { join, extname } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../../../', import.meta.url));

let failures = 0;
function check(name, condition, detail) {
  if (condition) console.log(`  ok  ${name}`);
  else {
    failures += 1;
    console.error(`FAIL  ${name}${detail ? ` — ${detail}` : ''}`);
  }
}

/* ── browser stubs ───────────────────────────────────────────── */

const store = new Map();
// Node ≥ 22 defines some of these as getter-only globals; define over them.
function stub(name, value) {
  Object.defineProperty(globalThis, name, { value, configurable: true, writable: true });
}
stub('localStorage', {
  getItem: (k) => (store.has(k) ? store.get(k) : null),
  setItem: (k, v) => store.set(k, String(v)),
  removeItem: (k) => store.delete(k),
});
stub('navigator', { language: 'it-IT' });
stub('window', { dispatchEvent: () => {} });

const localizeNodes = [];
stub('document', {
  documentElement: { lang: '', dir: '', setAttribute() {} },
  querySelectorAll: () => localizeNodes,
});

const catalogs = {
  '/i18n/index.json': { locales: [{ code: 'en', name: 'English' }, { code: 'it', name: 'Italiano' }] },
  '/i18n/en.json': {
    'k.text': 'Hello {name}',
    'k.onlyEn': 'Only English',
    'k.count.one': '{count} item',
    'k.count.other': '{count} items',
    'k.static': 'Static',
  },
  '/i18n/it.json': {
    'k.text': 'Ciao {name}',
    'k.count.one': '{count} oggetto',
    'k.count.other': '{count} oggetti',
    'k.static': 'Statico',
  },
};
globalThis.fetch = async (url) => {
  const key = String(url);
  if (!(key in catalogs)) {
    // A missing catalog is served as index.html with HTTP 200 (ServeDir
    // fallback) — the runtime must reject it, not crash.
    return { ok: true, text: async () => '<!DOCTYPE html><html></html>' };
  }
  return { ok: true, text: async () => JSON.stringify(catalogs[key]) };
};

const {
  initI18n, t, tp, localize, getLocale, getLocalePreference, setLocalePreference,
} = await import('../i18n.js');

/* ── runtime ─────────────────────────────────────────────────── */

await initI18n();
check('locale follows the browser language', getLocale() === 'it', getLocale());
check('a translated key is used', t('k.text', { name: 'Ada' }) === 'Ciao Ada', t('k.text', { name: 'Ada' }));
check('a key missing from the locale falls back to English', t('k.onlyEn') === 'Only English');
check('an unknown key returns the caller fallback', t('nope.key', null, 'Fallback') === 'Fallback');
check('an unknown key without fallback returns the key', t('nope.key') === 'nope.key');
check('an unfilled placeholder is kept', t('k.text') === 'Ciao {name}', t('k.text'));

check('tp picks the singular form', tp('k.count', 1) === '1 oggetto', tp('k.count', 1));
// The grouping depends on the ICU data the runtime ships, so compare against
// the platform's own formatter instead of a hardcoded string.
const grouped = new Intl.NumberFormat('it').format(1000);
check(
  'tp picks the plural form and formats the number',
  tp('k.count', 1000) === `${grouped} oggetti`,
  tp('k.count', 1000),
);

await setLocalePreference('en');
check('an explicit preference applies immediately', getLocale() === 'en', getLocale());
check('the preference is stored', getLocalePreference() === 'en');
check('the language switches catalogs', t('k.text', { name: 'Ada' }) === 'Hello Ada', t('k.text', { name: 'Ada' }));
check('fallback key missing in en returns the key', t('k.completely.missing') === 'k.completely.missing');

await setLocalePreference('');
check('empty preference returns to the browser language', getLocale() === 'it', getLocale());

function node(attributes, text) {
  return {
    attrs: { ...attributes },
    textContent: text,
    getAttribute(name) { return name in this.attrs ? this.attrs[name] : null; },
    setAttribute(name, value) { this.attrs[name] = value; },
  };
}
const translated = node({ 'data-i18n': 'k.static' }, 'Static');
const labelled = node({ 'data-i18n-aria-label': 'k.onlyEn' }, null);
const untouched = node({ 'data-i18n': 'k.not.translated' }, 'English stays');
localizeNodes.push(translated, labelled, untouched);
localize(document);
check('localize() replaces static text', translated.textContent === 'Statico', translated.textContent);
check('localize() replaces attributes', labelled.attrs['aria-label'] === 'Only English');
check('localize() leaves unknown keys as English', untouched.textContent === 'English stays');

/* ── catalog / key consistency ───────────────────────────────── */

const en = JSON.parse(readFileSync(join(root, 'web/i18n/en.json'), 'utf8'));
const enKeys = new Set(Object.keys(en).filter((k) => !k.startsWith('_')));

function walk(dir, out = []) {
  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry);
    if (statSync(path).isDirectory()) {
      if (entry === 'vendor' || entry === 'tests') continue;
      walk(path, out);
    } else if (extname(path) === '.js') {
      out.push(path);
    }
  }
  return out;
}

const used = new Map();   // key -> first file seen in
const plural = new Map(); // plural base key -> first file seen in (tp())
const dynamic = [];       // calls the checker cannot resolve
const dynamicPrefixes = []; // `t(`prefix.${…}`)` — whole namespaces, reported as used

for (const file of walk(join(root, 'web'))) {
  const source = readFileSync(file, 'utf8');
  const rel = file.slice(root.length);
  for (const match of source.matchAll(/(?:^|[^\w.$])tp\(\s*'([^']+)'/g)) {
    if (!plural.has(match[1])) plural.set(match[1], rel);
  }
  for (const match of source.matchAll(/(?:^|[^\w.$])t\(\s*'([^']+)'/g)) {
    if (!used.has(match[1])) used.set(match[1], rel);
  }
  // Table-driven keys: `{ labelKey: 'power.restart' }` — the call site is
  // `t(action.labelKey)`, which the scan above cannot see.
  for (const match of source.matchAll(/[A-Za-z]+Key:\s*'([^']+)'/g)) {
    if (!used.has(match[1])) used.set(match[1], rel);
  }
  for (const match of source.matchAll(/(?:^|[^\w.$])(?:t|tp)\(\s*`([^`]*?)\$\{/g)) {
    dynamic.push(`${rel} (line ${source.slice(0, match.index).split('\n').length})`);
    dynamicPrefixes.push(match[1]);
  }
}

const html = readFileSync(join(root, 'web/index.html'), 'utf8');
for (const match of html.matchAll(/data-i18n(?:-title|-placeholder|-aria-label)?="([^"]+)"/g)) {
  if (!used.has(match[1])) used.set(match[1], 'web/index.html');
}

const missing = [...used].filter(([key]) => !enKeys.has(key) && !key.startsWith('plugin.'));
check(
  `every key used in the tree exists in en.json (${used.size} checked)`,
  missing.length === 0,
  missing.map(([key, file]) => `${key} (${file})`).join(', '),
);

const noPluralForm = [...plural].filter(([key]) => (
  !enKeys.has(`${key}.one`) && !enKeys.has(`${key}.other`) && !enKeys.has(key)
));
check(
  `every tp() key has a plural form in en.json (${plural.size} checked)`,
  noPluralForm.length === 0,
  noPluralForm.map(([key, file]) => `${key} (${file})`).join(', '),
);

// A plural base key owns its `<base>.one` / `<base>.other` variants, and a
// dynamic call site owns its whole `prefix.` namespace.
const owned = (key) => (
  plural.has(key)
  || [...plural.keys()].some((base) => key === `${base}.one` || key === `${base}.other`)
  || dynamicPrefixes.some((prefix) => key.startsWith(prefix))
);
const unused = [...enKeys].filter((key) => !used.has(key) && !owned(key));
if (unused.length) console.log(`\n  note: ${unused.length} en.json key(s) not used in web/ (fine mid-extraction):\n    ${unused.join('\n    ')}`);

/* Every translation is checked against the reference catalogue: no invented
 * keys, `{placeholder}` / `%1` survival, and the plural forms the language
 * actually uses (`Intl.PluralRules` decides which those are — Polish needs
 * `.few` and `.many`, Latvian `.zero`, Irish `.two`, …). Missing keys are only
 * reported: a partial catalogue falls back to English on purpose. Placeholders
 * and a missing `.other` are hard failures — they break rendering. */

const placeholders = (text) => new Set([...String(text).matchAll(/\{[^}\s]+\}|%\d+/g)].map((m) => m[0]));

/** The plural categories a language can select (exact CLDR set). */
function pluralCategories(code) {
  return new Set(new Intl.PluralRules(code).resolvedOptions().pluralCategories);
}

function checkCatalog(label, code, locale, reference) {
  const referenceKeys = new Set(Object.keys(reference).filter((k) => !k.startsWith('_')));
  const categories = pluralCategories(code);
  // A locale may carry its own plural categories beyond the reference's
  // `.one`/`.other` (`pl.few`, `lv.zero`, …) — but only for a plural base the
  // reference defines, and only a category the language actually selects.
  const extra = Object.keys(locale).filter((k) => {
    if (k.startsWith('_') || referenceKeys.has(k)) return false;
    const match = k.match(/^(.+)\.(zero|one|two|few|many|other)$/);
    return !(
      match
      && (referenceKeys.has(`${match[1]}.one`) || referenceKeys.has(`${match[1]}.other`))
      && categories.has(match[2])
    );
  });
  check(`${label}: every key exists in en.json`, extra.length === 0, extra.join(', '));

  const missing = [...referenceKeys].filter((k) => !(k in locale));
  if (missing.length) {
    console.log(`  note: ${label} is missing ${missing.length}/${referenceKeys.size} key(s) — English shows through`);
  }

  const badPlaceholders = [];
  for (const [key, value] of Object.entries(locale)) {
    if (!referenceKeys.has(key) || typeof value !== 'string') continue;
    const want = placeholders(reference[key]);
    const got = placeholders(value);
    const lost = [...want].filter((t) => !got.has(t));
    const invented = [...got].filter((t) => !want.has(t));
    if (lost.length || invented.length) {
      badPlaceholders.push(
        `${key} (${[lost.length ? `lost ${lost.join(' ')}` : '', invented.length ? `invented ${invented.join(' ')}` : ''].filter(Boolean).join(', ')})`,
      );
    }
  }
  check(
    `${label}: {placeholders} preserved (${Object.keys(locale).length} checked)`,
    badPlaceholders.length === 0,
    badPlaceholders.slice(0, 8).join(', '),
  );

  const bases = new Set();
  for (const key of referenceKeys) {
    const match = key.match(/^(.+)\.(one|other)$/);
    if (match) bases.add(match[1]);
  }
  const withoutOther = [];
  const incomplete = [];
  for (const base of bases) {
    const present = [...categories].filter((c) => `${base}.${c}` in locale);
    if (!present.length) continue; // untranslated plural set — covered by the missing-keys note
    if (!(`${base}.other` in locale)) withoutOther.push(base);
    const absent = [...categories].filter((c) => !(`${base}.${c}` in locale));
    if (absent.length) incomplete.push(`${base}: no ${absent.join('/')}`);
  }
  check(
    `${label}: .other present for every translated plural (${bases.size} base(s))`,
    withoutOther.length === 0,
    withoutOther.slice(0, 8).join(', '),
  );
  if (incomplete.length) {
    console.log(`  note: ${label} is missing ${code} plural form(s) — falls back to .other:\n    ${incomplete.slice(0, 10).join('\n    ')}`);
  }
}

for (const file of readdirSync(join(root, 'web/i18n'))) {
  // `_`-prefixed files are in-flight staging artifacts, not shipped locales.
  if (!file.endsWith('.json') || file === 'en.json' || file === 'index.json' || file.startsWith('_')) continue;
  const code = file.replace(/\.json$/, '');
  const other = JSON.parse(readFileSync(join(root, 'web/i18n', file), 'utf8'));
  checkCatalog(file, code, other, en);
}

const index = JSON.parse(readFileSync(join(root, 'web/i18n/index.json'), 'utf8'));
const offered = new Set((index.locales || []).map((l) => l.code));
const shipped = readdirSync(join(root, 'web/i18n'))
  .filter((f) => f.endsWith('.json') && f !== 'index.json' && !f.startsWith('_'))
  .map((f) => f.replace(/\.json$/, ''));
check(
  'every shipped catalogue is offered in index.json',
  shipped.every((c) => offered.has(c)),
  shipped.filter((c) => !offered.has(c)).join(', '),
);
check(
  'every index.json locale has a shipped catalogue',
  [...offered].every((c) => shipped.includes(c)),
  [...offered].filter((c) => !shipped.includes(c)).join(', '),
);

if (dynamic.length) console.log(`\n  note: ${dynamic.length} dynamic t() key(s) the checker cannot resolve (their namespaces count as used):\n    ${dynamic.join('\n    ')}`);

/* ── plugin catalogues ───────────────────────────────────────── */

/** `.js` files under a plugin's web dir, minus generated wasm glue. */
function pluginJs(dir) {
  const out = [];
  const walkDir = (d) => {
    for (const entry of readdirSync(d)) {
      const path = join(d, entry);
      if (statSync(path).isDirectory()) {
        walkDir(path);
      } else if (extname(path) === '.js' && !/filmcraft_web|\.min\./.test(entry)) {
        out.push(path);
      }
    }
  };
  walkDir(dir);
  return out;
}

const pluginMissing = [];
const pluginExtra = [];
let pluginChecked = 0;
for (const name of readdirSync(join(root, 'plugins'))) {
  const webDir = join(root, 'plugins', name, 'web');
  if (!existsSync(webDir)) continue;
  const catalogPath = join(webDir, 'i18n', 'en.json');
  const catalog = existsSync(catalogPath)
    ? JSON.parse(readFileSync(catalogPath, 'utf8'))
    : {};

  const keys = new Set();
  const dynamicPrefixes = [];
  for (const file of pluginJs(webDir)) {
    const source = readFileSync(file, 'utf8');
    const patterns = [
      /(?:^|[^\w.$])t\(\s*'([^']+)'/g,
      /(?:^|[^\w.$])tp\(\s*'([^']+)'/g,
      /[A-Za-z]+Key:\s*'([^']+)'/g,
    ];
    for (const pattern of patterns) {
      for (const match of source.matchAll(pattern)) {
        if (match[1].startsWith('plugin.')) keys.add(match[1]);
      }
    }
    for (const match of source.matchAll(/(?:^|[^\w.$])(?:t|tp)\(\s*`([^`]*?)\$\{/g)) {
      dynamicPrefixes.push(match[1]);
    }
  }
  if (!keys.size && !Object.keys(catalog).length) continue;

  // A plugin's translations live in its own folder; they are checked against
  // the plugin's reference catalogue the same way the core ones are.
  const i18nDir = join(webDir, 'i18n');
  for (const file of existsSync(i18nDir) ? readdirSync(i18nDir) : []) {
    if (!file.endsWith('.json') || file === 'en.json' || file.startsWith('_')) continue;
    const locale = JSON.parse(readFileSync(join(i18nDir, file), 'utf8'));
    checkCatalog(`plugin ${name}/${file}`, file.replace(/\.json$/, ''), locale, catalog);
  }

  pluginChecked += keys.size;
  for (const key of keys) {
    const known = key in catalog || `${key}.one` in catalog || `${key}.other` in catalog;
    if (!known) pluginMissing.push(`${name}: ${key}`);
  }
  for (const key of Object.keys(catalog)) {
    // `meta.*` is consumed server-side (the Plugins window reads it through
    // /api/plugins), so it has no client call site to match.
    if (key.includes('.meta.')) continue;
    if (dynamicPrefixes.some((prefix) => key.startsWith(prefix))) continue;
    if (!keys.has(key) && !keys.has(key.replace(/\.(one|other)$/, ''))) {
      pluginExtra.push(`${name}: ${key}`);
    }
  }
}
check(
  `every plugin key exists in its own catalogue (${pluginChecked} checked)`,
  pluginMissing.length === 0,
  pluginMissing.slice(0, 12).join(', '),
);
if (pluginExtra.length) {
  console.log(`\n  note: ${pluginExtra.length} plugin catalogue key(s) unused by their JS (fine mid-extraction):\n    ${pluginExtra.slice(0, 20).join('\n    ')}`);
}

if (failures) {
  console.error(`\n${failures} check(s) failed`);
  process.exit(1);
}

console.log('\nall i18n checks passed');
