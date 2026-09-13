/**
 * fullscreen.js — real (browser) fullscreen for plugin windows.
 *
 * The window title bar's fullscreen button, the window/tray context menus and
 * Alt+Enter all come through here. Fullscreening a window does three things:
 *
 *   1. Moves it into a brand-new workspace, so the fullscreen app is isolated
 *      from the desktop it came from (desktop.js owns that half). That
 *      workspace is sealed while the app is fullscreen: nothing may be moved
 *      into it and the app may not be moved out of it.
 *   2. Puts the window into the desktop's fullscreen state and asks the
 *      browser for real fullscreen on the document. No user gesture (an AI
 *      action, or a browser that refuses) simply leaves the in-app fullscreen
 *      standing — never a broken state.
 *   3. Hides the chrome of the workspace it took over: the top bar goes away
 *      (even with `autohide_bar` off, one fullscreen app owns the screen) and
 *      the window's own title bar — the app name plus close/exit-fullscreen —
 *      docks flush under it, revealed by the same top-edge gesture. The other
 *      bar positions keep their desktop-wide behaviour.
 *
 * This module also owns the desktop-wide chrome reveal (Settings → Desktop →
 * Fullscreen): bar position top/left/right/center, plus an independent autohide
 * switch for the bar and the orb. That behaviour is not fullscreen-only — it
 * governs the whole desktop. fullscreen.css owns the actual hiding.
 *
 * A fullscreen app cannot be moved to another workspace while it is fullscreen,
 * so its dedicated (app-named) workspace stays its own until it leaves
 * fullscreen. Leaving fullscreen — the button again, Escape, or the browser's
 * own exit — hands the window back to the workspace it came from and drops the
 * dedicated one, so the desktop is exactly as it was.
 */
import {
  getFocus, getFullscreen, toggleFullscreen, clearFullscreen,
  isolateInNewWorkspace, restoreFromNewWorkspace,
} from './desktop.js';
import { getImmersive, applyImmersive } from './preferences.js';
import { isPluginActive } from './activePlugins.js';

const EDGE_ZONE = 120;    // px from the docking edge that reveals the top bar
const ORB_ZONE = 200;     // px from the bottom edge that reveals the orb…
const ORB_HALF = 340;     // …within this many px either side of centre
const ENTER_HOLD_MS = 1600; // show the chrome briefly after entering

const body = document.body;

let session = null;             // { plugin, token } while a window is fullscreen
let surfaceProvider = () => []; // active window names (set by tiles.js)
let initialized = false;
let immersive = null;           // cached chrome settings (bar pos + autohide)

let revealUntil = 0;            // timestamp: keep chrome up until then
let revealTimer = null;
let lastX = -1;
let lastY = -1;

/** The chrome settings, cached: this runs on every pointer move. */
function immersiveCfg() {
  if (!immersive) immersive = getImmersive();
  return immersive;
}

/* ── Public state ───────────────────────────────────────────── */

/** True while the browser is actually in fullscreen. */
export function isAppFullscreen() {
  return !!(document.fullscreenElement || document.webkitFullscreenElement);
}

/** The plugin window we took fullscreen, or null. */
export function fullscreenWindow() {
  return session?.plugin || null;
}

export function setFullscreenSurfaceProvider(fn) {
  surfaceProvider = typeof fn === 'function' ? fn : () => [];
}

/* ── Enter / exit ───────────────────────────────────────────── */

function requestFullscreen(el) {
  const fn = el.requestFullscreen || el.webkitRequestFullscreen;
  if (!fn) return Promise.reject(new Error('Fullscreen is not supported'));
  try {
    const out = fn.call(el, { navigationUI: 'hide' });
    return out && typeof out.then === 'function' ? out : Promise.resolve();
  } catch (err) {
    return Promise.reject(err);
  }
}

function leaveFullscreen() {
  const fn = document.exitFullscreen || document.webkitExitFullscreen;
  if (!fn) return Promise.resolve();
  try {
    const out = fn.call(document);
    return out && typeof out.then === 'function' ? out : Promise.resolve();
  } catch (_) {
    return Promise.resolve();
  }
}

/** The desktop half of leaving: give the window its workspace back. */
function endSession(s) {
  if (!s) return;
  if (s.token) restoreFromNewWorkspace(s.plugin, s.token);
  else clearFullscreen();
  body.classList.remove('fs-active', 'fs-top', 'fs-orb');
}

export function enterWindowFullscreen(name) {
  if (!name) return false;
  if (session) return session.plugin === name;

  // A brand-new workspace for the fullscreen app, then the tiled fullscreen
  // state (which doubles as the fallback when the browser refuses).
  const token = isolateInNewWorkspace(name);
  toggleFullscreen(name, true);
  session = { plugin: name, token };
  // The in-app half of the state, independent of whether the browser grants
  // the request: fullscreen.css keys the hidden/relocated chrome off this.
  body.classList.add('fs-active');

  // Must be called synchronously from the user gesture; browsers reject it
  // otherwise. A rejection is not an error — the in-app fullscreen stands and
  // the chrome simply keeps its normal, always-visible behaviour.
  requestFullscreen(document.documentElement).catch(() => {
    window.dispatchEvent(new CustomEvent('fullscreen:change', {
      detail: { fullscreen: false, plugin: name },
    }));
  });
  return true;
}

export function exitWindowFullscreen() {
  if (!session) {
    if (getFullscreen()) clearFullscreen();
    return false;
  }
  const s = session;
  session = null;
  if (isAppFullscreen()) void leaveFullscreen(); // fires fullscreenchange
  dismissChrome();
  endSession(s);
  nudgeResize();
  return true;
}

