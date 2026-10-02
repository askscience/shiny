/**
 * Pure-unit checks for powerShared.js. Run:
 *   node web/js/tests/powerShared.test.mjs
 */

import {
  batteryTriggersSaver, describeEngines, effectivePowerMode, isLowBattery,
  isOnBattery, lowPowerEngines, normalizePowerMode, powerModeHint,
  powerModeLabel, POWER_MODES,
} from '../powerShared.js';

let failures = 0;
function check(name, condition) {
  if (condition) {
    console.log(`  ok  ${name}`);
  } else {
    failures += 1;
    console.error(`FAIL  ${name}`);
  }
}

const onBattery = { available: true, state: 'discharging', percent: 76 };
const charging = { available: true, state: 'charging', percent: 40 };
const lowCharging = { available: true, state: 'charging', percent: 12 };
const full = { available: true, state: 'full', percent: 100 };
const noBattery = { available: false };

check('modes', POWER_MODES.join(',') === 'performance,balanced,saver');
check('normalize keeps known', POWER_MODES.every((m) => normalizePowerMode(m) === m));
check('normalize unknown -> balanced', normalizePowerMode('turbo') === 'balanced');
check('normalize null -> balanced', normalizePowerMode(null) === 'balanced');

check('label performance', powerModeLabel('performance') === 'Performance');
check('label saver', powerModeLabel('saver') === 'Power Saver');
check('label balanced', powerModeLabel('balanced') === 'Balanced');
check('hint is a string', typeof powerModeHint('balanced') === 'string');

check('on battery', isOnBattery(onBattery) === true && isOnBattery(full) === false);
check('on battery requires available', isOnBattery({ ...onBattery, available: false }) === false);
check('ac wins over a discharging blip', isOnBattery({ ...onBattery, ac_online: true }) === false);
check('unknown AC trusts discharging', isOnBattery({ ...onBattery, ac_online: null }) === true);
check('low battery', isLowBattery(lowCharging) === true && isLowBattery(onBattery) === false);
check('trigger discharging', batteryTriggersSaver(onBattery) === true);
check('trigger low while charging', batteryTriggersSaver(lowCharging) === true);
check('no trigger on AC', batteryTriggersSaver(full) === false && batteryTriggersSaver(noBattery) === false);

check('performance never saves', effectivePowerMode('performance', true, onBattery) === 'performance');
check('saver always saves', effectivePowerMode('saver', false, full) === 'saver');
check('balanced + auto on battery', effectivePowerMode('balanced', true, onBattery) === 'saver');
check('balanced + auto low', effectivePowerMode('balanced', true, lowCharging) === 'saver');
check('balanced + auto on AC', effectivePowerMode('balanced', true, full) === 'balanced');
check('balanced + auto off', effectivePowerMode('balanced', false, onBattery) === 'balanced');
check('balanced + no status', effectivePowerMode('balanced', true, null) === 'balanced');
check('unknown mode falls back to balanced', effectivePowerMode('nope', false, full) === 'balanced');

check('low power engines active', JSON.stringify(lowPowerEngines('saver', true)) === '{"stt":"vosk","tts":"supertonic"}');
check('low power engines disabled', lowPowerEngines('saver', false) === null);
check('low power engines not in balanced', lowPowerEngines('balanced', true) === null);

check('describe engines light', describeEngines('vosk', 'supertonic') === 'Vosk + Supertonic');
check('describe engines heavy', describeEngines('whisper', 'qwen') === 'Faster Whisper + Qwen3-TTS');

if (failures) {
  console.error(`\n${failures} check(s) failed`);
  process.exit(1);
}
console.log('\nall powerShared checks passed');
