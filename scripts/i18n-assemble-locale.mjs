/**
 * Assemble a locale catalogue from its per-part staging files and validate it.
 *
 * A translation pass writes its work as several standalone JSON parts
 * (`web/i18n/_parts-<code>-*.json` — one writer per file, so parallel passes
 * never touch the same document); this script merges them into
 * `web/i18n/<code>.json` in the reference catalogue's key order, checks the
 * result, and deletes the parts. Existing values in `<code>.json` are kept for
 * keys no part provides, so re-assembling after a proofreading edit never
 * reverts it.
 *
 *   node scripts/i18n-assemble-locale.mjs <code>
 *
 * Hard failures (nothing is written): a key that is not in en.json, a
 * non-string value, a key provided by two parts, a `{placeholder}` or `%N`
 * mismatch, or a missing plural category the language actually uses
 * (`Intl.PluralRules` decides which those are). Keys still missing from the
 * locale are reported as a note — English shows through for them.
 */
import { readFileSync, writeFileSync, unlinkSync, readdirSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const dir = join(root, 'web/i18n');

const code = (process.argv[2] || '').trim().toLowerCase();
if (!/^[a-z]{2}(-[a-z]{2})?$/.test(code)) {
  console.error('usage: node scripts/i18n-assemble-locale.mjs <code>   (e.g. de, pt-BR)');
  process.exit(1);
}

const en = JSON.parse(readFileSync(join(dir, 'en.json'), 'utf8'));
const enKeys = Object.keys(en).filter((k) => !k.startsWith('_'));
const enKeySet = new Set(enKeys);

/* The plural bases the reference defines, and the CLDR categories this
 * language selects. A catalogue may carry its own categories beyond the
 * reference's `.one`/`.other` (`pl.few`, `lv.zero`, `ga.two`, …) — always for
 * a base en.json defines, and only a category the language actually uses. */
const pluralBases = new Set();
for (const key of enKeys) {
  const match = key.match(/^(.+)\.(one|other)$/);
  if (match) pluralBases.add(match[1]);
}
const categories = new Set(new Intl.PluralRules(code).resolvedOptions().pluralCategories);
const CLDR_ORDER = ['zero', 'one', 'two', 'few', 'many', 'other'];

/** A key the catalogue may hold: a reference key, or a plural form the language selects. */
function isAllowed(key) {
  if (enKeySet.has(key)) return true;
  const match = key.match(/^(.+)\.([a-z]+)$/);
  return Boolean(match) && pluralBases.has(match[1]) && categories.has(match[2]);
}

/* ── collect the parts (plus anything already in the target) ───── */

const parts = readdirSync(dir)
  .filter((f) => f.startsWith(`_parts-${code}-`) && f.endsWith('.json'))
  .sort((a, b) => a.localeCompare(b, 'en', { numeric: true }));

const target = join(dir, `${code}.json`);
const values = new Map();
const from = new Map(); // key -> source, for duplicate reporting
let kept = 0;

if (existsSync(target)) {
  const existing = JSON.parse(readFileSync(target, 'utf8'));
  for (const [key, value] of Object.entries(existing)) {
    if (typeof value === 'string') values.set(key, value);
  }
  kept = values.size;
}

if (!parts.length) {
  console.error(`no _parts-${code}-*.json files in web/i18n — nothing to assemble`);
  process.exit(1);
}

const errors = [];
for (const file of parts) {
  let data;
  try {
    data = JSON.parse(readFileSync(join(dir, file), 'utf8'));
  } catch (err) {
    errors.push(`${file}: not valid JSON — ${err.message}`);
    continue;
  }
  for (const [key, value] of Object.entries(data)) {
    if (key.startsWith('_')) continue;
    if (!isAllowed(key)) {
      errors.push(`${file}: "${key}" is neither a key in en.json nor a plural form ${code} uses`);
      continue;
    }
    if (typeof value !== 'string') {
      errors.push(`${file}: "${key}" is not a string`);
      continue;
    }
    if (from.has(key) && from.get(key) !== file) {
      errors.push(`"${key}" appears in both ${from.get(key)} and ${file}`);
      continue;
    }
    values.set(key, value);
    from.set(key, file);
  }
}

/* ── placeholders must survive translation ─────────────────────── */

const tokens = (text) => new Set([...String(text).matchAll(/\{[^}\s]+\}|%\d+/g)].map((m) => m[0]));

