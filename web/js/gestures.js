/**
 * gestures.js — trackpad gestures from the peakd shell.
 *
 * WebKitGTK delivers no multi-touch pad gestures to the page, so the native
 * Linux shell recognises three-finger swipes from the trackpad's evdev stream
 * (crates/peakd/src/gestures.rs) and dispatches `trackpad:gesture`. This
 * module is the whole web-side vocabulary:
 *
 *   swipe left  → next workspace
 *   swipe right → previous workspace
 *   swipe up    → window overview (every open window, live)
 *   swipe down  → plugin launcher
 *
 * Swipes are continuous. The shell sends `{ phase, axis, direction, progress,
 * velocity }`: `begin` once three fingers have locked an axis, `update` as
 * they move (progress signed along the begin direction, 1 ≈ a full swipe),
 * `end` on release, `cancel` if the stream broke. The surface follows the
 * fingers and on release completes or springs back (gestureTransition.js
 * decides) — never left half-way. A detail with no `phase` is the older
 * discrete form and runs the whole transition at once, like `applyGesture`.
 *
 * The direction is decided natively because only there is the finger count
 * known; everything the gesture *does* stays in the desktop modules.
 */
import { switchWorkspace } from './desktop.js';
import { overviewTransition, openOverview, closeOverview, isOverviewActive } from './overview.js';
import { launcherTransition, openLauncher, closeLauncher, isLauncherActive } from './launcher.js';
import { beginWorkspaceSlide } from './tiles.js';
import { shouldCommit } from './gestureTransition.js';

const HORIZONTAL = new Set(['left', 'right']);
const VERTICAL = new Set(['up', 'down']);

/** The gesture under the fingers: `{ transition }` or `{ switchDir }`. */
let live = null;

function overlayUp() {
  return isOverviewActive() || isLauncherActive();
}

/** Dismiss whichever overlay is up, then run `then` (once the desktop is back). */
function dismissOverlays(then = null) {
  if (isLauncherActive()) closeLauncher();
  closeOverview(then);
}

/**
 * Apply one gesture direction as a whole, animated transition. Exported for
 * discrete input (a key, the Touch Bar, the console hook) so it reuses the
 * same mapping as the trackpad instead of duplicating it.
 *
 * While an overlay is open a vertical swipe only *pulls it back* — it never
 * jumps straight to the other overlay. So: swipe down opens the launcher,
 * swipe up closes it, and only the next swipe up opens the window overview
 * (and the reverse for the overview).
 */
export function applyGesture(direction) {
  if (HORIZONTAL.has(direction)) {
    // A workspace switch under an open overlay would leave it showing the old
    // workspace: dismiss it first, then switch.
    const go = () => switchWorkspace(direction === 'left' ? 'next' : 'prev');
    if (overlayUp()) dismissOverlays(go);
    else go();
    return;
  }
  if (VERTICAL.has(direction)) {
    // Any vertical swipe while an overlay is up is "pull back", whichever way
    // it points; opening the other overlay needs a fresh swipe.
    if (overlayUp()) {
      dismissOverlays();
      return;
    }
    if (direction === 'up') openOverview();
    else openLauncher();
  }
}

/** What a swipe that began towards `direction` drives. */
function begin(direction) {
  if (HORIZONTAL.has(direction)) {
    if (overlayUp()) {
      // Same rule as the discrete swipe: the overlay goes first; the switch
      // follows on release if the swipe was a real one.
      dismissOverlays();
      return { switchDir: direction };
    }
    const transition = beginWorkspaceSlide(direction === 'left' ? 1 : -1);
    return transition ? { transition } : null;
  }
  if (VERTICAL.has(direction)) {
    // Progress is positive in the direction the swipe began, so "pull back"
    // works whichever way the fingers go.
    let transition = null;
    if (isOverviewActive()) transition = overviewTransition(false);
    else if (isLauncherActive()) transition = launcherTransition(false);
    else if (direction === 'up') transition = overviewTransition(true);
    else transition = launcherTransition(true);
    return transition ? { transition } : null;
  }
  return null;
}

function onGesture(detail) {
  const { phase, direction } = detail;
  const progress = Number(detail.progress) || 0;
  const velocity = Number(detail.velocity) || 0;

  if (!phase) {
    if (typeof direction === 'string') applyGesture(direction);
    return;
  }

  if (phase === 'begin') {
    // A begin without the previous end (a lost event): settle that one first.
    live?.transition?.end(0);
    live = begin(direction);
    live?.transition?.update(progress);
    return;
  }
  if (!live) return;

  if (phase === 'update') {
    live.transition?.update(progress);
    return;
  }
  if (phase === 'end') {
    if (live.transition) {
      // The release carries the last position; updates may have coalesced it away.
      live.transition.update(progress);
      live.transition.end(velocity);
    }
    else if (live.switchDir && shouldCommit(progress, velocity)) {
      applyGesture(live.switchDir);
    }
    live = null;
    return;
  }
  if (phase === 'cancel') {
    live.transition?.cancel();
    live = null;
  }
}

/** Ctrl+Alt+arrows: the four swipes from the keyboard. */
const KEY_DIRECTIONS = {
  ArrowUp: 'up', ArrowDown: 'down', ArrowLeft: 'left', ArrowRight: 'right',
};

export function initGestures() {
  window.addEventListener('trackpad:gesture', (e) => {
    if (e.detail && typeof e.detail === 'object') onGesture(e.detail);
  });

  // The same swipes from the keyboard. Ctrl+Alt (no Shift — that set is the
  // Touch Bar's, no Super — the OS's) is free everywhere else in the app.
  window.addEventListener('keydown', (e) => {
    if (!e.ctrlKey || !e.altKey || e.shiftKey || e.metaKey) return;
    const direction = KEY_DIRECTIONS[e.key];
    if (!direction) return;
    e.preventDefault();
    applyGesture(direction);
  });

  // A hand-testing hook: `__shinyGesture('up')` in the console raises the same
  // actions as a swipe, so the UI can be checked without the native reader (and
  // without the udev rule). Harmless in production.
  window.__shinyGesture = applyGesture;
}