/** Toggle (or force) real fullscreen for a window. Defaults to the focused one
 *  and falls back to the first open window, matching the old Alt+Enter. */
export function toggleWindowFullscreen(name) {
  if (name == null) name = getFocus() || surfaceProvider()[0] || null;
  if (!name) return false;
  if (session && session.plugin === name) return exitWindowFullscreen(), false;
  if (session) exitWindowFullscreen();
  return enterWindowFullscreen(name);
}

export function toggleWindowFullscreenActive() {
  return toggleWindowFullscreen(getFocus() || surfaceProvider()[0] || null);
}

/* ── Chrome reveal (pointer at the docking edge / bottom centre) ── */

function setReveal(top, orb) {
  body.classList.toggle('fs-top', !!top);
  body.classList.toggle('fs-orb', !!orb);
}

/**
 * The bar and the orb hide and reveal on the whole desktop, not only in
 * fullscreen — the immersion settings are desktop-wide. Each piece is
 * independent, so an always-on bar can sit next to an autohiding orb.
 *
 * A fullscreen window overrides the bar's autohide for the top position only:
 * the app owns the screen while it is fullscreen, so the bar is away until the
 * pointer reaches the top edge whatever the setting says. The window's own
 * title bar docks right under it and rides the very same reveal (fullscreen.css
 * keys it off `fs-top` too).
 */
function evalReveal() {
  const cfg = immersiveCfg();
  const held = Date.now() < revealUntil;
  const vw = window.innerWidth;
  const vh = window.innerHeight;
  const fsActive = body.classList.contains('fs-active');
  const fsTopBar = fsActive && cfg.bar_position === 'top';

  // The bar comes back from whichever edge it docks to.
  let nearBar = false;
  if (cfg.bar_position === 'left') nearBar = lastX >= 0 && lastX <= EDGE_ZONE;
  else if (cfg.bar_position === 'right') nearBar = lastX >= 0 && vw - lastX <= EDGE_ZONE;
  else nearBar = lastY >= 0 && lastY <= EDGE_ZONE;

  const nearOrb = lastY >= 0
    && lastY >= vh - ORB_ZONE
    && Math.abs(lastX - vw / 2) <= ORB_HALF;

  // A fullscreen window hides the bar of its workspace no matter what the
  // desktop-wide autohide preference says.
  const showBar = fsTopBar ? (held || nearBar) : (!cfg.autohide_bar || held || nearBar);
  setReveal(showBar, !cfg.autohide_orb || held || nearOrb);
}

function holdReveal(ms) {
  revealUntil = Date.now() + ms;
  evalReveal();
  clearTimeout(revealTimer);
  revealTimer = setTimeout(() => {
    revealUntil = 0;
    evalReveal();
  }, ms + 40);
}

function dismissChrome() {
  clearTimeout(revealTimer);
  revealUntil = 0;
  body.classList.remove('fs-top', 'fs-orb');
  // A bar/orb the user pinned open must not be dismissed with the rest.
  evalReveal();
}

function onPointerMove(e) {
  // Touch/pen have no hover: fullscreen.css keeps the chrome visible there.
  if (e.pointerType === 'touch') return;
  lastX = e.clientX;
  lastY = e.clientY;
  evalReveal();
}

/* ── Browser fullscreen state ───────────────────────────────── */

function onFullscreenChange() {
  const on = isAppFullscreen();
  body.classList.toggle('fullscreen-active', on);
  window.dispatchEvent(new CustomEvent('fullscreen:change', {
    detail: { fullscreen: on, plugin: session?.plugin || null },
  }));

  if (on) {
    holdReveal(ENTER_HOLD_MS);
    nudgeResize();
    return;
  }

  // Escape (or any other browser-side exit): leave the desktop the way we
  // found it. `session` is null when we exited ourselves, so this runs once.
  dismissChrome();
  const s = session;
  session = null;
  endSession(s);
  nudgeResize();
}

/** A full-bleed window changes the map's box; Leaflet needs an invalidateSize. */
function nudgeResize() {
  window.dispatchEvent(new Event('map:resize'));
  requestAnimationFrame(() => window.dispatchEvent(new Event('map:resize')));
  setTimeout(() => window.dispatchEvent(new Event('map:resize')), 220);
}

export function initFullscreen() {
  if (initialized) return;
  initialized = true;
  // Publish the immersion settings (bar position + autohide) to :root before
  // the first reveal is ever evaluated.
  applyImmersive();
  immersive = getImmersive();
  // `fullscreenchange` is standard; the webkit alias covers older Safari.
  document.addEventListener('fullscreenchange', onFullscreenChange);
  document.addEventListener('webkitfullscreenchange', onFullscreenChange);
  window.addEventListener('pointermove', onPointerMove, { passive: true });
  window.addEventListener('resize', evalReveal);
  // Alt+Enter lives in desktop.js; it only announces the gesture.
  window.addEventListener('app:fullscreen-toggle', () => toggleWindowFullscreenActive());
  // Settings changed (Settings page → Desktop): re-reveal under the new rules.
  window.addEventListener('desktop:immersive', (e) => {
    immersive = e.detail || getImmersive();
    evalReveal();
  });
  // Alt-tabbing away is not a hover: put the chrome back out of the way.
  window.addEventListener('blur', dismissChrome);
  // A fullscreen app that gets deactivated (its window button, the tray, the
  // Plugins page) is gone: leaving fullscreen hands its workspace back rather
  // than leaving the desktop sealed around an empty screen.
  window.addEventListener('plugins:changed', () => {
    if (session && !isPluginActive(session.plugin)) exitWindowFullscreen();
  });
  // Show the chrome briefly on load, so an autohidden bar/orb is not a
  // mystery the first time the desktop appears.
  holdReveal(2600);
}
