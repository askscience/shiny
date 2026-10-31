/**
 * audioMenu.js — the Sound menu: a popover anchored to the top-bar chip with
 * the default output and input devices, their volume and mute state, and the
 * full device lists.
 *
 * Content is `/api/audio/status` (the server caches the PipeWire snapshot),
 * re-read by a short poll while the popover is open. A volume drag updates
 * the UI optimistically and pushes the value on a short debounce, so a slider
 * sweep is a handful of requests instead of one per pixel. Actions POST to
 * `/api/audio/*` — loopback-only server-side, which is exactly this case.
 *
 * The skeleton is built once and updated in place: re-rendering a slider
 * under the user's finger would drop the drag.
 */

import { apiFetch } from './api.js';
import { emptyState, iconButton, slider, toast, setIcon } from '../ui/index.js';
import { currentNode, isHeadphones, nodeIcon, nodeLabel, nodeSubtitle } from './audioShared.js';
import {
  el, menuFoot, menuLabel, menuNote, menuPill, menuPopup, menuRow, menuSlider, syncSliderFill,
} from './hudMenuShared.js';
import { createMenu } from './menuFactory.js';
import { t } from './i18n.js';

const POLL_MS = 2000;
const VOLUME_DEBOUNCE_MS = 120;

let bodyEl = null;
let hintEl = null;
let errorEl = null;
let loadingEl = null;
let status = null;
let pollTimer = null;
let dragging = false;
let signature = '';
let volumeTimer = null;
let audioCtx = null;
/** Per-target persistent elements, built once by `buildPopup`. */
const refs = {};

/* ── Open / close ──────────────────────────────────────────── */

const menu = createMenu({
  build: buildPopup,
  align: 'right',
  onOpen: () => {
    render(true);
    void refresh();
    if (!pollTimer) pollTimer = setInterval(() => void refresh(), POLL_MS);
  },
  onClose: () => {
    if (pollTimer) clearInterval(pollTimer);
    pollTimer = null;
  },
});

export function openAudioMenu(chip) {
  menu.show(chip);
}

export function closeAudioMenu() {
  menu.close();
}

export function toggleAudioMenu(chip) {
  menu.toggle(chip);
}

/* ── Skeleton ──────────────────────────────────────────────── */

function buildPopup() {
  const { popup, body } = menuPopup('audio-menu', t('hud.sound'));
  bodyEl = body;

  loadingEl = menuNote(t('audio.loading'));
  errorEl = emptyState({
    icon: 'ui/warning',
    title: t('audio.unavailable'),
    body: t('audio.pipewireUnreachable'),
  });
  errorEl.classList.add('hidden');

  refs.sink = buildSection(t('audio.output'), 'sink');
  refs.source = buildSection(t('audio.input'), 'source');

  bodyEl.append(loadingEl, errorEl, refs.sink.card, refs.source.card);

  const testBtn = menuPill({ label: t('audio.testSound'), iconName: 'ui/play', arrow: false, onClick: playTest });
  testBtn.title = t('audio.playTestSound');
  const { foot, hintEl: footHint } = menuFoot(testBtn);
  hintEl = footHint;
  popup.append(bodyEl, foot);
  return popup;
}

function buildSection(label, target) {
  const card = el('div', 'hud-menu-section hidden');

  const muteBtn = iconButton({
    icon: target === 'source' ? 'ui/mic-off' : 'hud/volume-muted',
    label: target === 'source' ? t('audio.muteMic') : t('audio.muteOutput'),
    variant: 'quiet',
    size: 'sm',
    className: 'hud-menu-lead-btn',
    onClick: () => void toggleMute(target),
  });
  const range = slider({
    min: 0,
    // Initial ceiling; `updateSection` replaces the output max with the value
    // the server advertises per node (150 % normally, more on the T2 DSP).
    // Input stays at 100 % so a boosted microphone cannot clip the capture.
    max: target === 'sink' ? 150 : 100,
    step: 1,
    value: 0,
    onInput: (value) => onVolumeInput(target, value),
  });
  range.setAttribute('aria-label', target === 'source' ? t('audio.inputVolume') : t('audio.outputVolume'));
  range.addEventListener('change', () => void commitVolume(target));
  const { row: controls, value: pctEl } = menuSlider({ leading: muteBtn, range });

  // Shown instead of the list when the section has no device at all.
  const emptyEl = menuNote('');
  emptyEl.classList.add('hidden');
  const listEl = el('div', 'hud-menu-list');

  card.append(menuLabel(label), controls, emptyEl, listEl);
  return { card, pctEl, muteBtn, range, emptyEl, listEl, current: null, listSig: '' };
}

