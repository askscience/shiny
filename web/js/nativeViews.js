/**
 * nativeViews.js — the native page views this document opened in the shell.
 *
 * A Browser tab is a native web view in the shell, outside the app's DOM. A
 * reload of the app (an update, a crash recovery) starts a new document that
 * knows nothing of the views the old one opened: they would stay on screen,
 * no longer moved with their window, hidden or closed. The ids are therefore
 * kept in `sessionStorage`, which outlives a reload of the same view, and the
 * new document closes whatever is left before anything opens again.
 *
 * (A current shell closes them itself when the app starts loading; this
 * covers a shell that does not, and costs a no-op `close` otherwise.)
 */
const KEY = 'liteaura.nativeViews';
const PREFIX = 'liteaura:view:';

function read() {
  try {
    const list = JSON.parse(sessionStorage.getItem(KEY) || '[]');
    return Array.isArray(list) ? list.filter((id) => typeof id === 'string' && id) : [];
  } catch (_) {
    return [];
  }
}

function write(list) {
  try {
    if (list.length) sessionStorage.setItem(KEY, JSON.stringify(list));
    else sessionStorage.removeItem(KEY);
  } catch (_) {
    /* storage blocked: a reload may leave a view behind, as before */
  }
}

/** A view was opened (`open`). */
export function rememberNativeView(id) {
  const list = read();
  if (!list.includes(id)) write([...list, id]);
}

/** A view was closed (`close`). */
export function forgetNativeView(id) {
  write(read().filter((v) => v !== id));
}

/** Close every view a previous document of this app left open. */
export function closeOrphanNativeViews() {
  const list = read();
  if (!list.length) return 0;
  const ipc = typeof window !== 'undefined' ? window.ipc : null;
  if (ipc && typeof ipc.postMessage === 'function') {
    for (const id of list) {
      try {
        ipc.postMessage(PREFIX + JSON.stringify({ op: 'close', id }));
      } catch (_) {
        /* the shell is gone; nothing to close */
      }
    }
  }
  write([]);
  return list.length;
}
