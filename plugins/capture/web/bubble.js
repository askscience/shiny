/**
 * capture/bubble.js — the webcam bubble: a floating, draggable,
 * resizable picture of the camera above every window.
 *
 * It is also what the recording shows: the engine (Rust) draws the camera
 * at exactly this rectangle with exactly this roundness, so the bubble in
 * the video is sharp and on time even when this element lags a frame. The
 * picture here comes from the engine too (one owner for the camera), as an
 * MJPEG stream the browser decodes in hardware.
 */

import { apiFetch } from '../../js/api.js';
import { t } from '../../js/i18n.js';

let el = null;
let img = null;
let state = { size: 240, roundness: 1, mirror: true, border: 0, x: null, y: null, device: '' };
const listeners = new Set();
let fallbackTimer = 0;
let streamUrl = '';

/** Screen pixels for a client rect (the kiosk webview sits at the screen origin). */
export function toScreenRect(r) {
  const dpr = window.devicePixelRatio || 1;
  const ox = window.screenX || 0;
  const oy = window.screenY || 0;
  return {
    x: Math.round((r.left + ox) * dpr),
    y: Math.round((r.top + oy) * dpr),
    w: Math.round(r.width * dpr),
    h: Math.round(r.height * dpr),
  };
}

export function bubbleState() {
  return { ...state };
}

/** The bubble's client rect, or null when hidden. */
export function bubbleRect() {
  return el && !el.hidden ? el.getBoundingClientRect() : null;
}

export function onBubbleChange(fn) {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

function emit() {
  for (const fn of listeners) { try { fn(bubbleState()); } catch (_) { /* noop */ } }
}

function clampPos() {
  const s = state.size;
  const maxX = Math.max(0, window.innerWidth - s);
  const maxY = Math.max(0, window.innerHeight - s);
  if (state.x == null || state.y == null) {
    state.x = maxX - 24;
    state.y = maxY - 24;
  }
  state.x = Math.min(maxX, Math.max(0, state.x));
  state.y = Math.min(maxY, Math.max(0, state.y));
}

function paint() {
  if (!el) return;
  clampPos();
  const s = state.size;
  el.style.width = `${s}px`;
  el.style.height = `${s}px`;
  el.style.transform = `translate(${state.x}px, ${state.y}px)`;
  el.style.borderRadius = `${(state.roundness * s) / 2}px`;
  el.style.setProperty('--bubble-border', `${state.border || 0}px`);
  img.style.transform = state.mirror ? 'scaleX(-1)' : '';
}

function startStream() {
  const dpr = window.devicePixelRatio || 1;
  const px = Math.round(state.size * dpr * 2);
  const q = new URLSearchParams({ w: String(px), h: String(px) });
  if (state.device) q.set('device', state.device);
  const url = `/api/capture/camera.mjpeg?${q}`;
  if (url === streamUrl) return;
  streamUrl = url;
  stopFallback();
  img.onerror = () => startFallback(q);
  img.src = url;
}

/* Without the session cookie an <img> cannot authenticate: poll single
   frames through the API instead (still the engine's camera). */
function startFallback(q) {
  stopFallback();
  let busy = false;
  let lastUrl = '';
  fallbackTimer = setInterval(async () => {
    if (busy || !el || el.hidden) return;
    busy = true;
    try {
      const blob = await apiFetch(`/api/capture/camera.jpg?${q}`, { responseType: 'blob' });
      if (blob?.size) {
        const u = URL.createObjectURL(blob);
        img.src = u;
        if (lastUrl) URL.revokeObjectURL(lastUrl);
        lastUrl = u;
      }
    } catch (_) { /* camera busy */ }
    busy = false;
  }, 90);
}

function stopFallback() {
  if (fallbackTimer) clearInterval(fallbackTimer);
  fallbackTimer = 0;
}

function build() {
  el = document.createElement('div');
  el.className = 'capture-bubble';
  el.setAttribute('role', 'img');
  el.setAttribute('aria-label', t('plugin.capture.camera.label', null, 'Webcam'));
  img = document.createElement('img');
  img.className = 'capture-bubble-img';
  img.alt = '';
  img.draggable = false;
  const grip = document.createElement('span');
  grip.className = 'capture-bubble-grip';
  grip.title = t('plugin.capture.camera.resize', null, 'Drag to resize');
  el.append(img, grip);
  document.body.appendChild(el);

  let drag = null;
  el.addEventListener('pointerdown', (e) => {
    if (e.button !== 0) return;
    e.preventDefault();
    el.setPointerCapture(e.pointerId);
    const resize = e.target === grip;
    drag = { resize, x0: e.clientX, y0: e.clientY, x: state.x, y: state.y, size: state.size };
    el.classList.add('is-dragging');
  });
  el.addEventListener('pointermove', (e) => {
    if (!drag) return;
    const dx = e.clientX - drag.x0;
    const dy = e.clientY - drag.y0;
    if (drag.resize) state.size = Math.round(Math.min(560, Math.max(96, drag.size + Math.max(dx, dy))));
    else { state.x = drag.x + dx; state.y = drag.y + dy; }
    paint();
  });
  const end = (e) => {
    if (!drag) return;
    const was = drag;
    drag = null;
    el.classList.remove('is-dragging');
    try { el.releasePointerCapture(e.pointerId); } catch (_) { /* noop */ }
    if (was.resize) startStream();
    emit();
  };
  el.addEventListener('pointerup', end);
  el.addEventListener('pointercancel', end);
  window.addEventListener('resize', paint);
}

export function showBubble(opts = {}) {
  state = { ...state, ...Object.fromEntries(Object.entries(opts).filter(([, v]) => v !== undefined)) };
  if (!el) build();
  el.hidden = false;
  paint();
  startStream();
}

export function hideBubble() {
  if (!el) return;
  stopFallback();
  img.removeAttribute('src');
  streamUrl = '';
  el.hidden = true;
  apiFetch('/api/capture/camera/release', { method: 'POST', body: '{}' }).catch(() => {});
}

export function isBubbleVisible() {
  return !!el && !el.hidden;
}
