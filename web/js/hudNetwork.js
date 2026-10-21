/**
 * hudNetwork.js — the top-bar network chip: the active link, its signal
 * strength and whether the internet actually works.
 *
 * Fed by `/api/network/events` (SSE, pushed whenever NetworkManager reports a
 * change) with a slow poll as a fallback while the stream is down. The icon
 * bucket (0–4) is computed server-side (`signal_level`, GNOME's thresholds),
 * so this module only maps a bucket to a theme icon: `hud/wifi-<n>`. States
 * that are not about strength get their own icon:
 *
 *   connected      hud/wifi-<level>      (is-connected, or is-warning when the
 *                                        connectivity check says no internet)
 *   wired          hud/ethernet
 *   radio off      hud/wifi-off
 *   rfkill / hw    hud/wifi-blocked
 *   not connected  hud/wifi-0            (dimmed)
 *   scanning       the chips just update; the window shows the spinner
 *
 * No wireless device and no wired link → the chip hides itself.
 */

import { createHudStream } from './hudStream.js';
import { setIcon } from '../ui/index.js';
import { toggleNetworkMenu } from './networkMenu.js';
import { t } from './i18n.js';

const chipEl = document.getElementById('hud-network');
const iconEl = document.getElementById('hud-network-icon');
const labelEl = document.getElementById('hud-network-label');
const pctEl = document.getElementById('hud-network-pct');

const stream = createHudStream({
  eventsUrl: '/api/network/events',
  statusUrl: '/api/network/status',
  pollMs: 15000,
  label: 'network',
  apply,
});

export function initHudNetwork() {
  if (!chipEl) return;
  chipEl.setAttribute('aria-haspopup', 'dialog');
  chipEl.setAttribute('aria-expanded', 'false');
  chipEl.addEventListener('click', () => toggleNetworkMenu(chipEl));
  stream.start();
}

/* ── Rendering ──────────────────────────────────────────────── */

function apply(status) {
  if (!chipEl) return;
  if (!status || !status.available) {
    chipEl.classList.add('hidden');
    return;
  }

  const { wifi, connection, connectivity } = status;
  chipEl.classList.remove('hidden', 'is-connected', 'is-warning', 'is-dim', 'is-wired');

  if (connection && connection.kind === 'ethernet') {
    setIcon(iconEl, 'hud/ethernet', { size: 20 });
    chipEl.classList.add('is-wired');
    labelEl.textContent = t('net.ethernet');
    pctEl.textContent = '';
    const bits = [
      t('net.ethernet'),
      connection.interface,
      connection.speed_mbps ? `${connection.speed_mbps} Mb/s` : null,
      connection.ip4 || null,
      connection.managed === false ? t('net.systemManaged') : null,
    ].filter(Boolean);
    chipEl.title = bits.join(' — ');
    chipEl.setAttribute('aria-label', bits.join(', '));
    return;
  }

  if (connection && connection.kind === 'wifi') {
    const limited = connectivity && connectivity.internet === false;
    setIcon(iconEl, `hud/wifi-${connection.level}`, { size: 20 });
    chipEl.classList.add(limited ? 'is-warning' : 'is-connected');
    labelEl.textContent = connection.ssid || t('net.wifi');
    pctEl.textContent = connection.strength != null ? `${connection.strength}%` : '';
    const bits = [
      connection.ssid || t('net.wifi'),
      connection.strength != null ? `${connection.strength}%` : null,
      connection.security || null,
      connection.band || null,
      limited ? t('net.noInternet') : null,
      connection.ip4 || null,
    ].filter(Boolean);
    chipEl.title = bits.join(' — ');
    chipEl.setAttribute('aria-label', bits.join(', '));
    return;
  }

  // Nothing connected: explain why as briefly as the chip allows.
  if (wifi.present && wifi.hardware_enabled === false) {
    setIcon(iconEl, 'hud/wifi-blocked', { size: 20 });
    chipEl.classList.add('is-dim');
    labelEl.textContent = t('hud.wifiBlocked');
    pctEl.textContent = '';
    chipEl.title = t('hud.wifiHardwareSwitch');
    chipEl.setAttribute('aria-label', chipEl.title);
    return;
  }

  if (wifi.present && wifi.enabled === false) {
    setIcon(iconEl, 'hud/wifi-off', { size: 20 });
    chipEl.classList.add('is-dim');
    labelEl.textContent = t('hud.wifiOff');
    pctEl.textContent = '';
    chipEl.title = t('hud.wifiTurnedOffHint');
    chipEl.setAttribute('aria-label', chipEl.title);
    return;
  }

  if (wifi.present) {
    setIcon(iconEl, 'hud/wifi-0', { size: 20 });
    chipEl.classList.add('is-dim');
    labelEl.textContent = t('net.notConnected');
    pctEl.textContent = '';
    chipEl.title = t('hud.noNetwork');
    chipEl.setAttribute('aria-label', chipEl.title);
    return;
  }

  // No wireless hardware: hide unless a wired link shows up later.
  chipEl.classList.add('hidden');
}
