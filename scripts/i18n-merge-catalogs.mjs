/**
 * Merge the per-surface staging catalogues into the reference catalogue.
 *
 * While the tree is being translated, parallel passes write their keys to
 * `web/i18n/_staging-<surface>.json` (one writer per file, so no two passes
 * ever touch the same document). This merges them into `web/i18n/en.json`,
 * preserving the existing group order, and deletes the staging files.
 *
 *   node scripts/i18n-merge-catalogs.mjs
 *
 * A key that already exists with a different English aborts the merge: that is
 * always a mistake (two surfaces inventing the same key) and has to be resolved
 * by hand. Run `node web/js/tests/i18n.test.mjs` afterwards.
 */
import { readFileSync, readdirSync, writeFileSync, unlinkSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const dir = join(root, 'web/i18n');
const target = join(dir, 'en.json');

const en = JSON.parse(readFileSync(target, 'utf8'));
const entries = new Map(Object.entries(en).filter(([k]) => !k.startsWith('_')));

const staging = readdirSync(dir).filter((f) => f.startsWith('_staging-') && f.endsWith('.json'));
if (!staging.length) {
  console.log('nothing to merge — no _staging-*.json files');
  process.exit(0);
}

let conflicts = 0;
let added = 0;
let identical = 0;
for (const file of staging) {
  const data = JSON.parse(readFileSync(join(dir, file), 'utf8'));
  for (const [key, value] of Object.entries(data)) {
    if (typeof value !== 'string') {
      console.error(`  bad value in ${file}: ${key} is not a string`);
      conflicts += 1;
      continue;
    }
    if (entries.has(key)) {
      if (entries.get(key) !== value) {
        console.error(`  conflict: ${key}\n    en.json: ${entries.get(key)}\n    ${file}: ${value}`);
        conflicts += 1;
      } else {
        identical += 1;
      }
      continue;
    }
    entries.set(key, value);
    added += 1;
  }
}

if (conflicts) {
  console.error(`\n${conflicts} conflict(s) — nothing written`);
  process.exit(1);
}

// Serialize with a blank line between prefix groups, keeping the order the
// groups already have in the file and appending new ones at the end.
const groups = new Map();
for (const [key, value] of entries) {
  const group = key.split('.')[0];
  if (!groups.has(group)) groups.set(group, []);
  groups.get(group).push([key, value]);
}
const body = [...groups.entries()]
  .map(([, rows]) => rows.map(([k, v]) => `  ${JSON.stringify(k)}: ${JSON.stringify(v)}`).join(',\n'))
  .join(',\n\n');
writeFileSync(target, `{\n${body}\n}\n`);

for (const file of staging) unlinkSync(join(dir, file));
console.log(`merged ${added} key(s) (${identical} already present) from ${staging.length} staging file(s)`);