/* ── Data ──────────────────────────────────────────────────── */

async function refresh(force = false) {
  if (!menu.isOpen()) return;
  try {
    const res = await apiFetch('/api/audio/status', { authRedirect: false });
    if (res?.data) {
      status = res.data;
      render(force);
    }
  } catch (_) {
    // Server restarting or offline: keep the last snapshot on screen.
  }
}

/* ── Rendering ─────────────────────────────────────────────── */

function render(force = false) {
  if (!bodyEl) return;
  // Never touch the controls mid-drag: an incoming snapshot would move the
  // slider under the user's finger.
  if (dragging && !force) return;

  if (!status) {
    loadingEl.classList.remove('hidden');
    return;
  }
  loadingEl.classList.add('hidden');

  const available = !!status.available;
  errorEl.classList.toggle('hidden', available);
  refs.sink.card.classList.toggle('hidden', !available);
  refs.source.card.classList.toggle('hidden', !available);

  if (!available) {
    hintEl.textContent = status.reason || t('audio.pipewireNotAvailable');
    const text = errorEl.querySelector('.ui-subtitle');
    if (text) text.textContent = status.reason || t('audio.pipewireUnreachable');
    return;
  }

  hintEl.textContent = status.server || 'PipeWire';
  hintEl.title = hintEl.textContent;
  const sig = JSON.stringify([
    status.default_sink,
    status.default_source,
    status.sinks,
    status.sources,
  ]);
  if (!force && sig === signature) return;
  signature = sig;

  updateSection(refs.sink, 'sink', status.sinks, status.default_sink);
  updateSection(refs.source, 'source', status.sources, status.default_source);
  requestAnimationFrame(() => menu.reposition());
}

function updateSection(ref, target, nodes, defaultName) {
  const node = currentNode(nodes, defaultName);
  ref.current = node;
  ref.range.disabled = !node;
  ref.muteBtn.disabled = !node;

  if (!node) {
    setButtonIcon(ref.muteBtn, nodeIcon(null, target));
    ref.emptyEl.textContent = `${target === 'sink' ? t('audio.noOutputs') : t('audio.noInputs')} · ${t('audio.pipewireNone')}`;
    ref.emptyEl.classList.remove('hidden');
    ref.pctEl.textContent = '';
    ref.listEl.classList.add('hidden');
    ref.listSig = '';
    return;
  }
  ref.emptyEl.classList.add('hidden');

  ref.pctEl.textContent = node.muted ? t('audio.muted') : `${node.volume_percent}`;
  ref.pctEl.classList.toggle('is-over', !node.muted && node.volume_percent > 100);
  ref.pctEl.classList.toggle('is-muted', !!node.muted);
  // The server tells us the ceiling: 150 % normally, more for the T2 speaker
  // DSP. Input stays at 100 % so a boosted microphone cannot clip the capture.
  ref.range.max = String(target === 'sink' ? (node.max_volume || 150) : 100);
  setButtonIcon(ref.muteBtn, nodeIcon(node, target));
  ref.muteBtn.classList.toggle('is-muted', !!node.muted);
  ref.muteBtn.setAttribute('aria-label', node.muted
    ? (target === 'source' ? t('audio.unmuteMic') : t('audio.unmuteOutput'))
    : (target === 'source' ? t('audio.muteMic') : t('audio.muteOutput')));
  ref.muteBtn.title = ref.muteBtn.getAttribute('aria-label');
  if (!dragging) ref.range.value = String(node.volume_percent);
  syncSliderFill(ref.range);

  renderDeviceList(ref, target, nodes, node);
}

