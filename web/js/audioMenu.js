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
import { emptyState, icon, iconButton, listItem, slider, toast, setIcon, placeMenu } from '../ui/index.js';
import { currentNode, nodeIcon, nodeLabel, nodeSubtitle } from './audioShared.js';
import { notifyMenuChange } from './menuState.js';
import { t } from './i18n.js';

const POLL_MS = 2000;
const VOLUME_DEBOUNCE_MS = 120;

let popup = null;
let bodyEl = null;
let hintEl = null;
let errorEl = null;
let loadingEl = null;
let open = false;
let trigger = null;
let status = null;
let pollTimer = null;
let dragging = false;
let signature = '';
let volumeTimer = null;
let audioCtx = null;
/** Per-target persistent elements, built once by `ensurePopup`. */
const refs = {};

/* ── Open / close ──────────────────────────────────────────── */

export function isAudioMenuOpen() {
  return open;
}

export function openAudioMenu(chip) {
  trigger = chip || trigger;
  ensurePopup();
  open = true;
  // The popup element is reused between opens (so its listeners and scroll
  // position survive), and only `ensurePopup` runs once — unhide it here,
  // otherwise the second open would render into a hidden element.
  popup.classList.remove('hidden');
  // Restart the entry animation: it played on the first open and a reused
  // element would otherwise just pop in.
  popup.style.animation = 'none';
  void popup.offsetHeight;
  popup.style.animation = '';
  render(true);
  void refresh();
  if (!pollTimer) pollTimer = setInterval(() => void refresh(), POLL_MS);
  document.addEventListener('pointerdown', onOutside, true);
  document.addEventListener('keydown', onKey, true);
  window.addEventListener('resize', reposition);
  trigger?.setAttribute('aria-expanded', 'true');
  trigger?.classList.add('is-open');
  reposition();
  notifyMenuChange();
}

export function closeAudioMenu() {
  if (!open) return;
  open = false;
  popup?.classList.add('hidden');
  if (pollTimer) clearInterval(pollTimer);
  pollTimer = null;
  document.removeEventListener('pointerdown', onOutside, true);
  document.removeEventListener('keydown', onKey, true);
  window.removeEventListener('resize', reposition);
  trigger?.setAttribute('aria-expanded', 'false');
  trigger?.classList.remove('is-open');
  notifyMenuChange();
}

export function toggleAudioMenu(chip) {
  if (open) closeAudioMenu();
  else openAudioMenu(chip);
}

function onOutside(event) {
  if (event.target.closest?.('.ui-modal')) return;
  if (popup?.contains(event.target)) return;
  if (trigger?.contains(event.target)) return;
  closeAudioMenu();
}

function onKey(event) {
  if (event.key === 'Escape' && !document.querySelector('.ui-modal:not(.hidden)')) {
    closeAudioMenu();
  }
}

function reposition() {
  if (!popup || !trigger) return;
  placeMenu(popup, trigger, { align: 'right' });
}

/* ── Skeleton ──────────────────────────────────────────────── */

function ensurePopup() {
  if (popup) return;
  popup = document.createElement('div');
  popup.className = 'ui-hud-menu-popup audio-menu hidden';
  popup.setAttribute('role', 'dialog');
  popup.setAttribute('aria-label', t('hud.sound'));

  const head = document.createElement('div');
  head.className = 'audio-head';
  const meta = document.createElement('div');
  meta.className = 'audio-head-main';
  const title = document.createElement('div');
  title.className = 'audio-head-title';
  title.appendChild(icon('ui/volume', { size: 15 }));
  const label = document.createElement('span');
  label.textContent = t('hud.sound');
  title.appendChild(label);
  hintEl = document.createElement('div');
  hintEl.className = 'audio-head-hint';
  meta.append(title, hintEl);
  head.appendChild(meta);

  bodyEl = document.createElement('div');
  bodyEl.className = 'audio-body';

  loadingEl = emptyState({ title: t('audio.loading') });
  errorEl = emptyState({
    icon: 'ui/warning',
    title: t('audio.unavailable'),
    body: t('audio.pipewireUnreachable'),
  });
  errorEl.classList.add('hidden');

  refs.sink = buildSection(t('audio.output'), 'sink');
  refs.source = buildSection(t('audio.input'), 'source');

  bodyEl.append(loadingEl, errorEl, refs.sink.card, refs.source.card);
  popup.append(head, bodyEl);
  document.body.appendChild(popup);
}

