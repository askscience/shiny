/**
 * backStack — one answer to "go back".
 *
 * A phone has one Back (Android's gesture, the browser's history, an edge
 * swipe); the desktop has Escape. Whatever is open on top — a pushed pane
 * (components/panes.js), a sheet or modal (components/overlay.js), a menu —
 * registers a handler while it is up and removes it when it closes. `popBack()`
 * asks the newest handler first; the first that returns true has consumed
 * the Back, and nothing else moves. With nothing registered the caller does
 * its own thing (phoneShell.js goes home).
 *
 * Handlers are plain functions; registering returns the remover, so a
 * surface can keep one line of state:
 *
 *   let release = null;
 *   open()  { release = pushBack(() => { close(); return true; }); }
 *   close() { release?.(); release = null; … }
 */

const handlers = [];

/** Register `handler` as the newest Back consumer. Returns its remover. */
export function pushBack(handler) {
  if (typeof handler !== 'function') return () => {};
  handlers.push(handler);
  return () => removeBack(handler);
}

/** Forget `handler`, wherever it sits in the stack. */
export function removeBack(handler) {
  const at = handlers.lastIndexOf(handler);
  if (at !== -1) handlers.splice(at, 1);
}

/** Go back one step: true when some surface consumed it. */
export function popBack() {
  for (let i = handlers.length - 1; i >= 0; i -= 1) {
    let consumed = false;
    try { consumed = handlers[i]() === true; } catch (_) { consumed = false; }
    if (consumed) return true;
  }
  return false;
}

/** How many surfaces are waiting for a Back. */
export function backDepth() { return handlers.length; }
