/**
 * sphere.js — the voice bar at the bottom of the screen.
 *
 * There is no rendered orb any more: the assistant's presence is a small lip
 * with an accent-coloured shadow that rises from it and swells with the audio.
 * This module owns the gesture vocabulary (tap = listen, long-press = wake,
 * double-tap = type) and pushes the live audio level into CSS custom
 * properties on the bar. The look lives in /css/sphere.css; the colour follows
 * the active theme's accent (or --error while the mic is unhappy).
 */

const container = document.getElementById('sphere-container');

const LONG_PRESS_MS = 400;
let currentState = 'idle';
let pressTimer = null;
let pressStart = 0;
let conversationMode = false;
// True once the long-press timer has actually fired for the current gesture.
// `pressTimer` is null by then, so release handling needs this flag.
let longPressFired = false;
let voiceReady = false;
/** Resolvers waiting for the next setVoiceReady(true) — see waitForVoiceReady. */
const readyWaiters = new Set();
let pointerId = null;

// Human double-clicks vary a lot (macOS default allows ~500ms between taps).
// Too tight a window lets the first tap's single-tap action fire mid-gesture.
const DOUBLE_TAP_MS = 500;
// If the single-tap action already fired and the second tap lands right after,
// we still convert the pair into a double-tap (the action is cancelled by the
// double-tap handler). Covers slow double-clicks up to ~800ms total.
const LATE_DOUBLE_TAP_MS = 300;
let lastTapAt = 0;
let singleTapTimer = null;
let lastSingleTapFiredAt = 0;
// Set when a second pointerdown arrives while a single-tap action is pending —
// the gesture resolves as a double-tap on pointerup, no matter how slow.
let pendingDoubleTap = false;

const callbacks = {
  onShortTap: null,
  onLongPressStart: null,
  onLongPressEnd: null,
  onDoubleTap: null,
};

export function setSphereState(state) {
  currentState = state;
  [...container.classList].forEach((cls) => {
    if (cls.startsWith('state-')) container.classList.remove(cls);
  });
  if (state !== 'idle') {
    container.classList.add(`state-${state}`);
  }
}

export function getSphereState() {
  return currentState;
}

export function setConversationMode(on) {
  conversationMode = on;
  if (on) {
    setSphereState('conversation');
  } else if (currentState === 'conversation') {
    setSphereState('idle');
  }
}

export function setVoiceReady(ready) {
  voiceReady = ready;
  container.classList.toggle('disabled', !ready);
  if (!ready && currentState === 'idle') {
    setSphereState('disabled');
  } else if (ready && (currentState === 'disabled' || currentState === 'warming')) {
    setSphereState('idle');
  }
  if (ready && readyWaiters.size) {
    const waiters = [...readyWaiters];
    readyWaiters.clear();
    for (const resolve of waiters) resolve(true);
  }
}

/**
 * Resolve true the moment the speech model becomes usable, or false after
 * `timeoutMs`. A gesture that arrives while the bar is still `warming` waits
 * here instead of being dropped with a toast — on a cold login the
 * faster-whisper sidecar needs a few seconds before it can listen.
 */
export function waitForVoiceReady(timeoutMs = 30000) {
  if (voiceReady) return Promise.resolve(true);
  return new Promise((resolve) => {
    let timer = null;
    const settle = (ok) => {
      if (timer) clearTimeout(timer);
      readyWaiters.delete(settle);
      resolve(ok);
    };
    timer = setTimeout(() => settle(false), timeoutMs);
    readyWaiters.add(settle);
  });
}

export function onShortTap(fn) { callbacks.onShortTap = fn; }
export function onLongPressStart(fn) { callbacks.onLongPressStart = fn; }
export function onLongPressEnd(fn) { callbacks.onLongPressEnd = fn; }
export function onDoubleTap(fn) { callbacks.onDoubleTap = fn; }

function scheduleSingleTap() {
  const tapId = Date.now();
  lastTapAt = tapId;
  if (singleTapTimer) clearTimeout(singleTapTimer);
  singleTapTimer = setTimeout(() => {
    if (lastTapAt === tapId) {
      lastSingleTapFiredAt = Date.now();
      callbacks.onShortTap?.();
      lastTapAt = 0;
    }
    singleTapTimer = null;
  }, DOUBLE_TAP_MS);
}