for (const [key, value] of values) {
  // An extra plural category is checked against its base's English `.other`.
  const match = key.match(/^(.+)\.([a-z]+)$/);
  const reference = enKeySet.has(key)
    ? en[key]
    : (match && pluralBases.has(match[1]) ? en[`${match[1]}.other`] : undefined);
  if (reference === undefined) continue;
  const want = tokens(reference);
  const got = tokens(value);
  const lost = [...want].filter((t) => !got.has(t));
  const added = [...got].filter((t) => !want.has(t));
  if (lost.length || added.length) {
    const detail = [lost.length ? `missing ${lost.join(' ')}` : '', added.length ? `invented ${added.join(' ')}` : '']
      .filter(Boolean)
      .join(', ');
    errors.push(`"${key}": placeholder mismatch — ${detail}`);
  }
}

/* ── the plural categories this language actually uses ─────────── */

// A category the language never selects can never be rendered — drop a stray
// copy (e.g. an English-inherited `.few`) instead of fossilizing it.
let pruned = 0;
for (const key of [...values.keys()]) {
  const match = key.match(/^(.+)\.([a-z]+)$/);
  if (match && pluralBases.has(match[1]) && !categories.has(match[2])) {
    values.delete(key);
    pruned += 1;
  }
}

for (const base of pluralBases) {
  // The whole plural set untranslated is reported below as missing keys.
  if (![...categories].some((c) => values.has(`${base}.${c}`))) continue;
  const missing = [...categories].filter((c) => !values.has(`${base}.${c}`));
  if (missing.length) {
    errors.push(`"${base}": missing ${code} plural form(s) ${missing.map((c) => `${base}.${c}`).join(', ')}`);
  }
}

/* ── verdict ───────────────────────────────────────────────────── */

const missingKeys = enKeys.filter((k) => !values.has(k));

if (errors.length) {
  console.error(`${errors.length} problem(s) — nothing written:\n`);
  for (const error of errors.slice(0, 40)) console.error(`  ${error}`);
  if (errors.length > 40) console.error(`  … and ${errors.length - 40} more`);
  process.exit(1);
}

/* Serialize in the reference's group order, blank line between groups, with
 * each base's extra plural categories right after its `.other` — the same
 * shape scripts/i18n-merge-catalogs.mjs gives en.json. */
const extras = new Map(); // base -> [category]
for (const key of values.keys()) {
  const match = key.match(/^(.+)\.([a-z]+)$/);
  if (!match || enKeySet.has(key) || !pluralBases.has(match[1])) continue;
  if (!extras.has(match[1])) extras.set(match[1], []);
  extras.get(match[1]).push(match[2]);
}
for (const list of extras.values()) {
  list.sort((a, b) => CLDR_ORDER.indexOf(a) - CLDR_ORDER.indexOf(b));
}

const orderedKeys = [];
for (const key of enKeys) {
  if (values.has(key)) orderedKeys.push(key);
  const match = key.match(/^(.+)\.other$/);
  if (match && extras.has(match[1])) {
    for (const category of extras.get(match[1])) orderedKeys.push(`${match[1]}.${category}`);
    extras.delete(match[1]);
  }
}
for (const [base, list] of extras) {
  for (const category of list) orderedKeys.push(`${base}.${category}`);
}

const groups = new Map();
for (const key of orderedKeys) {
  const group = key.split('.')[0];
  if (!groups.has(group)) groups.set(group, []);
  groups.get(group).push([key, values.get(key)]);
}
const body = [...groups.values()]
  .map((rows) => rows.map(([k, v]) => `  ${JSON.stringify(k)}: ${JSON.stringify(v)}`).join(',\n'))
  .join(',\n\n');
writeFileSync(target, `{\n${body}\n}\n`);

for (const file of parts) unlinkSync(join(dir, file));

console.log(
  `assembled web/i18n/${code}.json — ${values.size}/${enKeys.length} key(s)`
  + (kept ? ` (${kept} kept from the existing file)` : '')
  + (pruned ? `, ${pruned} unused plural categor${pruned === 1 ? 'y' : 'ies'} dropped` : '')
  + `, ${pluralBases.size} plural base(s), ${parts.length} part(s) merged`,
);
if (missingKeys.length) {
  console.log(`  note: ${missingKeys.length} key(s) still missing — English shows through for them`);
  console.log(`    ${missingKeys.slice(0, 25).join('\n    ')}${missingKeys.length > 25 ? '\n    …' : ''}`);
}
