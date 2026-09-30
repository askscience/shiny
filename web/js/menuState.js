/**
 * menuState — one signal for "a popup menu just opened or closed".
 *
 * The Browser window renders its page in a native child view that stacks above
 * every HTML layer (see `plugins/browser/web/plugin.js`). HTML popups — the
 * desktop context menus and the HUD menus — therefore cannot be seen over it,
 * and the window has to re-check what covers its page whenever a menu appears
 * or disappears. Rather than have each menu engine know about the Browser, the
 * engines just announce the change here and any surface that draws outside the
 * HTML stacking order listens for `menu:change`.
 *
 * The listener recomputes the truth from the DOM (which menus are open), so a
 * missed or duplicated announcement is harmless.
 */

/** Announce that a popup menu opened or closed. */
export function notifyMenuChange() {
  if (typeof window === 'undefined' || typeof window.dispatchEvent !== 'function') return;
  window.dispatchEvent(new Event('menu:change'));
}
