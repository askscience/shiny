/**
 * touchbar.js — wire Touch Bar input to desktop actions.
 *
 * This module is the single place that turns a Touch Bar press into something
 * the app does. Two sources feed it, both harmless on a machine without a
 * Touch Bar:
 *
 *  - `touchbar:action` CustomEvents — the native macOS bar (`peakd`) evaluates
 *    these in the page.
 *  - `F13`–`F21` keydowns — the Linux T2 `tiny-dfr` buttons, which emit plain
 *    uinput key codes.
 *
 * It is active only when `isTouchBarActive()` says so, so a normal PC that
 * never sees those keys behaves exactly as before. The `talk` action is passed
 * in by `app.js` so it reuses the orb's exact gesture logic (voice-readiness,
 * barge-in, wake handling) rather than a second copy of it.
 */

import { apiFetch } from './api.js';
import { stopActiveTurn } from './agent.js';
import { switchWorkspace } from './desktop.js';
import { openCoreWindow } from './tiles.js';
import { toggleKeyboard } from './keyboard.js';
import { currentSink } from './audioShared.js';
import { toast } from '../ui/index.js';
import { getTouchBarEnabled } from './preferences.js';
import { actionForEvent, isTouchBarActive } from './touchbarShared.js';

let handlers = {};
let installed = false;

// The kiosk shell (`peakd`) sets `window.__shinyTouchBar` before any module
// runs; the server's `/api/touchbar` probe refines it for a plain browser.
let hostFlag = window.__shinyTouchBar === true;

/**
 * Install the Touch Bar listeners. Safe to call more than once (the second
 * call only swaps the handlers); the listeners themselves are registered once.
 */
export function initTouchBar(next = {}) {
  handlers = next || {};
  if (installed) {
    refreshTouchBar();
    return;
  }
  installed = true;
  window.addEventListener('keydown', onKeyDown, true);
  window.addEventListener('touchbar:action', onHostAction);
  window.addEventListener('touchbar:settings', refreshTouchBar);
  refreshTouchBar();
  void probeHost();
}

/** True when a Touch Bar is present (kiosk flag or server detection). */
export function hostHasTouchBar() {
  return hostFlag;
}

/**
 * Ask the server whether this machine has a T2 Touch Bar. The kiosk sets the
 * flag directly, but a plain browser does not — and the server runs on the
 * same machine, so it can see the hardware. A 404 (older server) or an offline
 * server simply leaves the kiosk flag as it was.
 */
async function probeHost() {
  try {
    const res = await apiFetch('/api/touchbar', { authRedirect: false });
    const available = res?.data?.available === true;
    if (available !== hostFlag) {
      hostFlag = available;
      refreshTouchBar();
      window.dispatchEvent(new CustomEvent('touchbar:settings'));
    }
  } catch (_) { /* no server probe: keep whatever the host told us */ }
}

/** Re-read the preference and publish the resolved state for the UI. */
export function refreshTouchBar() {
  const on = active();
  const root = document.documentElement;
  root.dataset.touchbar = on ? '1' : '0';
  root.dataset.touchbarHost = hostFlag ? '1' : '0';
}

function active() {
  return isTouchBarActive(getTouchBarEnabled(), hostFlag);
}

function onKeyDown(event) {
  if (!active()) return;
  const action = actionForEvent(event);
  if (!action) return;
  event.preventDefault();
  event.stopImmediatePropagation();
  void dispatchTouchBarAction(action);
}

function onHostAction(event) {
  if (!active()) return;
  const action = event?.detail?.action;
  if (action) void dispatchTouchBarAction(action);
}

/** Run one named action. Exported so the settings preview can try them. */
export async function dispatchTouchBarAction(action) {
  try {
    switch (action) {
      case 'talk':
        return handlers.talk ? await handlers.talk() : undefined;
      case 'stop':
        return await stopActiveTurn('touchbar');
      case 'mute':
        return await toggleMute();
      case 'volume-down':
        return await nudgeVolume(-5);
      case 'volume-up':
        return await nudgeVolume(5);
      case 'screen-down':
        return await adjustScreenBrightness(-10);
      case 'screen-up':
        return await adjustScreenBrightness(10);
      case 'workspace-prev':
        return void switchWorkspace('prev');
      case 'workspace-next':
        return void switchWorkspace('next');
      case 'kbd-backlight-down':
        return await adjustKeyboardBacklight(-10);
      case 'kbd-backlight-up':
        return await adjustKeyboardBacklight(10);
      case 'keyboard':
        return void toggleKeyboard();
      case 'settings':
        return void openCoreWindow('settings');
      default:
        return undefined;
    }
  } catch (err) {
    toast(err?.message || 'Touch Bar action failed', { type: 'error' });
  }
}

/* ── Host audio ───────────────────────────────────────────────
 * The Sound chip owns the menu, but a Touch Bar press can arrive with the
 * menu closed, so the current sink is read fresh and the same endpoints the
 * menu posts to are used. No audio server → no output node → a silent no-op.
 * ─────────────────────────────────────────────────────────── */

async function currentOutput() {
  const res = await apiFetch('/api/audio/status', { authRedirect: false });
  return currentSink(res?.data);
}

async function nudgeVolume(delta) {
  const sink = await currentOutput();
  if (!sink) return;
  const percent = Math.max(0, Math.min(100, Math.round((sink.volume_percent || 0) + delta)));
  await apiFetch('/api/audio/volume', {
    method: 'POST',
    body: JSON.stringify({ target: 'sink', id: sink.id, percent }),
  });
}

async function toggleMute() {
  const sink = await currentOutput();
  if (!sink) return;
  await apiFetch('/api/audio/mute', {
    method: 'POST',
    body: JSON.stringify({ target: 'sink', id: sink.id, muted: !sink.muted }),
  });
}

/* ── Host backlights ──────────────────────────────────────────
 * The Mac's `kbd_backlight` LED and `gmux_backlight` panel, via the server (a
 * browser cannot write sysfs). No device → the server answers
 * `available: false` and this is a silent no-op; a device that is not writable
 * throws with the fix, which the dispatcher toasts.
 * ─────────────────────────────────────────────────────────── */

async function adjustKeyboardBacklight(delta) {
  await apiFetch('/api/keyboard/backlight', {
    method: 'POST',
    body: JSON.stringify({ delta }),
  });
}

async function adjustScreenBrightness(delta) {
  await apiFetch('/api/screen/brightness', {
    method: 'POST',
    body: JSON.stringify({ delta }),
  });
}
