/**
 * gestures.js — trackpad gestures from the peakd shell.
 *
 * WebKitGTK delivers no multi-touch pad gestures to the page, so the native
 * Linux shell recognises three-finger swipes from the trackpad's evdev stream
 * (crates/peakd/src/gestures.rs) and dispatches `trackpad:gesture` with a
 * direction. This module is the whole web-side vocabulary:
 *
 *   swipe left  → next workspace
 *   swipe right → previous workspace
 *   swipe up    → window overview (every open window, live)
 *   swipe down  → plugin launcher
 *
 * The direction is decided natively because only there is the finger count
 * known; everything the gesture *does* stays in the desktop modules.
 */
import { switchWorkspace } from './desktop.js';
import { openOverview, closeOverview, isOverviewOpen } from './overview.js';
import { openLauncher, closeLauncher, isLauncherOpen } from './launcher.js';

/**
 * Apply one gesture direction. Exported so a future input source (a key, a
 * Touch Bar button) can reuse the same mapping instead of duplicating it.
 *
 * While an overlay is open a vertical swipe only *pulls it back* — it never
 * jumps straight to the other overlay. So: swipe down opens the launcher,
 * swipe up closes it, and only the next swipe up opens the window overview
 * (and the reverse for the overview).
 */
export function applyGesture(direction) {
  const overlayOpen = isOverviewOpen() || isLauncherOpen();

  if (direction === 'left' || direction === 'right') {
    // A workspace switch under an open overlay would leave it showing the old
    // workspace: dismiss it first, then switch.
    if (isOverviewOpen()) closeOverview();
    if (isLauncherOpen()) closeLauncher();
    switchWorkspace(direction === 'left' ? 'next' : 'prev');
    return;
  }

  if (direction === 'up' || direction === 'down') {
    // Any vertical swipe while an overlay is up is "pull back", whichever way
    // it points; opening the other overlay needs a fresh swipe.
    if (overlayOpen) {
      if (isOverviewOpen()) closeOverview();
      if (isLauncherOpen()) closeLauncher();
      return;
    }
    if (direction === 'up') openOverview();
    else openLauncher();
  }
}

export function initGestures() {
  window.addEventListener('trackpad:gesture', (e) => {
    const direction = e.detail?.direction;
    if (typeof direction === 'string') applyGesture(direction);
  });

  // A hand-testing hook: `__shinyGesture('up')` in the console raises the same
  // actions as a swipe, so the UI can be checked without the native reader (and
  // without the udev rule). Harmless in production.
  window.__shinyGesture = applyGesture;
}
