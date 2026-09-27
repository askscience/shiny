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

import { apiFetch, getToken } from './api.js';
import { setIcon } from '../ui/index.js';
import { toggleBluetoothMenu } from './bluetoothMenu.js';
import {
  batteryLabel, chipIconName, chipLabel, connectedDevice, deviceIconName,
} from './bluetoothShared.js';

const chipEl = document.getElementById('hud-bluetooth');
const iconEl = document.getElementById('hud-bluetooth-icon');
const labelEl = document.getElementById('hud-bluetooth-label');
const pctEl = document.getElementById('hud-bluetooth-pct');

const RECONNECT_MS = 5000;
const POLL_MS = 15000;

let abort = null;
let pollTimer = null;

export function initHudBluetooth() {
  if (!chipEl) return;
  chipEl.setAttribute('aria-haspopup', 'dialog');
  chipEl.setAttribute('aria-expanded', 'false');
  chipEl.addEventListener('click', () => toggleBluetoothMenu(chipEl));
  void subscribe();
}

function headers() {
  const token = getToken();
  return token ? { Authorization: `Bearer ${token}` } : {};
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/* ── SSE ────────────────────────────────────────────────────── */

async function subscribe() {
  for (;;) {
    abort = new AbortController();
    try {
      const res = await fetch('/api/bluetooth/events', {
        headers: headers(),
        credentials: 'same-origin',
        signal: abort.signal,
      });
      if (!res.ok || !res.body) throw new Error(`bluetooth events: ${res.status}`);
      stopPolling();
      await readStream(res);
    } catch (_) {
      if (abort.signal.aborted) return;
    }
    // Stream ended or failed: keep the chip honest with the poll while we
    // wait to reconnect (the kiosk proxy may drop an idle stream).
    startPolling();
    await sleep(RECONNECT_MS);
  }
}

async function readStream(res) {
  const reader = res.body.getReader();
  const decoder = new TextDecoder();
  let buffer = '';

  for (;;) {
    const { done, value } = await reader.read();
    if (done) return;
    buffer += decoder.decode(value, { stream: true });

    const chunks = buffer.split('\n\n');
    buffer = chunks.pop() || '';

    for (const chunk of chunks) {
      const dataLine = chunk.split('\n').find((line) => line.startsWith('data:'));
      if (!dataLine) continue;
      try {
        apply(JSON.parse(dataLine.replace(/^data:\s*/, '')));
      } catch (_) { /* keep-alive comment or partial frame */ }
    }
  }
}

/* ── Poll fallback ──────────────────────────────────────────── */

function startPolling() {
  if (pollTimer) return;
  void poll();
  pollTimer = setInterval(() => void poll(), POLL_MS);
}

function stopPolling() {
  if (!pollTimer) return;
  clearInterval(pollTimer);
  pollTimer = null;
}

async function poll() {
  try {
    const res = await apiFetch('/api/bluetooth/status', { authRedirect: false });
    apply(res?.data);
  } catch (_) { /* server restarting — keep the last state */ }
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
  setIcon(iconEl, device ? deviceIconName(device.kind) : chipIconName(status), { size: 17 });
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
