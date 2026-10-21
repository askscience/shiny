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

import { createHudStream } from './hudStream.js';
import { setIcon } from '../ui/index.js';
import { setPowerBatteryStatus } from './preferences.js';
import { toggleBatteryMenu } from './batteryMenu.js';
import { t } from './i18n.js';
import {
  batteryIconName, chipLabel, chipPercent, chipStateClass, timeLabel,
} from './batteryShared.js';

const chipEl = document.getElementById('hud-battery');
const iconEl = document.getElementById('hud-battery-icon');
const labelEl = document.getElementById('hud-battery-label');
const pctEl = document.getElementById('hud-battery-pct');

const stream = createHudStream({
  eventsUrl: '/api/battery/events',
  statusUrl: '/api/battery/status',
  pollMs: 30000,
  label: 'battery',
  apply,
});

export function initHudBattery() {
  if (!chipEl) return;
  chipEl.addEventListener('click', () => toggleBatteryMenu(chipEl));
  stream.start();
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
    status.ac_online === true ? t('battery.acConnected') : null,
    status.status || null,
  ].filter(Boolean);
  chipEl.title = bits.join(' — ');
  chipEl.setAttribute('aria-label', bits.join(', '));
}
