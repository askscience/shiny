/**
 * i18n harness for the pure-unit tests.
 *
 * Installs the few browser globals the runtime touches, serves the real
 * `web/i18n/*.json` catalogs from disk, and initializes it — so a module under
 * test that calls `t()`/`tp()` resolves exactly the English the app shows, and
 * a key that disappears from en.json fails the test instead of silently
 * rendering the key.
 *
 *   import '../helpers/i18n.mjs';   // first, before the module under test
 */
import { readFileSync, readdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../../../../', import.meta.url));

const catalogs = new Map();
for (const file of readdirSync(`${root}web/i18n`)) {
  if (!file.endsWith('.json')) continue;
  catalogs.set(`/i18n/${file}`, JSON.parse(readFileSync(`${root}web/i18n/${file}`, 'utf8')));
}

// Node ≥ 22 defines some of these as getter-only globals; define over them.
function stub(name, value) {
  Object.defineProperty(globalThis, name, { value, configurable: true, writable: true });
}

const store = new Map();
stub('localStorage', {
  getItem: (k) => (store.has(k) ? store.get(k) : null),
  setItem: (k, v) => store.set(k, String(v)),
  removeItem: (k) => store.delete(k),
});
stub('navigator', { language: 'en' });
stub('window', { dispatchEvent: () => {} });
stub('document', {
  documentElement: { lang: '', dir: '', setAttribute() {} },
  querySelectorAll: () => [],
});
stub('fetch', async (url) => {
  const key = String(url);
  if (!catalogs.has(key)) return { ok: true, text: async () => '<!DOCTYPE html>' };
  return { ok: true, text: async () => JSON.stringify(catalogs.get(key)) };
});

const i18n = await import('../../i18n.js');
await i18n.initI18n();

export const { t, tp } = i18n;
export default i18n;
