/**
 * fullscreen.js — real (browser) fullscreen for plugin windows.
 *
 * The window title bar's fullscreen button, the window/tray context menus,
 * Alt+Enter and the AI's `desktop_fullscreen` all come through here.
 * Fullscreening a window does three things:
 *
 *   1. Moves it into a brand-new workspace, so the fullscreen app is isolated
 *      from the desktop it came from (desktop.js owns that half). That
 *      workspace is sealed while the app is fullscreen: nothing may be moved
 *      into it and the app may not be moved out of it. Several windows can own
 *      a screen this way at once, one workspace each, and each workspace
 *      remembers where its window came from (`ws.home`) — so a window is handed
 *      back to the right place even after a reload.
 *   2. Gives the window the screen: the desktop's fullscreen state goes to it
 *      and the chrome of the workspace it owns steps aside (`body.fs-active`) —
 *      the top bar goes away (even with `autohide_bar` off, one fullscreen app
 *      owns the screen) and comes back from the top edge once the pointer
 *      insists there. While it is up, the window slides down by the bar's height
 *      so the app's own top bar is never covered. The window gives up its own
 *      title-bar stripe; its close/exit-fullscreen controls move into a glass
 *      bubble on the left of the top bar (fullscreenBubble.js), so the way out
 *      rides the same top-edge reveal as the rest of the chrome. The other bar
 *      positions keep their desktop-wide behaviour.
 *   3. Asks the browser for real fullscreen on the document. That request is the
 *      enhancement, never the effect: one that is refused (an AI action, a
 *      browser without the API) leaves the in-app fullscreen standing, and a
 *      window fullscreened while the document already is requests nothing at all
 *      — leaving and re-requesting is a race the browser always wins, which is
 *      how two fullscreen windows used to fight over the screen. Real fullscreen
 *      is only given up when the last fullscreen window leaves.
 *
 * Leaving fullscreen — the button again, Escape, or the browser's own exit —
 * hands the window back to the workspace it came from and drops the dedicated
 * one, so the desktop is exactly as it was. Escape ends the fullscreen of the
 * window on screen; the fullscreen windows of other workspaces keep theirs.
 *
 * This module also owns the desktop-wide chrome reveal (Settings → Appearance
 * → Top bar & voice bar): bar position top/left/right/center, plus an independent
 * autohide switch for the bar and the voice bar. That behaviour is not
 * fullscreen-only — it
 * governs the whole desktop. fullscreen.css owns the actual hiding.
 */
import {
  getFocus, getFullscreen, toggleFullscreen, clearFullscreen,
  isolateInNewWorkspace, restoreFromNewWorkspace, hasFullscreenWindow,
} from './desktop.js';
import { getImmersive, applyImmersive, effectiveOrbPosition, orbIsAttached } from './preferences.js';
import { isPluginActive } from './activePlugins.js';

const EDGE_ZONE = 120;     // px from the edge that keeps the bar up once shown
const REVEAL_ZONE = 28;    // …but only this close to the very edge summons it
const BAR_DWELL_MS = 700;  // the pointer must *stay* in the reveal zone this long
const ORB_ZONE = 200;      // px from the bottom edge that reveals the voice bar…
const ORB_HALF = 340;      // …within this many px either side of centre
const ENTER_HOLD_MS = 1600; // show the chrome briefly after entering

const body = document.body;

let browserFullscreen = false;  // the browser's last reported state (edge detection)
let ownExit = false;            // we asked the browser to leave; that exit is ours
let surfaceProvider = () => []; // active window names (set by tiles.js)
let initialized = false;
let immersive = null;           // cached chrome settings (bar pos + autohide)

let revealUntil = 0;            // timestamp: keep chrome up until then
let revealTimer = null;
let lastX = -1;
let lastY = -1;
let edgeDwell = false;          // the pointer has insisted at the docking edge
let edgeDwellTimer = null;

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

/** True when the document root itself owns the screen: there is no fullscreen
 *  request left to make (and a second one would be rejected). */
function documentIsFullscreen() {
  return (document.fullscreenElement || document.webkitFullscreenElement)
    === document.documentElement;
}

/** The window that owns the screen of the workspace on view, or null. The
 *  desktop's per-workspace fullscreen state is the truth; the browser's is only
 *  an enhancement, so this needs no session of its own. */
