/**
 * hudBattery.js — the top-bar battery chip: charge level, whether the machine
 * is on wall power, and (when the kernel can estimate it) time remaining.
 *
 * Fed by `/api/battery/events` (SSE, pushed whenever the snapshot changes) with
 * a slow poll as a fallback while the stream is down. The label, icon and state
 * class come from `batteryShared.js`, so the chip never disagrees with itself.
 *
 *   charging       hud/battery-charging   (is-charging; accent, "Charging")
 *   discharging    hud/battery-<level>    (is-low at ≤30%; "On battery")
 *   full / idle    hud/battery-<level>    (is-dim; "Full" / "Plugged in")
 *   no battery     hidden
 *
 * No battery hardware → the chip hides itself, like the network chip with no
 * Wi-Fi. Clicking it opens the reduced power-management quick menu
 * (`batteryMenu.js`); the big controls live in Settings → Power.
 */

import { apiFetch, getToken } from './api.js';
import { setIcon } from '../ui/index.js';
import { setPowerBatteryStatus } from './preferences.js';
import { toggleBatteryMenu } from './batteryMenu.js';
import {
  batteryIconName, chipLabel, chipPercent, chipStateClass, timeLabel,
} from './batteryShared.js';

const chipEl = document.getElementById('hud-battery');
const iconEl = document.getElementById('hud-battery-icon');
const labelEl = document.getElementById('hud-battery-label');
const pctEl = document.getElementById('hud-battery-pct');

const RECONNECT_MS = 5000;
const POLL_MS = 30000;

let abort = null;
let pollTimer = null;

export function initHudBattery() {
  if (!chipEl) return;
  chipEl.addEventListener('click', () => toggleBatteryMenu(chipEl));
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
      const res = await fetch('/api/battery/events', {
        headers: headers(),
        credentials: 'same-origin',
        signal: abort.signal,
      });
      if (!res.ok || !res.body) throw new Error(`battery events: ${res.status}`);
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
    const res = await apiFetch('/api/battery/status', { authRedirect: false });
    apply(res?.data);
  } catch (_) { /* server restarting — keep the last state */ }
}

/* ── Rendering ──────────────────────────────────────────────── */

function apply(status) {
  if (!chipEl) return;
  // Power management and the quick menu both key off the live snapshot.
  setPowerBatteryStatus(status || null);
  window.dispatchEvent(new CustomEvent('battery:changed', { detail: status || null }));
  if (!status || !status.available || !status.present) {
    chipEl.classList.add('hidden');
    return;
  }

  chipEl.classList.remove('hidden', 'is-charging', 'is-low', 'is-dim');
  setIcon(iconEl, batteryIconName(status), { size: 20 });
  labelEl.textContent = chipLabel(status);
  pctEl.textContent = chipPercent(status);
  const state = chipStateClass(status);
  if (state) chipEl.classList.add(state);

  const bits = [
    chipLabel(status),
    status.percent != null ? `${status.percent}%` : null,
    timeLabel(status),
    status.ac_online === true ? 'AC connected' : null,
    status.status || null,
  ].filter(Boolean);
  chipEl.title = bits.join(' — ');
  chipEl.setAttribute('aria-label', bits.join(', '));
}
