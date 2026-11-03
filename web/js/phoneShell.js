/**
 * phoneShell.js — the phone's home screen, its bar and one-finger gestures.
 *
 * A phone shows one window at a time (tiles.js). On top of that this module
 * gives it the shape of a mobile OS:
 *
 *  - Home screen: with no window on screen (none open, or the user went
 *    home) the launcher grid is the home screen. It stays up until a window
 *    takes the screen — nothing else dismisses it (launcher.js `isHome`).
 *  - The bar: one thin strip along the bottom edge with a line in it, and no
 *    buttons. It is both the way around and the assistant (the voice bar does
 *    not show on a phone; the line takes its colour while the assistant is
 *    busy):
 *      tap                    → home (while listening: stop listening)
 *      press and hold         → window switcher (the overview)
 *      swipe up               → the chat (type to the assistant)
 *      swipe up and hold      → talk (listen once)
 *      drag left / right      → the window follows the finger and the
 *                               next / previous one comes in beside it
 *                               (the workspace carousel's gesture); let go
 *                               past half-way, or flick, to switch
 *    With the AI switched off a swipe up goes home and a swipe up and hold
 *    opens the switcher.
 *  - Back: with a window on screen the browser's Back (Android's back
 *    gesture) goes home instead of leaving the page — one history entry is
 *    held while a window is up and given back on the way home.
 *
 * The assistant's actions come from app.js (`setPhoneBarActions`), which owns
 * the voice bar's own. Everything is keyed to the phone breakpoint
 * (`PHONE_QUERY`, the one tiles.js uses for its one-window layout) and
 * mirrored on `html[data-phone]` for css/mobile.css.
 */

import { PHONE_QUERY } from './viewport.js';
import { showPhoneHome, stepPhoneWindow, beginPhoneSlide } from './tiles.js';
import { openLauncher, closeLauncher, isLauncherActive } from './launcher.js';
import { openOverview, closeOverview, isOverviewActive } from './overview.js';
import { closeControlCenter } from './controlCenter.js';
import { t } from './i18n.js';

/** Finger travel (px) that makes a swipe. */
const SWIPE_PX = 36;
/** Sideways travel (px) that switches window. */
const SIDE_PX = 60;
/** A pause this long, once swiping up, is "swipe up and hold". */
const HOLD_MS = 320;
/** Movement (px) that counts as "still moving", not holding. */
const STILL_PX = 8;
/** Pressing the bar this long without moving opens the switcher. */
const LONG_PRESS_MS = 450;
/** After the home launcher closed without a window coming (a failed
 *  activation, say), put the home screen back after this long. */
const HOME_RECOVER_MS = 1500;
/** Voice states in which a tap on the bar stops listening. */
const LISTENING = new Set(['listening', 'conversation']);

let dock = null;
/** A history entry is held for the window on screen (Back goes home). */
let heldEntry = false;
/** The launcher is up because it is the home screen. */
let homeLaunched = false;
let recoverTimer = null;
let ready = false;
/** The assistant's actions (app.js); null until wired, or with the AI off. */
let actions = { chat: null, voice: null, enabled: () => false };

function root() { return document.documentElement; }

function isPhone() { return PHONE_QUERY.matches; }

function isHome() { return isPhone() && root().dataset.phoneHome === '1'; }

/**
 * The assistant behind the bar: `chat()` opens the keyboard, `voice()` listens
 * once (or stops a listen in progress), `enabled()` says whether the AI is on.
 */
export function setPhoneBarActions({ chat = null, voice = null, enabled = () => true } = {}) {
  actions = { chat, voice, enabled };
}

function aiOn() {
  try { return !!actions.enabled?.(); } catch (_) { return false; }
}

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

/** Swipe up: the chat, or home with the AI off. */
function swipeUp() {
  closeControlCenter();
  if (aiOn() && actions.chat) {
    if (isOverviewActive()) closeOverview();
    actions.chat();
  } else {
    goHome();
  }
}

/** Swipe up and hold: talk, or the switcher with the AI off. */
function swipeHold() {
  navigator.vibrate?.(10);
  if (aiOn() && actions.voice) {
    closeControlCenter();
    if (isOverviewActive()) closeOverview();
    actions.voice();
  } else {
    toggleSwitcher();
  }
}

/** Tap: home — unless the assistant is listening, then the tap stops it. */
function tap() {
  if (aiOn() && actions.voice && LISTENING.has(root().dataset.voice)) {
    actions.voice();
    return;
  }
  goHome();
}

/* ── Gestures ──────────────────────────────────────────────── */

let gesture = null;

