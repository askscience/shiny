/**
 * hudAudio.js — the top-bar sound chip: the default output device, its
 * volume and its mute state.
 *
 * Fed by `/api/audio/events` (SSE, pushed whenever PipeWire reports a change)
 * with a slow poll as a fallback while the stream is down. The icon bucket
 * (0–3) is computed server-side (`volume_level`), so this module only maps a
 * bucket to a theme icon:
 *
 *   muted          hud/volume-muted
 *   headphones     hud/headphones
 *   otherwise      hud/volume-<level>   (0 silent … 3 loud)
 *
 * No audio server (or no output devices) → the chip hides itself.
 */

import { apiFetch, getToken } from './api.js';
import { setIcon } from '../ui/index.js';
import { toggleAudioMenu } from './audioMenu.js';
import { currentSink, nodeIcon } from './audioShared.js';

const chipEl = document.getElementById('hud-audio');
const iconEl = document.getElementById('hud-audio-icon');
const labelEl = document.getElementById('hud-audio-label');
const pctEl = document.getElementById('hud-audio-pct');

const RECONNECT_MS = 5000;
const POLL_MS = 15000;

let abort = null;
let pollTimer = null;

export function initHudAudio() {
  if (!chipEl) return;
  chipEl.setAttribute('aria-haspopup', 'dialog');
  chipEl.setAttribute('aria-expanded', 'false');
  chipEl.addEventListener('click', () => toggleAudioMenu(chipEl));
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
      const res = await fetch('/api/audio/events', {
        headers: headers(),
        credentials: 'same-origin',
        signal: abort.signal,
      });
      if (!res.ok || !res.body) throw new Error(`audio events: ${res.status}`);
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
    const res = await apiFetch('/api/audio/status', { authRedirect: false });
    apply(res?.data);
  } catch (_) { /* server restarting — keep the last state */ }
}

/* ── Rendering ──────────────────────────────────────────────── */

function apply(status) {
  if (!chipEl) return;
  const sink = currentSink(status);
  if (!status || !status.available || !sink) {
    chipEl.classList.add('hidden');
    return;
  }

  chipEl.classList.remove('hidden', 'is-muted', 'is-active');
  setIcon(iconEl, nodeIcon(sink, 'sink'), { size: 17 });

  const label = sink.nick || sink.description || 'Sound';
  labelEl.textContent = label;
  pctEl.textContent = sink.muted ? 'Muted' : `${sink.volume_percent}%`;
  chipEl.classList.add(sink.muted ? 'is-muted' : 'is-active');

  const bits = [
    sink.description,
    sink.muted ? 'muted' : `${sink.volume_percent}%`,
    sink.card,
    sink.active_port,
  ].filter(Boolean);
  chipEl.title = bits.join(' — ');
  chipEl.setAttribute('aria-label', `Sound: ${bits.join(', ')}`);
}
