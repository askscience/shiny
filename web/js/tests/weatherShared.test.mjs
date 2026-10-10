/**
 * Pure-unit checks for weatherShared.js. Run:
 *   node web/js/tests/weatherShared.test.mjs
 */

import {
  forecastDayLabel, roundTemp, tempText, weatherIconStem, weatherLabel,
} from '../weatherShared.js';

let failures = 0;
function check(name, condition) {
  if (condition) {
    console.log(`  ok  ${name}`);
  } else {
    failures += 1;
    console.error(`FAIL  ${name}`);
  }
}

check('stem clear', weatherIconStem(0) === 'weather-sun');
check('stem partly cloudy', [1, 2, 3].every((c) => weatherIconStem(c) === 'weather-partly'));
check('stem fog', [45, 48].every((c) => weatherIconStem(c) === 'weather-fog'));
check('stem drizzle', [51, 53, 55].every((c) => weatherIconStem(c) === 'weather-drizzle'));
check('stem rain', [61, 63, 65, 80, 81, 82].every((c) => weatherIconStem(c) === 'weather-rain'));
check('stem snow', [71, 73, 75, 77].every((c) => weatherIconStem(c) === 'weather-snow'));
check('stem storm', [95, 96, 99].every((c) => weatherIconStem(c) === 'weather-storm'));
check('stem unknown', weatherIconStem(42) === 'weather-cloud');

check('label known', weatherLabel(0) === 'Clear' && weatherLabel(65) === 'Heavy rain');
check('label unknown', weatherLabel(123) === 'Cloudy');

check('round whole', roundTemp(12.4) === 12 && roundTemp(12.6) === 13 && roundTemp('9') === 9);
check('round unknown', roundTemp(null) === null && roundTemp('nope') === null);

check('temp text', tempText(12.6) === '13°' && tempText(-3.2) === '-3°');
check('temp text unknown', tempText(null) === '—');

check('day label today', forecastDayLabel('2026-10-10', 0, 'en-US') === 'Today');
check('day label weekday', forecastDayLabel('2026-10-11', 1, 'en-US') === 'Sun');
check('day label bad input', forecastDayLabel('nope', 1, 'en-US') === '');

if (failures) {
  console.error(`\n${failures} check(s) failed`);
  process.exit(1);
}
console.log('\nall weatherShared checks passed');
