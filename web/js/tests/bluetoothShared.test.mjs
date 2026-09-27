/**
 * Pure-unit checks for bluetoothShared.js. Run:
 *   node web/js/tests/bluetoothShared.test.mjs
 */

import {
  batteryLabel, chipIconName, chipLabel, connectedDevice, deviceIconName, signalLabel,
} from '../bluetoothShared.js';

let failures = 0;
function check(name, condition) {
  if (condition) {
    console.log(`  ok  ${name}`);
  } else {
    failures += 1;
    console.error(`FAIL  ${name}`);
  }
}

const off = {
  available: true, present: true, powered: false, discovering: false, devices: [],
};
const on = {
  available: true, present: true, powered: true, discovering: false, devices: [],
};
const scanning = { ...on, discovering: true };
const connected = {
  ...on,
  devices: [
    { id: '/org/bluez/hci0/dev_AA', address: 'AA', name: 'Sony WH-1000XM4', kind: 'headset', connected: true, battery: 80, rssi: -41 },
    { id: '/org/bluez/hci0/dev_BB', address: 'BB', name: 'Keyboard', kind: 'keyboard', connected: false },
  ],
};

check('no status', chipIconName(null) === 'hud/bluetooth-off');
check('unavailable hides power', chipIconName({ available: false }) === 'hud/bluetooth-off');
check('no adapter', chipIconName({ available: true, present: false }) === 'hud/bluetooth-off');
check('off', chipIconName(off) === 'hud/bluetooth-off');
check('on', chipIconName(on) === 'hud/bluetooth');

check('label unavailable', chipLabel({ available: false }) === 'Bluetooth');
check('label no adapter', chipLabel({ available: true, present: false }) === 'No Bluetooth');
check('label off', chipLabel(off) === 'Bluetooth off');
check('label scanning', chipLabel(scanning) === 'Searching…');
check('label idle', chipLabel(on) === 'Bluetooth');
check('label connected device', chipLabel(connected) === 'Sony WH-1000XM4');

check('connected device picked', connectedDevice(connected)?.address === 'AA');
check('no connected device', connectedDevice(on) === null);

check('audio device icon', deviceIconName('headset') === 'hud/headphones');
check('speaker device icon', deviceIconName('speaker') === 'hud/headphones');
check('keyboard device icon', deviceIconName('keyboard') === 'hud/bluetooth');
check('battery', batteryLabel(connected.devices[0]) === '80%');
check('no battery', batteryLabel(connected.devices[1]) === '');
check('signal', signalLabel(connected.devices[0]) === '-41 dBm');
check('no signal', signalLabel(connected.devices[1]) === '');

if (failures) {
  console.error(`\n${failures} check(s) failed`);
  process.exit(1);
}
console.log('\nall bluetoothShared checks passed');
