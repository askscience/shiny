/**
 * phoneShell.js — the phone's home screen, dock and one-finger gestures.
 *
 * A phone shows one window at a time (tiles.js). On top of that this module
 * gives it the shape of a mobile OS:
 *
 *  - Home screen: with no window on screen (none open, or the user went
 *    home) the launcher grid is the home screen. It stays up until a window
 *    takes the screen — nothing else dismisses it (launcher.js `isHome`).
 *  - Dock: the strip along the bottom edge, around the voice bar, with a
 *    handle under it. No buttons: the handle is the control —
 *      tap the handle         → home
 *      press and hold it      → window switcher (the overview)
 *    and the whole strip takes the swipes —
 *      swipe up               → home
 *      swipe up and hold      → window switcher (also: a long swipe up)
 *      swipe left / right     → next / previous window
 *  - Back: with a window on screen the browser's Back (Android's back
 *    gesture) goes home instead of leaving the page — one history entry is
 *    held while a window is up and given back on the way home.
 *
 * Everything is keyed to the phone breakpoint (`PHONE_QUERY`, the one
 * tiles.js uses for its one-window layout) and mirrored on `html[data-phone]`
 * for css/mobile.css.
 */

import { PHONE_QUERY } from './viewport.js';
import { showPhoneHome, stepPhoneWindow } from './tiles.js';
import { openLauncher, closeLauncher, isLauncherActive } from './launcher.js';
import { openOverview, closeOverview, isOverviewActive } from './overview.js';
import { closeControlCenter } from './controlCenter.js';
import { t } from './i18n.js';

/** Finger travel (px) that makes a swipe. */
const SWIPE_PX = 36;
/** Sideways travel (px) that switches window. */
const SIDE_PX = 60;
/** A pause this long, once swiping up, opens the switcher. */
const HOLD_MS = 320;
/** Movement (px) that counts as "still moving", not holding. */
const STILL_PX = 8;
/** An upward swipe longer than this share of the screen opens the switcher. */
const LONG_SWIPE = 0.3;
/** Holding the handle this long opens the switcher. */
const LONG_PRESS_MS = 450;
/** After the home launcher closed without a window coming (a failed
 *  activation, say), put the home screen back after this long. */
const HOME_RECOVER_MS = 1500;

let dock = null;
let handle = null;
/** A history entry is held for the window on screen (Back goes home). */
let heldEntry = false;
/** The launcher is up because it is the home screen. */
let homeLaunched = false;
let recoverTimer = null;
let ready = false;

function root() { return document.documentElement; }

function isPhone() { return PHONE_QUERY.matches; }

function isHome() { return isPhone() && root().dataset.phoneHome === '1'; }

/** Keep the launcher in step with the home screen. */
function syncHome() {
  const home = isHome();
  if (home) root().dataset.launcherHome = '1';
  else delete root().dataset.launcherHome;
  if (!ready) return;
  if (home) {
    if (!isLauncherActive() && !isOverviewActive()) {
      homeLaunched = true;
      openLauncher();
    }
  } else if (homeLaunched && isLauncherActive()) {
    homeLaunched = false;
    closeLauncher();
  }
}

function goHome() {
  closeControlCenter();
  if (isOverviewActive()) {
    closeOverview(() => { showPhoneHome(); syncHome(); });
    return;
  }
  showPhoneHome();
  syncHome();
}

function toggleSwitcher() {
  closeControlCenter();
  if (isOverviewActive()) closeOverview();
  else openOverview();
}

/* ── Gestures ──────────────────────────────────────────────── */

let gesture = null;

function endGesture() {
  if (!gesture) return;
  clearTimeout(gesture.holdTimer);
  gesture = null;
}

function onPointerDown(e) {
  if (!isPhone() || e.button > 0) return;
  if (e.target instanceof Element && e.target.closest('.phone-dock-handle') && e.target !== dock) return;
  endGesture();
  gesture = {
    id: e.pointerId,
    x: e.clientX,
    y: e.clientY,
    lastX: e.clientX,
    lastY: e.clientY,
    holdTimer: null,
    done: false,
  };
  try { dock.setPointerCapture(e.pointerId); } catch (_) { /* synthetic events */ }
}

function onPointerMove(e) {
  if (!gesture || e.pointerId !== gesture.id || gesture.done) return;
  const up = gesture.y - e.clientY;
  const side = e.clientX - gesture.x;
  if (up > SWIPE_PX && up > Math.abs(side)) {
    if (up > window.innerHeight * LONG_SWIPE) {
      gesture.done = true;
      clearTimeout(gesture.holdTimer);
      toggleSwitcher();
      return;
    }
    // Swiping up: a pause here is the "hold" that asks for the switcher.
    const moved = Math.hypot(e.clientX - gesture.lastX, e.clientY - gesture.lastY);
    if (moved > STILL_PX || !gesture.holdTimer) {
      gesture.lastX = e.clientX;
      gesture.lastY = e.clientY;
      clearTimeout(gesture.holdTimer);
      gesture.holdTimer = setTimeout(() => {
        if (!gesture || gesture.done) return;
        gesture.done = true;
        toggleSwitcher();
      }, HOLD_MS);
    }
  }
}

