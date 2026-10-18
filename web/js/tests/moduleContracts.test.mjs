// Public ES module entrypoints must retain their existing exports even when
// implementations move. Imports stay relative so the kiosk can serve them.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
const contracts = JSON.parse(readFileSync(new URL('../../../tests/fixtures/ui-contracts.json', import.meta.url)));
function exportsAt(url) {
  const source = readFileSync(url, 'utf8');
  const names = [...source.matchAll(/^export\s+(?:async\s+)?(?:function|const|class)\s+(\w+)/gm)].map((m) => m[1]);
  if (/^export default/m.test(source)) names.push('default');
  for (const match of source.matchAll(/^export \* from '([^']+)'/gm)) {
    names.push(...exportsAt(new URL(match[1], url)).filter((name) => name !== 'default'));
  }
  return names;
}
for (const [path, required] of Object.entries(contracts)) {
  const actual = new Set(exportsAt(new URL(`../${path}`, import.meta.url)));
  for (const name of required) assert.ok(actual.has(name), `${path} still exports ${name}`);
}
console.log('Public ES module contract checks passed');