function endGesture() {
  if (!gesture) return;
  clearTimeout(gesture.holdTimer);
  clearTimeout(gesture.pressTimer);
  // A slide still under the finger (a cancelled pointer) rolls back.
  gesture.slide?.cancel();
  gesture = null;
}

/** Progress of a sideways drag: the share of the screen travelled. */
function slideProgress(g, x) {
  return (g.slideDir * (g.x - x)) / Math.max(1, window.innerWidth);
}

function onPointerDown(e) {
  if (!isPhone() || e.button > 0) return;
  endGesture();
  const g = {
    id: e.pointerId,
    x: e.clientX,
    y: e.clientY,
    lastX: e.clientX,
    lastY: e.clientY,
    holdTimer: null,
    pressTimer: null,
    moved: false,
    done: false,
    /** The window slide this drag drives, once it turned sideways. */
    slide: null,
    slideDir: 0,
    /** Recent progress samples, for the release velocity. */
    samples: [],
  };
  // Held still: the switcher.
  g.pressTimer = setTimeout(() => {
    if (gesture !== g || g.moved || g.done) return;
    g.done = true;
    navigator.vibrate?.(10);
    toggleSwitcher();
  }, LONG_PRESS_MS);
  gesture = g;
  try { dock.setPointerCapture(e.pointerId); } catch (_) { /* synthetic events */ }
}

function onPointerMove(e) {
  const g = gesture;
  if (!g || e.pointerId !== g.id || g.done) return;
  const up = g.y - e.clientY;
  const side = e.clientX - g.x;
  if (!g.moved && Math.hypot(up, side) > STILL_PX * 2) {
    g.moved = true;
    clearTimeout(g.pressTimer);
    // Sideways first: the window follows the finger from here on.
    if (Math.abs(side) > Math.abs(up)) {
      g.slideDir = side < 0 ? 1 : -1;
      closeControlCenter();
      g.slide = beginPhoneSlide(g.slideDir);
      if (!g.slide) g.slideDir = 0;
    }
  }
  if (g.slide) {
    const p = slideProgress(g, e.clientX);
    g.slide.update(p);
    const now = performance.now();
    g.samples.push({ p, t: now });
    while (g.samples.length > 2 && now - g.samples[0].t > 100) g.samples.shift();
    return;
  }
  if (up > SWIPE_PX && up > Math.abs(side)) {
    // Swiping up: a pause here is the "hold".
    const moved = Math.hypot(e.clientX - g.lastX, e.clientY - g.lastY);
    if (moved > STILL_PX || !g.holdTimer) {
      g.lastX = e.clientX;
      g.lastY = e.clientY;
      clearTimeout(g.holdTimer);
      g.holdTimer = setTimeout(() => {
        if (gesture !== g || g.done) return;
        g.done = true;
        swipeHold();
      }, HOLD_MS);
    }
  }
}

function onPointerUp(e) {
  const g = gesture;
  if (!g || e.pointerId !== g.id) return;
  const up = g.y - e.clientY;
  const side = e.clientX - g.x;
  if (g.slide) {
    // Commit past half-way or on a flick towards the neighbour.
    const first = g.samples[0];
    const last = g.samples[g.samples.length - 1];
    const dt = first && last ? (last.t - first.t) / 1000 : 0;
    const velocity = dt > 0 ? (last.p - first.p) / dt : 0;
    const { slide } = g;
    g.slide = null;
    endGesture();
    slide.update(slideProgress(g, e.clientX));
    slide.end(velocity);
    return;
  }
  endGesture();
  if (g.done) return;
  if (!g.moved) {
    tap();
  } else if (up > SWIPE_PX && up > Math.abs(side)) {
    swipeUp();
  } else if (Math.abs(side) > SIDE_PX && Math.abs(side) > Math.abs(up)) {
    closeControlCenter();
    // Like turning pages: a swipe to the left brings the next window.
    stepPhoneWindow(side < 0 ? 1 : -1);
  }
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
  const label = t('hud.phoneBar', null, 'Home — hold for open windows, swipe up for the chat');
  dock.title = label;
  dock.setAttribute('aria-label', label);
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
  dock = document.createElement('button');
  dock.type = 'button';
  dock.id = 'phone-dock';
  dock.className = 'phone-dock';
  // Keyboard and screen readers: Enter goes home, the context menu key (or a
  // long press) opens the switcher.
  dock.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); goHome(); }
  });
  dock.addEventListener('contextmenu', (e) => { e.preventDefault(); });
  dock.addEventListener('pointerdown', onPointerDown);
  dock.addEventListener('pointermove', onPointerMove);
  dock.addEventListener('pointerup', onPointerUp);
  dock.addEventListener('pointercancel', endGesture);
  document.body.appendChild(dock);
  relabel();

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
