/**
 * bluetoothShared.js — pure helpers shared by the top-bar Bluetooth chip
 * (`hudBluetooth.js`) and the Bluetooth menu (`bluetoothMenu.js`), so the two
 * can never disagree about what the status means.
 *
 * Import-free on purpose: `web/js/tests/bluetoothShared.test.mjs` loads it in
 * Node with no DOM.
 */

/** The device currently connected, if any. */
export function connectedDevice(status) {
  const devices = status?.devices || [];
  return devices.find((device) => device.connected) || null;
}

/** Which chip icon to draw. Off / no adapter share the crossed-out glyph. */
export function chipIconName(status) {
  if (!status || !status.available || !status.present || !status.powered) {
    return 'hud/bluetooth-off';
  }
  return 'hud/bluetooth';
}

/** The chip's short label. */
export function chipLabel(status) {
  if (!status || !status.available) return 'Bluetooth';
  if (!status.present) return 'No Bluetooth';
  if (!status.powered) return 'Bluetooth off';
  const device = connectedDevice(status);
  if (device) return device.name || device.address || 'Connected';
  if (status.discovering) return 'Searching…';
  return 'Bluetooth';
}

/** The icon for one device row. Audio devices reuse the headphones glyph. */
export function deviceIconName(kind) {
  if (kind === 'headset' || kind === 'headphones' || kind === 'audio' || kind === 'speaker') {
    return 'hud/headphones';
  }
  return 'hud/bluetooth';
}

/** A device's battery percentage as a label, or '' when unknown. */
export function batteryLabel(device) {
  return device && device.battery != null ? `${device.battery}%` : '';
}

/** Signal bars aren't meaningful with no RSSI; only show a real reading. */
export function signalLabel(device) {
  return device && device.rssi != null ? `${device.rssi} dBm` : '';
}