function handleShortTapGesture() {
  const now = Date.now();
  // Slow double-click: the single-tap action just fired, but the second tap
  // is already here — convert the pair into a double-tap.
  if (lastSingleTapFiredAt && now - lastSingleTapFiredAt < LATE_DOUBLE_TAP_MS) {
    lastSingleTapFiredAt = 0;
    lastTapAt = 0;
    callbacks.onDoubleTap?.();
    return;
  }
  if (lastTapAt && now - lastTapAt < DOUBLE_TAP_MS) {
    if (singleTapTimer) {
      clearTimeout(singleTapTimer);
      singleTapTimer = null;
    }
    lastTapAt = 0;
    callbacks.onDoubleTap?.();
    return;
  }
  scheduleSingleTap();
}

function handleStart(e) {
  // Only the primary button drives the voice gestures. A right-click is the
  // context-menu gesture (contextMenu.js), so it must not start a tap/listen.
  if (e.button !== 0) return;
  // Never gate gestures on voice readiness here — the action callbacks decide
  // (typing must always work; voice gestures show their own feedback).
  e.preventDefault();
  container.classList.add('pressed');
  // A second pointerdown while a single-tap is pending = double-tap in
  // progress. Cancel the pending action before it can fire mid-gesture.
  if (singleTapTimer) {
    clearTimeout(singleTapTimer);
    singleTapTimer = null;
    pendingDoubleTap = true;
    lastTapAt = 0;
  }
  if (container.setPointerCapture && e.pointerId != null) {
    try {
      container.setPointerCapture(e.pointerId);
      pointerId = e.pointerId;
    } catch (_) {}
  }
  pressStart = Date.now();
  longPressFired = false;
  pressTimer = setTimeout(() => {
    pressTimer = null;
    if (!conversationMode) {
      conversationMode = true;
      longPressFired = true;
      setSphereState('conversation');
      callbacks.onLongPressStart?.();
    }
  }, LONG_PRESS_MS);
}

function handleEnd(e) {
  if (e.button !== 0) return;
  e.preventDefault();
  container.classList.remove('pressed');
  if (pointerId != null && container.releasePointerCapture) {
    try { container.releasePointerCapture(pointerId); } catch (_) {}
    pointerId = null;
  }
  const duration = Date.now() - pressStart;
  // The long press fired during this gesture: releasing always ends it (the
  // old duration-based branch could never run once the timer had fired).
  if (longPressFired) {
    longPressFired = false;
    conversationMode = false;
    setSphereState('idle');
    callbacks.onLongPressEnd?.();
    return;
  }
  if (pressTimer) {
    clearTimeout(pressTimer);
    pressTimer = null;
    if (duration < LONG_PRESS_MS) {
      if (pendingDoubleTap) {
        pendingDoubleTap = false;
        lastTapAt = 0;
        callbacks.onDoubleTap?.();
      } else {
        handleShortTapGesture();
      }
    }
  }
}

export function initSphere() {
  container.addEventListener('pointerdown', handleStart);
  container.addEventListener('pointerup', handleEnd);
  container.addEventListener('pointercancel', handleEnd);
  setSphereState('disabled');
}

/**
 * Feed the live audio level into the bar. Accepts the `voice:level` detail
 * ({ level, pan }) or a bare number; both are clamped before they reach CSS.
 */
export function setMicLevel(input) {
  let level = 0;
  let pan = 0;
  if (typeof input === 'number') {
    level = input;
  } else if (input && typeof input === 'object') {
    level = Number(input.level);
    pan = Number(input.pan);
  }
  if (Number.isFinite(level)) {
    container.style.setProperty('--voice-level', clamp(level, 0, 1).toFixed(3));
  }
  if (Number.isFinite(pan)) {
    container.style.setProperty('--voice-pan', clamp(pan, -1, 1).toFixed(3));
  }
}

export function resetMicLevel() {
  container.style.setProperty('--voice-level', '0');
  container.style.setProperty('--voice-pan', '0');
}

function clamp(value, min, max) {
  return Math.min(max, Math.max(min, value));
}
