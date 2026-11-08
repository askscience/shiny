/**
 * The accent across a light/dark switch (web/ui/appearance.js).
 *
 * The accent is stored once per user and used as ink. A colour that would
 * vanish on the other canvas falls back, but a neutral one keeps its intent:
 * White picked on Ink becomes Dusk's Ink, not Dusk's orange default. A
 * coloured accent passes through, and the stored choice is never rewritten.
 *
 * Run:  node web/js/tests/accentAcrossThemes.test.mjs
 */
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

let failures = 0;
function check(name, condition, detail = '') {
  if (condition) {
    console.log(`  ok  ${name}`);
  } else {
    failures += 1;
    console.error(`FAIL  ${name}${detail ? ` — ${detail}` : ''}`);
  }
}

const here = dirname(fileURLToPath(import.meta.url));
const manifestOf = (name) => JSON.parse(readFileSync(join(here, `../../themes/${name}/theme.json`), 'utf8'));
const themes = { dusk: manifestOf('dusk'), ink: manifestOf('ink') };

const store = new Map();
globalThis.localStorage = {
  getItem: (k) => (store.has(k) ? store.get(k) : null),
  setItem: (k, v) => store.set(k, String(v)),
  removeItem: (k) => store.delete(k),
};
globalThis.window = { addEventListener() {}, dispatchEvent() {} };
globalThis.document = { documentElement: { style: {}, dataset: {} }, getElementById: () => null };
globalThis.fetch = async (url) => {
  if (String(url).endsWith('/themes/themes.json')) return { ok: true, json: async () => Object.keys(themes) };
  const name = String(url).match(/themes\/([^/]+)\/theme\.json/)?.[1];
  return { ok: !!themes[name], json: async () => themes[name] };
};

const loader = await import('../../ui/theme-loader.js');
const { getAccent, setAccent } = await import('../../ui/appearance.js');

async function on(theme) {
  const ok = await loader.setTheme(theme);
  if (!ok) throw new Error(`could not switch to ${theme}`);
}

await on('ink');
setAccent('#f2f2f0');
check('White shows on Ink', getAccent() === '#f2f2f0', getAccent());
await on('dusk');
check('White on Dusk becomes Dusk\'s Ink, not the orange default', getAccent() === '#0b0b0c', getAccent());
check('the stored choice is kept', localStorage.getItem('ui.accent') === '#f2f2f0');
await on('ink');
check('back on Ink it is White again', getAccent() === '#f2f2f0', getAccent());

setAccent('#0b0b0c');
check('Ink picked on Dusk becomes Ink\'s White on Ink', getAccent() === '#f2f2f0', getAccent());

setAccent('#8b5cf6');
await on('dusk');
check('a coloured accent passes through on Dusk', getAccent() === '#8b5cf6', getAccent());
await on('ink');
check('and on Ink', getAccent() === '#8b5cf6', getAccent());

setAccent('#fff3a0');
await on('dusk');
check('a pale colour on Dusk still falls back to the default', getAccent() === themes.dusk.defaultAccent, getAccent());

if (failures) {
  console.error(`\n${failures} accent check(s) failed`);
  process.exit(1);
}
console.log('\nall accent checks passed');
