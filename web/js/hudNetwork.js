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

import { apiFetch, getToken } from './api.js';
import { setIcon } from '../ui/index.js';
import { toggleNetworkMenu } from './networkMenu.js';

const chipEl = document.getElementById('hud-network');
const iconEl = document.getElementById('hud-network-icon');
const labelEl = document.getElementById('hud-network-label');
const pctEl = document.getElementById('hud-network-pct');

const RECONNECT_MS = 5000;
const POLL_MS = 15000;

let abort = null;
let pollTimer = null;

export function initHudNetwork() {
  if (!chipEl) return;
  chipEl.setAttribute('aria-haspopup', 'dialog');
  chipEl.setAttribute('aria-expanded', 'false');
  chipEl.addEventListener('click', () => toggleNetworkMenu(chipEl));
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
      const res = await fetch('/api/network/events', {
        headers: headers(),
        credentials: 'same-origin',
        signal: abort.signal,
      });
      if (!res.ok || !res.body) throw new Error(`network events: ${res.status}`);
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
    const res = await apiFetch('/api/network/status', { authRedirect: false });
    apply(res?.data);
  } catch (_) { /* server restarting — keep the last state */ }
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
    setIcon(iconEl, 'hud/ethernet', { size: 17 });
    chipEl.classList.add('is-wired');
    labelEl.textContent = 'Ethernet';
    pctEl.textContent = '';
    const bits = [
      'Ethernet',
      connection.interface,
      connection.speed_mbps ? `${connection.speed_mbps} Mb/s` : null,
      connection.ip4 || null,
      connection.managed === false ? 'system-managed' : null,
    ].filter(Boolean);
    chipEl.title = bits.join(' — ');
    chipEl.setAttribute('aria-label', bits.join(', '));
    return;
  }

  if (connection && connection.kind === 'wifi') {
    const limited = connectivity && connectivity.internet === false;
    setIcon(iconEl, `hud/wifi-${connection.level}`, { size: 17 });
    chipEl.classList.add(limited ? 'is-warning' : 'is-connected');
    labelEl.textContent = connection.ssid || 'Wi-Fi';
    pctEl.textContent = connection.strength != null ? `${connection.strength}%` : '';
    const bits = [
      connection.ssid || 'Wi-Fi',
      connection.strength != null ? `${connection.strength}%` : null,
      connection.security || null,
      connection.band || null,
      limited ? 'no internet' : null,
      connection.ip4 || null,
    ].filter(Boolean);
    chipEl.title = bits.join(' — ');
    chipEl.setAttribute('aria-label', bits.join(', '));
    return;
  }

  // Nothing connected: explain why as briefly as the chip allows.
  if (wifi.present && wifi.hardware_enabled === false) {
    setIcon(iconEl, 'hud/wifi-blocked', { size: 17 });
    chipEl.classList.add('is-dim');
    labelEl.textContent = 'Wi-Fi blocked';
    pctEl.textContent = '';
    chipEl.title = 'Wireless is disabled by a hardware switch';
    chipEl.setAttribute('aria-label', chipEl.title);
    return;
  }

  if (wifi.present && wifi.enabled === false) {
    setIcon(iconEl, 'hud/wifi-off', { size: 17 });
    chipEl.classList.add('is-dim');
    labelEl.textContent = 'Wi-Fi off';
    pctEl.textContent = '';
    chipEl.title = 'Wi-Fi is turned off — open the network panel to enable it';
    chipEl.setAttribute('aria-label', chipEl.title);
    return;
  }

  if (wifi.present) {
    setIcon(iconEl, 'hud/wifi-0', { size: 17 });
    chipEl.classList.add('is-dim');
    labelEl.textContent = 'Not connected';
    pctEl.textContent = '';
    chipEl.title = 'No network connection';
    chipEl.setAttribute('aria-label', chipEl.title);
    return;
  }

  // No wireless hardware: hide unless a wired link shows up later.
  chipEl.classList.add('hidden');
}
