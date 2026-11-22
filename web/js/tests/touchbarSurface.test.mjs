/**
 * Touch Bar surface: the arithmetic and the data rules shared by the bar page
 * (`touchbarSurface/bar.js`) and the desktop page (`touchbarHost.js`).
 *
 * Run:  node web/js/tests/touchbarSurface.test.mjs
 */
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

import {
  CHANNEL, FN_KEYS, backlightFor, clamp01, flingStep, normalizeContextItems,
  normalizeTouchBarDock, scrollLimit, sliderValue,
} from '../touchbarSurface/barShared.js';

let failures = 0;
const check = (name, condition, detail = '') => {
  console.log(`  ${condition ? 'ok  ' : 'FAIL'} ${name}${detail ? ` — ${detail}` : ''}`);
  if (!condition) failures += 1;
};
const read = (relative) => readFileSync(fileURLToPath(new URL(relative, import.meta.url)), 'utf8');

console.log('touch bar surface');

check('twelve function keys', FN_KEYS.length === 12 && FN_KEYS[0] === 1 && FN_KEYS[11] === 12);
check('clamp01 bounds a value', clamp01(-1) === 0 && clamp01(2) === 1 && clamp01(0.4) === 0.4);
check('clamp01 of junk is 0', clamp01('x') === 0 && clamp01(undefined) === 0);

check('a slider moves relative to where the finger landed', sliderValue(0.4, 150, 300) === 0.9);
check('a slider stops at its ends', sliderValue(0.9, 300, 300) === 1 && sliderValue(0.1, -300, 300) === 0);
check('a slider with no travel keeps its value', sliderValue(0.3, 50, 0) === 0.3);

check('content that fits does not scroll', scrollLimit(500, 800) === 0);
check('longer content scrolls by the overflow', scrollLimit(1300, 800) === 500);

const step = flingStep(1, 16);
check('a fling moves by velocity x time', step.distance === 16);
check('a fling slows down', step.velocity < 1 && step.velocity > 0.9);
check('a slow fling ends', flingStep(0.01, 16).done === true);
let v = 2;
let frames = 0;
while (!flingStep(v, 16).done && frames < 1000) { v = flingStep(v, 16).velocity; frames += 1; }
check('a fling always comes to rest', frames < 200, `${frames} frames`);
check('a long frame is capped', flingStep(1, 5000).distance === 64);

check('a bar in use is lit', backlightFor(0) === 2 && backlightFor(59_000) === 2);
check('an idle bar dims', backlightFor(60_000) === 1);
check('a long idle bar goes dark', backlightFor(240_000) === 0);

check('the dock setting is a strict on', normalizeTouchBarDock('1') && !normalizeTouchBarDock('0') && !normalizeTouchBarDock(null));

const noop = () => {};
const items = normalizeContextItems([
  { label: 'Refresh', icon: 'ui/refresh', onClick: noop },
  { type: 'separator' },
  { label: 'No handler' },
  { id: 'a', label: 'A', checked: true, onClick: noop },
  { id: 'a', label: 'Duplicate', onClick: noop },
  null,
  { label: 'x'.repeat(100), onClick: noop },
]);
check('entries without a label or a handler are dropped', items.length === 3, items.map((i) => i.id).join(','));
check('an entry is identified by its label by default', items[0].id === 'Refresh' && items[0].icon === 'ui/refresh');
check('checked means active', items[1].active === true);
check('labels are bounded', items[2].label.length === 40);
check('the list is bounded', normalizeContextItems(Array.from({ length: 30 }, (_, i) => ({ label: `i${i}`, onClick: noop }))).length === 12);
check('junk is an empty list', normalizeContextItems('nope').length === 0);

// The three sides of the surface must agree on the names they share.
const barJs = read('../touchbarSurface/bar.js');
const hostJs = read('../touchbarHost.js');
const shell = read('../../../crates/liteaura/src/touchbar.rs');
const main = read('../../../crates/liteaura/src/main.rs');
check('one channel name', CHANNEL === 'shiny.touchbar' && barJs.includes('CHANNEL') && hostJs.includes('CHANNEL'));
check('the page defines what the shell calls',
  ['__shinyBarTouch', '__shinyBarFn'].every((name) => barJs.includes(`window.${name} =`) && shell.includes(`window.${name}`)));
check('every message the bar sends is handled by the desktop',
  [...barJs.matchAll(/send\(\{ t: '([a-z]+)'/g)].every((m) => hostJs.includes(`case '${m[1]}'`)),
  [...new Set([...barJs.matchAll(/send\(\{ t: '([a-z]+)'/g)].map((m) => m[1]))].join(' '));
check('every message the desktop posts is handled by the bar',
  [...hostJs.matchAll(/post\(\{ t: '([a-z]+)'/g)].every((m) => barJs.includes(`data.t === '${m[1]}'`)),
  [...new Set([...hostJs.matchAll(/post\(\{ t: '([a-z]+)'/g)].map((m) => m[1]))].join(' '));
check('the shell understands what the desktop asks of it',
  ['backlight', 'fkey'].every((key) => hostJs.includes(`shell({ ${key}`) && main.includes(`"${key}"`)));
check('the page is where the shell looks for it', main.includes('/touchbar.html'));

if (failures) {
  console.error(`\n${failures} failure(s)`);
  process.exit(1);
}
console.log('\nall touch bar surface checks passed');