function onPointerUp(e) {
  if (!gesture || e.pointerId !== gesture.id) return;
  const { done } = gesture;
  const up = gesture.y - e.clientY;
  const side = e.clientX - gesture.x;
  endGesture();
  if (done) return;
  if (up > SWIPE_PX && up > Math.abs(side)) {
    goHome();
  } else if (Math.abs(side) > SIDE_PX && Math.abs(side) > Math.abs(up)) {
    closeControlCenter();
    // Like turning pages: a swipe to the left brings the next window.
    stepPhoneWindow(side < 0 ? 1 : -1);
  }
}

/* ── The handle: tap = home, hold = switcher ──────────────── */

let press = null;

function onHandleDown(e) {
  if (!isPhone() || e.button > 0) return;
  e.stopPropagation();
  clearTimeout(press?.timer);
  press = {
    id: e.pointerId,
    x: e.clientX,
    y: e.clientY,
    held: false,
    timer: setTimeout(() => {
      if (!press) return;
      press.held = true;
      navigator.vibrate?.(10);
      toggleSwitcher();
    }, LONG_PRESS_MS),
  };
  try { handle.setPointerCapture(e.pointerId); } catch (_) { /* synthetic events */ }
}

function onHandleMove(e) {
  if (!press || e.pointerId !== press.id || press.held) return;
  // A finger that travels is a swipe, which the dock reads on its own.
  if (Math.hypot(e.clientX - press.x, e.clientY - press.y) > STILL_PX * 2) {
    clearTimeout(press.timer);
    const start = press;
    press = null;
    try { handle.releasePointerCapture(e.pointerId); } catch (_) { /* gone */ }
    onPointerDown({ pointerId: start.id, clientX: start.x, clientY: start.y, button: 0, target: dock });
    onPointerMove(e);
  }
}

function onHandleUp(e) {
  if (!press || e.pointerId !== press.id) {
    onPointerUp(e);
    return;
  }
  clearTimeout(press.timer);
  const { held } = press;
  press = null;
  if (!held) goHome();
}

function onHandleCancel() {
  clearTimeout(press?.timer);
  press = null;
}

/* ── Back goes home ────────────────────────────────────────── */

function syncHistory() {
  if (!isPhone()) return;
  if (!isHome() && !heldEntry) {
    heldEntry = true;
    history.pushState({ peakdWindow: true }, '');
  } else if (isHome() && heldEntry) {
    // Home by any other way: give the entry back, so Back at home leaves.
    heldEntry = false;
    history.back();
  }
}

function onPopState() {
  if (!heldEntry) return;
  heldEntry = false;
  goHome();
}

/* ── Setup ─────────────────────────────────────────────────── */

function relabel() {
  const label = t('hud.homeHandle', null, 'Home — hold for open windows');
  handle.title = label;
  handle.setAttribute('aria-label', label);
}

function syncScreen() {
  root().dataset.phone = isPhone() ? '1' : '0';
  if (!isPhone()) {
    endGesture();
    delete root().dataset.launcherHome;
  }
  syncHome();
}

export function initPhoneShell() {
  if (dock) return;
  dock = document.createElement('div');
  dock.id = 'phone-dock';
  dock.className = 'phone-dock';
  dock.setAttribute('role', 'toolbar');
  handle = document.createElement('button');
  handle.type = 'button';
  handle.id = 'phone-dock-handle';
  handle.className = 'phone-dock-handle';
  // Keyboard and screen readers: Enter goes home, the context menu key (or
  // a long press) opens the switcher.
  handle.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); goHome(); }
  });
  handle.addEventListener('contextmenu', (e) => { e.preventDefault(); });
  handle.addEventListener('pointerdown', onHandleDown);
  handle.addEventListener('pointermove', onHandleMove);
  handle.addEventListener('pointerup', onHandleUp);
  handle.addEventListener('pointercancel', onHandleCancel);
  dock.append(handle);
  document.body.appendChild(dock);
  relabel();

  dock.addEventListener('pointerdown', onPointerDown);
  dock.addEventListener('pointermove', onPointerMove);
  dock.addEventListener('pointerup', onPointerUp);
  dock.addEventListener('pointercancel', endGesture);

  // The home flag lives on <html> (tiles.js sets it on every layout pass).
  new MutationObserver(() => { syncHome(); syncHistory(); })
    .observe(root(), { attributes: true, attributeFilter: ['data-phone-home'] });
  window.addEventListener('popstate', onPopState);
  window.addEventListener('launcher:closed', () => {
    homeLaunched = false;
    clearTimeout(recoverTimer);
    // A pick closes the home launcher before its window arrives; only if
    // none came is the home screen put back.
    recoverTimer = setTimeout(syncHome, HOME_RECOVER_MS);
  });
  window.addEventListener('overview:closed', () => setTimeout(syncHome, 0));
  window.addEventListener('locale:change', relabel);
  PHONE_QUERY.addEventListener('change', syncScreen);
  syncScreen();

  // The first layout passes run before the remembered windows are back; the
  // home screen waits for them instead of flashing up at boot.
  setTimeout(() => { ready = true; syncHome(); }, 800);
}
