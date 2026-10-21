/**
 * hudBluetooth.js — the top-bar Bluetooth chip: whether the adapter is on and
 * which device is connected.
 *
 * Fed by `/api/bluetooth/events` (SSE, pushed whenever BlueZ reports a change)
 * with a slow poll as a fallback while the stream is down. The label and icon
 * come from `bluetoothShared.js` so the chip and the menu always agree.
 *
 *   connected     device icon + name      (is-connected; battery as the %)
 *   on, idle      hud/bluetooth           (is-dim)
 *   off / no HW   hud/bluetooth-off       (is-dim)
 *   unavailable   hidden
 *
 * No adapter → the chip hides itself, like the network chip without Wi-Fi.
 */

import { createHudStream } from './hudStream.js';
import { setIcon } from '../ui/index.js';
import { toggleBluetoothMenu } from './bluetoothMenu.js';
import {
  batteryLabel, chipIconName, chipLabel, connectedDevice, deviceIconName,
} from './bluetoothShared.js';

const chipEl = document.getElementById('hud-bluetooth');
const iconEl = document.getElementById('hud-bluetooth-icon');
const labelEl = document.getElementById('hud-bluetooth-label');
const pctEl = document.getElementById('hud-bluetooth-pct');

const stream = createHudStream({
  eventsUrl: '/api/bluetooth/events',
  statusUrl: '/api/bluetooth/status',
  pollMs: 15000,
  label: 'bluetooth',
  apply,
});

export function initHudBluetooth() {
  if (!chipEl) return;
  chipEl.setAttribute('aria-haspopup', 'dialog');
  chipEl.setAttribute('aria-expanded', 'false');
  chipEl.addEventListener('click', () => toggleBluetoothMenu(chipEl));
  stream.start();
}

/* ── Rendering ──────────────────────────────────────────────── */

function apply(status) {
  if (!chipEl) return;
  if (!status || !status.available || !status.present) {
    chipEl.classList.add('hidden');
    return;
  }

  const device = connectedDevice(status);
  chipEl.classList.remove('hidden', 'is-connected', 'is-dim');
  setIcon(iconEl, device ? deviceIconName(device.kind) : chipIconName(status), { size: 20 });
  labelEl.textContent = chipLabel(status);
  pctEl.textContent = device ? batteryLabel(device) : '';

  chipEl.classList.add(device ? 'is-connected' : 'is-dim');

  const bits = [
    chipLabel(status),
    device ? device.address : null,
    device ? batteryLabel(device) : null,
    status.adapter ? `adapter ${status.adapter}` : null,
  ].filter(Boolean);
  chipEl.title = bits.join(' — ');
  chipEl.setAttribute('aria-label', bits.join(', '));
}
