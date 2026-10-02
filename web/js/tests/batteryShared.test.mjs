/**
 * Pure-unit checks for batteryShared.js. Run:
 *   node web/js/tests/batteryShared.test.mjs
 */

import {
  batteryIconName, batteryLevel, chipLabel, chipPercent, chipStateClass,
  formatDuration, isLow, timeLabel,
} from '../batteryShared.js';

let failures = 0;
function check(name, condition) {
  if (condition) {
    console.log(`  ok  ${name}`);
  } else {
    failures += 1;
    console.error(`FAIL  ${name}`);
  }
}

const discharging = {
  available: true, present: true, percent: 76, level: 3, state: 'discharging',
  ac_online: false, time_to_empty_secs: 7500,
};
const low = { ...discharging, percent: 8, level: 0 };
const charging = {
  available: true, present: true, percent: 40, level: 2, state: 'charging',
  ac_online: true, time_to_full_secs: 4200,
};
const full = { ...charging, percent: 100, level: 4, state: 'full', time_to_full_secs: null };
const idle = { ...charging, percent: 64, level: 3, state: 'idle', time_to_full_secs: null };

check('level buckets', [0, 10, 11, 30, 31, 60, 61, 85, 86, 100]
  .map(batteryLevel).join(',') === '0,0,1,1,2,2,3,3,4,4');
check('level unknown', batteryLevel(null) === null);
check('level clamps', batteryLevel(200) === 4 && batteryLevel(-5) === 0);

check('icon discharging', batteryIconName(discharging) === 'hud/battery-3');
check('icon low', batteryIconName(low) === 'hud/battery-0');
check('icon charging', batteryIconName(charging) === 'hud/battery-charging');
check('icon charging ignores level', batteryIconName({ ...charging, level: 0 }) === 'hud/battery-charging');
check('icon from percent when no level', batteryIconName({ state: 'discharging', percent: 55 }) === 'hud/battery-2');
check('icon unknown percentage', batteryIconName({ state: 'idle', percent: null }) === 'hud/battery-4');

check('label charging', chipLabel(charging) === 'Charging');
check('label full', chipLabel(full) === 'Full');
check('label discharging', chipLabel(discharging) === 'On battery');
check('label idle', chipLabel(idle) === 'Plugged in');
check('label unavailable', chipLabel({ available: false }) === 'Battery');

check('percent', chipPercent(discharging) === '76%');
check('percent unknown', chipPercent({ percent: null }) === '');

check('class charging', chipStateClass(charging) === 'is-charging');
check('class low', chipStateClass(low) === 'is-low');
check('class discharging normal', chipStateClass(discharging) === '');
check('class full', chipStateClass(full) === 'is-dim');
check('class idle', chipStateClass(idle) === 'is-dim');
check('isLow', isLow(low) === true && isLow(discharging) === false);

check('duration minutes', formatDuration(300) === '5 min');
check('duration hours', formatDuration(7200) === '2 h');
check('duration mixed', formatDuration(7500) === '2 h 5 min');
check('duration small', formatDuration(20) === '<1 min');
check('duration none', formatDuration(null) === '' && formatDuration(0) === '');

check('time left', timeLabel(discharging) === '2 h 5 min left');
check('time to full', timeLabel(charging) === '1 h 10 min to full');
check('time full none', timeLabel(full) === '');

if (failures) {
  console.error(`\n${failures} check(s) failed`);
  process.exit(1);
}
console.log('\nall batteryShared checks passed');
