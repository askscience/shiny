/**
 * nativeOccluders.js — core surfaces that must be cut out of a native page.
 *
 * The Browser plugin renders each page in a native WebEngine item in the
 * shell's scene (crates/peakd), above the whole app webview. HTML can only stay
 * visible over a page when the plugin lists the surface's rect in
 * `peakd:view:setMask`; otherwise the page paints over it.
 *
 * This module owns the core half of that list. The Browser adds its own
 * popovers and any higher floating window. Keep the selector in step with new
 * fixed chrome; the watcher only signals that the geometry may have changed.
 */

/** Core HTML surfaces that can be painted over a native page. */
export const NATIVE_OCCLUDER_SELECTOR = [
  // Shared popups and transient surfaces.
  '.ctx-menu',
  '.ui-hud-menu-popup:not(.hidden)',
  '.ui-select-menu',
  '.ui-modal:not(.hidden)',
  '.ui-sheet:not(.hidden)',
  '.ui-floating:not(.hidden)',
  '.ui-toast-container',
  '.ui-notification-container',
  // Chat: the overlay itself is a transparent full-screen layer; only the
  // conversation panel is opaque and interactive.
  '#compose-mode:not(.hidden) .compose-conversation',
  // Fixed or modal desktop chrome. Persistent, non-modal overlays (the voice
  // bar, the artifact dock, insight cards, the voice transcript) stay out: a
  // mask hole disables page input, so only surfaces the user is actively
  // working in may take the page's pointer.
  '.hud-header-main',
  '#workspace-bar:not(.hidden)',
  '#fullscreen-bubble:not(.hidden)',
  '#keyboard-bar.visible',
  // Navigation and travel.
  '#nav-hud:not(.hidden) #nav-strip',
  '#nav-hud:not(.hidden) #nav-speed-chip',
  '#nav-puck:not(.hidden)',
  '#travel-panel:not(.hidden)',
  '#travel-panel-backdrop.visible',
  // Full-window artifact overlay.
  '#tile-overlay:not(.hidden)',
].join(', ');

/** Roots whose class/style changes move or reveal a hole. */
const WATCH_ROOTS = [
  '#compose-mode',
  '#keyboard-bar',
  '#nav-hud',
  '#nav-puck',
  '#travel-panel',
  '#travel-panel-backdrop',
  '#tile-overlay',
  '#fullscreen-bubble',
  '#workspace-bar',
  '.ui-modal',
  '.ui-sheet',
  '.ui-floating',
  '.ui-hud-menu-popup',
  '.ctx-menu',
  '.ui-select-menu',
  '.ui-toast-container',
  '.ui-notification-container',
].join(', ');

/**
 * The holes a page must show through, in the same client-pixel space as the
 * viewport rect the plugin sends with `setBounds`.
 */
export function nativeOccluderHoles(viewport) {
  if (!viewport) return [];
  const dpr = viewport.dpr || 1;
  const right = viewport.x + viewport.w;
  const bottom = viewport.y + viewport.h;
  const holes = [];
  for (const el of document.querySelectorAll(NATIVE_OCCLUDER_SELECTOR)) {
    const rect = el.getBoundingClientRect();
    if (!rect || rect.width < 1 || rect.height < 1) continue;
    if (rect.right <= viewport.x || rect.left >= right) continue;
    if (rect.bottom <= viewport.y || rect.top >= bottom) continue;
    holes.push({ x: rect.left, y: rect.top, w: rect.width, h: rect.height, dpr });
  }
  return holes;
}

/**
 * Call `onChange` when a core surface is added, removed, or toggles a visibility
 * class. The Browser uses it to refresh the mask at once instead of waiting for
 * the 1 s bounds fallback.
 */
export function watchNativeOccluders(onChange) {
  if (typeof MutationObserver !== 'function' || !document.body) return () => {};

  const observer = new MutationObserver(() => {
    attach();
    onChange();
  });
  const seen = new WeakSet();

  function attach() {
    for (const el of document.querySelectorAll(WATCH_ROOTS)) {
      if (seen.has(el)) continue;
      seen.add(el);
      observer.observe(el, { attributes: true, attributeFilter: ['class', 'style'] });
    }
  }

  attach();
  // Menus, modals, floating windows and feedback containers are appended
  // directly to <body>, so watching its children catches surfaces created
  // after the Browser window mounted.
  observer.observe(document.body, { childList: true });
  return () => observer.disconnect();
}