export function fullscreenWindow() {
  return getFullscreen();
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

/** Tell the chrome — and the fullscreen bubble — that the state changed.
 *  `fullscreen` is the browser's state, `plugin` the window on screen's. */
function announce() {
  window.dispatchEvent(new CustomEvent('fullscreen:change', {
    detail: { fullscreen: isAppFullscreen(), plugin: fullscreenWindow() },
  }));
}

/**
 * The chrome takeover follows the window that owns the workspace on view — never
 * the browser's answer, which is only an enhancement. Switching to a workspace
 * with no fullscreen window gives the desktop its chrome back.
 */
function syncFullscreenChrome() {
  body.classList.toggle('fs-active', !!getFullscreen());
  evalReveal();
}

export function enterWindowFullscreen(name) {
  if (!name) return false;
  if (getFullscreen() === name) return true; // already owns this screen

  // A brand-new workspace for the fullscreen app, whose record doubles as the
  // in-app fullscreen state. On a vertical phone screen there is no workspace
  // system to isolate in, so the window simply fills the single column.
  if (!isolateInNewWorkspace(name)) toggleFullscreen(name, true);
  syncFullscreenChrome();
  // The state is already live: show the top-bar bubble now, before the browser
  // transition (or its refusal) round-trips through an event.
  announce();
  holdReveal(ENTER_HOLD_MS);
  nudgeResize();

  // Ask the browser for the real thing — but only when it does not have it
  // already. A window fullscreened while the document is fullscreen needs no
  // request at all, and leaving to re-request is a race the browser always wins:
  // the exit lands after the request, which is how two fullscreen windows ended
  // up fighting over the screen. Must be called synchronously from the user
  // gesture; a rejection is not an error — the in-app fullscreen stands.
  if (!documentIsFullscreen()) {
    requestFullscreen(document.documentElement).catch(() => announce());
  }
  return true;
}

export function exitWindowFullscreen() {
  const name = getFullscreen();
  if (!name) return false;
  // Hand the workspace back (a phone has none to hand back).
  if (!restoreFromNewWorkspace(name)) clearFullscreen();
  syncFullscreenChrome();
  dismissChrome();
  // Real fullscreen is only given up when the last fullscreen window leaves, so
  // exiting one of several never drops another window's screen. `ownExit` marks
  // the event that follows as ours: it must never touch a window fullscreened in
  // the meantime (see `onFullscreenChange`).
  if (!hasFullscreenWindow() && isAppFullscreen()) {
    ownExit = true;
    void leaveFullscreen();
  }
  announce();
  nudgeResize();
  return true;
}

/** Toggle fullscreen for a window. Defaults to the focused one and falls back to
 *  the first open window, matching the old Alt+Enter. */
export function toggleWindowFullscreen(name) {
  if (name == null) name = getFocus() || surfaceProvider()[0] || null;
  if (!name) return false;
  if (getFullscreen() === name) return exitWindowFullscreen(), false;
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
 * How much of the bottom edge is already spoken for by the voice bar: reaching
 * the bar that is *there* is reaching the edge, so the strip starts at its top
 * instead of at the screen's. Zero unless the voice bar shares that edge.
 */
function bottomVoiceBarGap(cfg) {
  const onBottom = effectiveOrbPosition(cfg).startsWith('bottom')
    || (orbIsAttached(cfg) && cfg.bar_position === 'bottom');
  if (!onBottom) return 0;
  const rect = document.getElementById('sphere-container')?.getBoundingClientRect();
  if (!rect || !rect.height) return 0;
  return Math.max(0, window.innerHeight - rect.top);
}

/**
 * The narrow strip along the edge the bar docks to. Only reaching this close
 * summons the bar: a plugin window's own top bar lives at the very top of a
 * fullscreen app, so resting on its buttons must not be enough. The pointer has
 * to insist here (see [`armEdgeDwell`]) before the chrome comes back. At the
 * bottom the strip covers the voice bar too — it sits on that edge, and the
 * pointer lands on it long before it reaches the screen's last pixels.
 */
function barRevealProximity() {
  const cfg = immersiveCfg();
  const vw = window.innerWidth;
  const vh = window.innerHeight;
  if (cfg.bar_position === 'left') return lastX >= 0 && lastX <= REVEAL_ZONE;
  if (cfg.bar_position === 'right') return lastX >= 0 && vw - lastX <= REVEAL_ZONE;
  if (cfg.bar_position === 'bottom') return lastY >= 0 && vh - lastY <= REVEAL_ZONE + bottomVoiceBarGap(cfg);
  return lastY >= 0 && lastY <= REVEAL_ZONE;
}

/**
 * The wider band along the same edge that keeps a *shown* bar up, so the user
 * can move from the edge onto the bar itself (and a little past it) without the
 * chrome vanishing mid-reach.
 */
function barKeepProximity() {
  const cfg = immersiveCfg();
  const vw = window.innerWidth;
  const vh = window.innerHeight;
  if (cfg.bar_position === 'left') return lastX >= 0 && lastX <= EDGE_ZONE;
  if (cfg.bar_position === 'right') return lastX >= 0 && vw - lastX <= EDGE_ZONE;
  if (cfg.bar_position === 'bottom') return lastY >= 0 && vh - lastY <= EDGE_ZONE + bottomVoiceBarGap(cfg);
  return lastY >= 0 && lastY <= EDGE_ZONE;
}

/**
 * Arm, or cancel, the dwell timer. The bar only appears after the pointer has
 * insisted in the reveal strip for [`BAR_DWELL_MS`]; leaving the wider keep band
 * forgets the insistence entirely, so brushing the top while reaching for a
 * window's own toolbar never summons the chrome.
 */
function armEdgeDwell(atEdge, nearBar) {
  if (!nearBar) {
    clearTimeout(edgeDwellTimer);
    edgeDwellTimer = null;
    if (edgeDwell) {
      edgeDwell = false;
      evalReveal();
    }
    return;
  }
  if (edgeDwell || edgeDwellTimer) return; // already up, or already counting down
  if (!atEdge) return;                     // in the band, but not at the very edge
  edgeDwellTimer = setTimeout(() => {
    edgeDwellTimer = null;
    edgeDwell = true;
    evalReveal();
  }, BAR_DWELL_MS);
}

/**
 * The bar and the voice bar hide and reveal on the whole desktop, not only in
 * fullscreen — the immersion settings are desktop-wide. Each piece is
 * independent, so an always-on bar can sit next to an autohiding voice bar —
 * except a voice bar riding the top bar's row: that one is part of the bar and
 * hides and reveals with it.
 *
 * The bar only reveals once the pointer has insisted at its docking edge
 * (`edgeDwell`), so brushing past the top while reaching for a window's own
 * toolbar does not summon it. A fullscreen window still overrides the bar's
 * autohide whatever edge it docks to — top or bottom — because the app owns the
 * screen while it is fullscreen, so the bar is away until the pointer insists
 * at that edge however the setting reads. The window rides the same reveal:
 * fullscreen.css shifts it down by the bar's height so the chrome never covers
 * it.
 */
function evalReveal() {
  const cfg = immersiveCfg();
  const held = Date.now() < revealUntil;
  const vw = window.innerWidth;
  const vh = window.innerHeight;
  const fsActive = body.classList.contains('fs-active');

  // The bar comes back from whichever edge it docks to — only after the
  // pointer has insisted there (or when held up around entering fullscreen).
  const nearBar = edgeDwell;

  // The voice bar keeps its own hot zone only on the edge it holds alone; a
  // free bar docks to a corner of the bottom strip.
  const orbPos = effectiveOrbPosition(cfg);
  const attached = orbIsAttached(cfg);
  const nearOrb = lastY >= 0 && lastY >= vh - ORB_ZONE
    && (orbPos === 'bottom-left'
      ? lastX >= 0 && lastX <= ORB_HALF
      : orbPos === 'bottom-right'
        ? lastX >= 0 && vw - lastX <= ORB_HALF
        : Math.abs(lastX - vw / 2) <= ORB_HALF);

  // A fullscreen window hides the bar of its workspace no matter what the
  // desktop-wide autohide preference says — at every edge it can dock to.
  const showBar = (fsActive || cfg.autohide_bar) ? (held || nearBar) : true;
  setReveal(showBar, attached ? showBar : (!cfg.autohide_orb || held || nearOrb));
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
  clearTimeout(edgeDwellTimer);
  edgeDwellTimer = null;
  edgeDwell = false;
  body.classList.remove('fs-top', 'fs-orb');
  // A bar/voice bar the user pinned open must not be dismissed with the rest.
  evalReveal();
}

function onPointerMove(e) {
  // Touch/pen have no hover: fullscreen.css keeps the chrome visible there.
  if (e.pointerType === 'touch') return;
  lastX = e.clientX;
  lastY = e.clientY;
  armEdgeDwell(barRevealProximity(), barKeepProximity());
  evalReveal();
}

/* ── Browser fullscreen state ───────────────────────────────── */

function onFullscreenChange() {
  const on = isAppFullscreen();
  // Browsers fire the prefixed and the standard event for one transition (and may
  // repeat it): only a real edge is a change, or the trailing event would act on
  // a window fullscreened in the meantime.
  if (on === browserFullscreen) return;
  browserFullscreen = on;
  body.classList.toggle('fullscreen-active', on);

  if (on) {
    ownExit = false; // the document is fullscreen again: a pending exit is moot
    announce();
    holdReveal(ENTER_HOLD_MS);
    nudgeResize();
    return;
  }

  // The browser left fullscreen: Escape, the browser's own chrome, or our exit.
  const mine = ownExit;
  ownExit = false;
  if (mine) {
    // Already handled at the moment we asked, and a new window may own the
    // screen by now — that one keeps its chrome.
    if (!hasFullscreenWindow()) dismissChrome();
  } else if (fullscreenWindow()) {
    // Escape: the window on screen gives up its screen. The fullscreen windows of
    // other workspaces keep theirs — they never depended on the browser.
    exitWindowFullscreen();
  } else {
    dismissChrome();
  }
  announce();
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
  browserFullscreen = isAppFullscreen();
  // `fullscreenchange` is standard; the webkit alias covers older Safari.
  document.addEventListener('fullscreenchange', onFullscreenChange);
  document.addEventListener('webkitfullscreenchange', onFullscreenChange);
  window.addEventListener('pointermove', onPointerMove, { passive: true });
  window.addEventListener('resize', evalReveal);
  // Alt+Enter lives in desktop.js; it only announces the gesture.
  window.addEventListener('app:fullscreen-toggle', () => toggleWindowFullscreenActive());
  // The AI's `desktop_fullscreen` asks for exactly what the window button does,
  // so the AI and the user can never disagree about what fullscreen means. The
  // request may lack the gesture the browser wants — the state does not depend
  // on the browser's answer.
  window.addEventListener('app:fullscreen-window', (e) => {
    const { name, on } = e.detail || {};
    if (!name) return;
    if (on === false) {
      if (fullscreenWindow() === name) exitWindowFullscreen();
    } else if (fullscreenWindow() !== name) {
      enterWindowFullscreen(name);
    }
  });
  // Settings changed (Settings page → Desktop): re-reveal under the new rules.
  window.addEventListener('desktop:immersive', (e) => {
    immersive = e.detail || getImmersive();
    evalReveal();
  });
  // Alt-tabbing away is not a hover: put the chrome back out of the way.
  window.addEventListener('blur', dismissChrome);
  // A workspace switch moves the takeover — and the bubble — to the window that
  // owns the screen there, or gives the desktop its chrome back.
  window.addEventListener('desktop:changed', () => {
    syncFullscreenChrome();
    announce();
  });
  // A fullscreen app that gets deactivated (its window button, the tray, the
  // Plugins page) is gone: leaving fullscreen hands its workspace back rather
  // than leaving the desktop sealed around an empty screen. A dedicated
  // workspace whose window is deactivated off screen is dropped when the windows
  // are reconciled (desktop.js).
  window.addEventListener('plugins:changed', () => {
    const name = fullscreenWindow();
    if (name && !isPluginActive(name)) exitWindowFullscreen();
  });
  // The state restored from storage is live by now (tiles.js — and with it
  // initDesktop — runs first): take over the chrome for a workspace that came
  // back fullscreen, and hand the bubble its window.
  syncFullscreenChrome();
  announce();
  // Show the chrome briefly on load, so an autohidden bar/voice bar is not a
  // mystery the first time the desktop appears.
  holdReveal(2600);
}