function buildSection(label, target) {
  const card = document.createElement('div');
  card.className = 'audio-card hidden';

  const head = document.createElement('div');
  head.className = 'audio-section-head';
  const title = document.createElement('div');
  title.className = 'audio-section-title';
  title.textContent = label;
  head.appendChild(title);
  if (target === 'sink') {
    head.appendChild(iconButton({
      icon: 'ui/play',
      label: t('audio.playTestSound'),
      variant: 'quiet',
      size: 'sm',
      onClick: playTest,
    }));
  }

  const device = document.createElement('div');
  device.className = 'audio-device';
  const iconEl = icon('hud/volume-0', { size: 18, className: 'audio-leading' });
  const main = document.createElement('div');
  main.className = 'audio-device-main';
  const titleEl = document.createElement('div');
  titleEl.className = 'audio-device-title';
  const subEl = document.createElement('div');
  subEl.className = 'audio-device-sub';
  main.append(titleEl, subEl);
  const pctEl = document.createElement('span');
  pctEl.className = 'audio-pct';
  device.append(iconEl, main, pctEl);

  const controls = document.createElement('div');
  controls.className = 'audio-controls';
  const muteBtn = iconButton({
    icon: target === 'source' ? 'ui/mic-off' : 'hud/volume-muted',
    label: target === 'source' ? t('audio.muteMic') : t('audio.muteOutput'),
    variant: 'quiet',
    size: 'sm',
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
  controls.append(muteBtn, range);

  const listEl = document.createElement('div');
  listEl.className = 'ui-list audio-devices hidden';

  card.append(head, device, controls, listEl);
  return { card, iconEl, titleEl, subEl, pctEl, muteBtn, range, listEl, current: null, listSig: '' };
}

/* ── Data ──────────────────────────────────────────────────── */

async function refresh(force = false) {
  if (!open) return;
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
  requestAnimationFrame(() => reposition());
}

function updateSection(ref, target, nodes, defaultName) {
  const node = currentNode(nodes, defaultName);
  ref.current = node;
  ref.range.disabled = !node;
  ref.muteBtn.disabled = !node;

  if (!node) {
    setIcon(ref.iconEl, nodeIcon(null, target), { size: 18 });
    ref.titleEl.textContent = target === 'sink' ? t('audio.noOutputs') : t('audio.noInputs');
    ref.subEl.textContent = t('audio.pipewireNone');
    ref.pctEl.textContent = '';
    ref.listEl.classList.add('hidden');
    return;
  }

  setIcon(ref.iconEl, nodeIcon(node, target), { size: 18 });
  ref.titleEl.textContent = nodeLabel(node);
  ref.subEl.textContent = nodeSubtitle(node);
  ref.pctEl.textContent = node.muted ? t('audio.muted') : `${node.volume_percent}%`;
  ref.pctEl.classList.toggle('is-over', !node.muted && node.volume_percent > 100);
  // The server tells us the ceiling: 150 % normally, more for the T2 speaker
  // DSP. Input stays at 100 % so a boosted microphone cannot clip the capture.
  ref.range.max = String(target === 'sink' ? (node.max_volume || 150) : 100);
  setButtonIcon(ref.muteBtn, nodeIcon(node, target));
  ref.muteBtn.setAttribute('aria-label', node.muted
    ? (target === 'source' ? t('audio.unmuteMic') : t('audio.unmuteOutput'))
    : (target === 'source' ? t('audio.muteMic') : t('audio.muteOutput')));
  if (!dragging) ref.range.value = String(node.volume_percent);

  renderDeviceList(ref, target, nodes);
}

/** Replace an icon-button's glyph (iconButton has no setter of its own). */
function setButtonIcon(button, name) {
  const glyph = button.querySelector('.ui-icon');
  if (glyph) void setIcon(glyph, name, { size: 16 });
}

function renderDeviceList(ref, target, nodes) {
  if (!nodes || nodes.length <= 1) {
    ref.listEl.classList.add('hidden');
    ref.listSig = '';
    return;
  }
  ref.listEl.classList.remove('hidden');

  const sig = nodes.map((node) => `${node.id}:${node.is_default ? 1 : 0}`).join(',');
  if (sig === ref.listSig) return;
  ref.listSig = sig;

  ref.listEl.textContent = '';
  for (const node of nodes) {
    ref.listEl.appendChild(listItem({
      leading: icon(nodeIcon(node, target), { size: 17, className: 'audio-leading' }),
      title: nodeLabel(node),
      subtitle: nodeSubtitle(node),
      trailing: node.is_default ? icon('ui/check', { size: 14, className: 'audio-check' }) : null,
      onClick: node.is_default ? undefined : () => void setDefault(target, node),
    }));
  }
}

/* ── Interaction ───────────────────────────────────────────── */

function onVolumeInput(target, value) {
  const ref = refs[target];
  dragging = true;
  if (ref) {
    ref.pctEl.textContent = `${value}%`;
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