/** Replace an icon-button's glyph (iconButton has no setter of its own). */
function setButtonIcon(button, name) {
  const glyph = button.querySelector('.ui-icon');
  if (glyph) void setIcon(glyph, name, { size: 16 });
}

/** Every device of the section, the default one checked; a click switches. */
function renderDeviceList(ref, target, nodes, current) {
  const list = nodes || [];
  ref.listEl.classList.toggle('hidden', !list.length);

  const sig = list.map((node) => `${node.id}:${node === current ? 1 : 0}:${nodeLabel(node)}`).join(',');
  if (sig === ref.listSig) return;
  ref.listSig = sig;

  ref.listEl.textContent = '';
  for (const node of list) {
    const selected = node === current;
    ref.listEl.appendChild(menuRow({
      iconName: deviceIcon(node, target),
      title: nodeLabel(node),
      tooltip: nodeSubtitle(node) || nodeLabel(node),
      selected,
      onClick: selected ? undefined : () => void setDefault(target, node),
    }));
  }
}

/** A device row's glyph: what the device is, not its mute/level state. */
function deviceIcon(node, target) {
  if (isHeadphones(node)) return 'hud/headphones';
  if (target === 'source') return 'ui/mic';
  return /hdmi|displayport/i.test(`${node.active_port || ''} ${node.description || ''}`) ? 'ui/monitor' : 'ui/volume';
}

/* ── Interaction ───────────────────────────────────────────── */

function onVolumeInput(target, value) {
  const ref = refs[target];
  dragging = true;
  if (ref) {
    ref.pctEl.textContent = `${value}`;
    ref.pctEl.classList.toggle('is-over', Number(value) > 100);
  }
  clearTimeout(volumeTimer);
  volumeTimer = setTimeout(() => void pushVolume(target, value), VOLUME_DEBOUNCE_MS);
}

async function commitVolume(target) {
  const ref = refs[target];
  dragging = false;
  clearTimeout(volumeTimer);
  if (!ref) return;
  await pushVolume(target, Number(ref.range.value));
  await refresh(true);
}

async function pushVolume(target, percent) {
  const node = refs[target]?.current;
  if (!node) return;
  try {
    await apiFetch('/api/audio/volume', {
      method: 'POST',
      body: JSON.stringify({ target, id: node.id, percent }),
    });
  } catch (err) {
    toast(err.message || t('audio.setVolumeFailed'), { type: 'error' });
  }
}

async function toggleMute(target) {
  const node = refs[target]?.current;
  if (!node) return;
  try {
    await apiFetch('/api/audio/mute', {
      method: 'POST',
      body: JSON.stringify({ target, id: node.id, muted: !node.muted }),
    });
  } catch (err) {
    toast(err.message || t('audio.setMuteFailed'), { type: 'error' });
  } finally {
    await refresh(true);
  }
}

async function setDefault(target, node) {
  try {
    await apiFetch('/api/audio/default', {
      method: 'POST',
      body: JSON.stringify({ target, id: node.id }),
    });
    toast(t('audio.switched', {
      target: target === 'sink' ? t('audio.output') : t('audio.input'),
      name: nodeLabel(node),
    }));
  } catch (err) {
    toast(err.message || t('audio.switchFailed'), { type: 'error' });
  } finally {
    await refresh(true);
  }
}

/** Short chime through the browser, i.e. through the sink we are controlling. */
function playTest() {
  try {
    audioCtx = audioCtx || new (window.AudioContext || window.webkitAudioContext)();
    if (audioCtx.state === 'suspended') void audioCtx.resume();
    const now = audioCtx.currentTime;
    const osc = audioCtx.createOscillator();
    const gain = audioCtx.createGain();
    osc.type = 'sine';
    osc.frequency.setValueAtTime(523.25, now);
    osc.frequency.setValueAtTime(783.99, now + 0.16);
    gain.gain.setValueAtTime(0, now);
    gain.gain.linearRampToValueAtTime(0.15, now + 0.02);
    gain.gain.exponentialRampToValueAtTime(0.0001, now + 0.5);
    osc.connect(gain).connect(audioCtx.destination);
    osc.start(now);
    osc.stop(now + 0.55);
  } catch (_) {
    toast(t('audio.testSoundFailed'), { type: 'error' });
  }
}
