import { initOrbCanvas, setOrbPalette, setOrbIntensity, resetOrbIntensity } from './orbCanvas.js';

const container = document.getElementById('sphere-container');
const canvas = document.getElementById('orb-canvas');

const LONG_PRESS_MS = 400;
let currentState = 'idle';
let pressTimer = null;
let pressStart = 0;
let conversationMode = false;
let voiceReady = false;
let pointerId = null;

const DOUBLE_TAP_MS = 320;
let lastTapAt = 0;
let singleTapTimer = null;

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
  const paletteState = state === 'idle' ? 'idle' : state;
  setOrbPalette(paletteState);
}

export function setOrbCaption(_text) {
  /* caption removed from chrome */
}

export function getSphereState() {
  return currentState;
}

export function isConversationMode() {
  return conversationMode;
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
  } else if (ready && (currentState === 'disabled' || currentState === 'downloading')) {
    setSphereState('idle');
  }
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
      callbacks.onShortTap?.();
      lastTapAt = 0;
    }
    singleTapTimer = null;
  }, DOUBLE_TAP_MS);
}

function handleShortTapGesture() {
  const now = Date.now();
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
  if (!voiceReady) return;
  e.preventDefault();
  container.classList.add('pressed');
  if (container.setPointerCapture && e.pointerId != null) {
    try {
      container.setPointerCapture(e.pointerId);
      pointerId = e.pointerId;
    } catch (_) {}
  }
  pressStart = Date.now();
  pressTimer = setTimeout(() => {
    pressTimer = null;
    if (!conversationMode) {
      conversationMode = true;
      setSphereState('conversation');
      callbacks.onLongPressStart?.();
    }
  }, LONG_PRESS_MS);
}

function handleEnd(e) {
  e.preventDefault();
  container.classList.remove('pressed');
  if (pointerId != null && container.releasePointerCapture) {
    try { container.releasePointerCapture(pointerId); } catch (_) {}
    pointerId = null;
  }
  const duration = Date.now() - pressStart;
  if (pressTimer) {
    clearTimeout(pressTimer);
    pressTimer = null;
    if (duration < LONG_PRESS_MS) {
      if (conversationMode) {
        conversationMode = false;
        setSphereState('idle');
        callbacks.onLongPressEnd?.();
      } else {
        handleShortTapGesture();
      }
    }
  } else if (conversationMode && duration < LONG_PRESS_MS) {
    conversationMode = false;
    setSphereState('idle');
    callbacks.onLongPressEnd?.();
  }
}

export function initSphere() {
  if (canvas) initOrbCanvas(canvas);
  container.addEventListener('pointerdown', handleStart);
  container.addEventListener('pointerup', handleEnd);
  container.addEventListener('pointercancel', handleEnd);
  setSphereState('disabled');
}

export function setMicLevel(level) {
  setOrbIntensity(level);
}

export function resetMicLevel() {
  resetOrbIntensity();
}
