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
import {
  emptyState, icon, iconButton, listItem, slider, toast, setIcon,
} from '../ui/index.js';
import { currentNode, nodeIcon, nodeLabel, nodeSubtitle } from './audioShared.js';

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
  const rect = trigger.getBoundingClientRect();
  const width = popup.offsetWidth;
  const left = Math.max(12, Math.min(rect.right - width, window.innerWidth - width - 12));
  const top = rect.bottom + 8;
  popup.style.left = `${left}px`;
  popup.style.top = `${top}px`;
  popup.style.maxHeight = `${Math.max(220, window.innerHeight - top - 16)}px`;
}

/* ── Skeleton ──────────────────────────────────────────────── */

function ensurePopup() {
  if (popup) return;
  popup = document.createElement('div');
  popup.className = 'ui-hud-menu-popup audio-menu hidden';
  popup.setAttribute('role', 'dialog');
  popup.setAttribute('aria-label', 'Sound');

  const head = document.createElement('div');
  head.className = 'audio-head';
  const meta = document.createElement('div');
  meta.className = 'audio-head-main';
  const title = document.createElement('div');
  title.className = 'audio-head-title';
  title.appendChild(icon('ui/volume', { size: 15 }));
  const label = document.createElement('span');
  label.textContent = 'Sound';
  title.appendChild(label);
  hintEl = document.createElement('div');
  hintEl.className = 'audio-head-hint';
  meta.append(title, hintEl);
  head.appendChild(meta);

  bodyEl = document.createElement('div');
  bodyEl.className = 'audio-body';

  loadingEl = emptyState({ title: 'Loading sound state…' });
  errorEl = emptyState({
    icon: 'ui/warning',
    title: 'Sound panel unavailable',
    body: 'PipeWire is not reachable.',
  });
  errorEl.classList.add('hidden');

  refs.sink = buildSection('Output', 'sink');
  refs.source = buildSection('Input', 'source');

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
      label: 'Play a test sound',
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
    label: target === 'source' ? 'Mute microphone' : 'Mute output',
    variant: 'quiet',
    size: 'sm',
    onClick: () => void toggleMute(target),
  });
  const range = slider({
    min: 0,
    max: 100,
    step: 1,
    value: 0,
    onInput: (value) => onVolumeInput(target, value),
  });
  range.setAttribute('aria-label', target === 'source' ? 'Input volume' : 'Output volume');
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
    hintEl.textContent = status.reason || 'PipeWire is not available';
    const text = errorEl.querySelector('.ui-subtitle');
    if (text) text.textContent = status.reason || 'PipeWire is not reachable.';
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
    ref.titleEl.textContent = target === 'sink' ? 'No output devices' : 'No input devices';
    ref.subEl.textContent = 'PipeWire reports none';
    ref.pctEl.textContent = '';
    ref.listEl.classList.add('hidden');
    return;
  }

  setIcon(ref.iconEl, nodeIcon(node, target), { size: 18 });
  ref.titleEl.textContent = nodeLabel(node);
  ref.subEl.textContent = nodeSubtitle(node);
  ref.pctEl.textContent = node.muted ? 'Muted' : `${node.volume_percent}%`;
  setButtonIcon(ref.muteBtn, nodeIcon(node, target));
  ref.muteBtn.setAttribute('aria-label', node.muted
    ? (target === 'source' ? 'Unmute microphone' : 'Unmute output')
    : (target === 'source' ? 'Mute microphone' : 'Mute output'));
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
  if (ref) ref.pctEl.textContent = `${value}%`;
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
    toast(err.message || 'Could not set the volume', { type: 'error' });
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
    toast(err.message || 'Could not change the mute state', { type: 'error' });
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
    toast(`${target === 'sink' ? 'Output' : 'Input'}: ${nodeLabel(node)}`);
  } catch (err) {
    toast(err.message || 'Could not switch the device', { type: 'error' });
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
    toast('Could not play a test sound', { type: 'error' });
  }
}
