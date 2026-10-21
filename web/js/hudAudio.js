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

import { createHudStream } from './hudStream.js';
import { setIcon } from '../ui/index.js';
import { toggleAudioMenu } from './audioMenu.js';
import { currentSink, nodeIcon } from './audioShared.js';
import { t } from './i18n.js';

const chipEl = document.getElementById('hud-audio');
const iconEl = document.getElementById('hud-audio-icon');
const labelEl = document.getElementById('hud-audio-label');
const pctEl = document.getElementById('hud-audio-pct');

const stream = createHudStream({
  eventsUrl: '/api/audio/events',
  statusUrl: '/api/audio/status',
  pollMs: 15000,
  label: 'audio',
  apply,
});

export function initHudAudio() {
  if (!chipEl) return;
  chipEl.setAttribute('aria-haspopup', 'dialog');
  chipEl.setAttribute('aria-expanded', 'false');
  chipEl.addEventListener('click', () => toggleAudioMenu(chipEl));
  stream.start();
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
  setIcon(iconEl, nodeIcon(sink, 'sink'), { size: 20 });

  const label = sink.nick || sink.description || t('hud.sound');
  labelEl.textContent = label;
  pctEl.textContent = sink.muted ? t('audio.muted') : `${sink.volume_percent}%`;
  chipEl.classList.add(sink.muted ? 'is-muted' : 'is-active');

  const bits = [
    sink.description,
    sink.muted ? 'muted' : `${sink.volume_percent}%`,
    sink.card,
    sink.active_port,
  ].filter(Boolean);
  chipEl.title = bits.join(' — ');
  chipEl.setAttribute('aria-label', t('hud.soundAria', { details: bits.join(', ') }));
}
