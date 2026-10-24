/**
 * Pure-unit checks for hudChipsShared.js. Run:
 *   node web/js/tests/hudChipsShared.test.mjs
 */

import {
  DEFAULT_HUD_CHIPS, HUD_CHIP_PARTS, hudChipsDataset, isIconOnly, normalizeHudChips,
} from '../hudChipsShared.js';

let failures = 0;
function check(name, condition) {
  if (condition) {
    console.log(`  ok  ${name}`);
  } else {
    failures += 1;
    console.error(`FAIL  ${name}`);
  }
}

check('parts are name + percent', HUD_CHIP_PARTS.join(',') === 'name,percent');
check('default hides names and percentages',
  DEFAULT_HUD_CHIPS.name === false && DEFAULT_HUD_CHIPS.percent === false);

check('missing value → icon only',
  normalizeHudChips(undefined).name === false
  && normalizeHudChips(undefined).percent === false);
check('null value → icon only',
  normalizeHudChips(null).name === false && normalizeHudChips(null).percent === false);
check('array value → name off', normalizeHudChips([]).name === false);
check('string value → icon only',
  normalizeHudChips('nope').name === false && normalizeHudChips('nope').percent === false);
check('name on kept', normalizeHudChips({ name: true }).name === true);
check('name off kept', normalizeHudChips({ name: false }).name === false);
check('percent off kept', normalizeHudChips({ percent: false }).percent === false);
check('empty object → icon only',
  normalizeHudChips({}).name === false && normalizeHudChips({}).percent === false);

check('icon only when both off', isIconOnly({ name: false, percent: false }) === true);
check('not icon only with names on', isIconOnly({ name: true, percent: false }) === false);
check('not icon only with percents on', isIconOnly({ name: false, percent: true }) === false);
check('not icon only when both on', isIconOnly({ name: true, percent: true }) === false);
check('default is icon only', isIconOnly(undefined) === true);

check('dataset default → icon only',
  hudChipsDataset({}).hudName === '0' && hudChipsDataset({}).hudPercent === '0');
check('dataset both on', hudChipsDataset({ name: true, percent: true }).hudName === '1'
  && hudChipsDataset({ name: true, percent: true }).hudPercent === '1');
check('dataset both off', hudChipsDataset({ name: false, percent: false }).hudName === '0'
  && hudChipsDataset({ name: false, percent: false }).hudPercent === '0');
check('dataset names on only', hudChipsDataset({ name: true, percent: false }).hudName === '1'
  && hudChipsDataset({ name: true, percent: false }).hudPercent === '0');

if (failures) {
  console.error(`\n${failures} check(s) failed`);
  process.exit(1);
}
console.log('\nall hudChipsShared checks passed');
