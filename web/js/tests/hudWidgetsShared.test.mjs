/**
 * Pure-unit checks for hudWidgetsShared.js. Run:
 *   node web/js/tests/hudWidgetsShared.test.mjs
 */

import {
  DEFAULT_HUD_WIDGETS, HUD_WIDGET_PARTS, hudWidgetsDataset, normalizeHudWidgets,
} from '../hudWidgetsShared.js';

let failures = 0;
function check(name, condition) {
  if (condition) {
    console.log(`  ok  ${name}`);
  } else {
    failures += 1;
    console.error(`FAIL  ${name}`);
  }
}

check('parts are clock date + weather text',
  HUD_WIDGET_PARTS.join(',') === 'clock_date,weather_text');
check('default shows both text parts',
  DEFAULT_HUD_WIDGETS.clock_date === true && DEFAULT_HUD_WIDGETS.weather_text === true);

check('missing value → both shown',
  normalizeHudWidgets(undefined).clock_date === true
  && normalizeHudWidgets(undefined).weather_text === true);
check('null value → both shown',
  normalizeHudWidgets(null).clock_date === true && normalizeHudWidgets(null).weather_text === true);
check('array value → both shown',
  normalizeHudWidgets([]).clock_date === true && normalizeHudWidgets([]).weather_text === true);
check('string value → both shown',
  normalizeHudWidgets('nope').clock_date === true && normalizeHudWidgets('nope').weather_text === true);
check('clock date off kept', normalizeHudWidgets({ clock_date: false }).clock_date === false);
check('weather text off kept', normalizeHudWidgets({ weather_text: false }).weather_text === false);
check('empty object → both shown',
  normalizeHudWidgets({}).clock_date === true && normalizeHudWidgets({}).weather_text === true);

check('dataset default → both shown',
  hudWidgetsDataset({}).hudClockDate === '1' && hudWidgetsDataset({}).hudWeatherText === '1');
check('dataset both off',
  hudWidgetsDataset({ clock_date: false, weather_text: false }).hudClockDate === '0'
  && hudWidgetsDataset({ clock_date: false, weather_text: false }).hudWeatherText === '0');
check('dataset clock date off only',
  hudWidgetsDataset({ clock_date: false }).hudClockDate === '0'
  && hudWidgetsDataset({ clock_date: false }).hudWeatherText === '1');
check('dataset weather text off only',
  hudWidgetsDataset({ weather_text: false }).hudClockDate === '1'
  && hudWidgetsDataset({ weather_text: false }).hudWeatherText === '0');

if (failures) {
  console.error(`\n${failures} check(s) failed`);
  process.exit(1);
}
console.log('\nall hudWidgetsShared checks passed');
