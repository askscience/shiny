/**
 * Files an embedded app writes on its own.
 *
 * A web app inside the shell (a `kind = "web"` plugin in its iframe, the Craft
 * apps) saves through the ordinary browser APIs: the save picker is routed
 * through the shell's own dialog, and the fallback — an anchor download —
 * through the shell's download handler. Either way the bytes land on disk
 * without the app telling anyone, so the Qt shell reports the path back with a
 * `shiny:file-saved` event (see `crates/peakd/shim/peakd.cpp`). This turns that
 * into the two things the desktop would have done for any other save: tell the
 * user where the file went, and repaint the folder, so an open Files window
 * does not keep showing the listing it had before.
 *
 * The message reuses `files.savedTo`, the same line the Files-backed saves show.
 */
import { t } from './i18n.js';

/** The message for one reported save: the path is the interesting part. */
export function fileSavedNotice(detail) {
  const path = detail?.path || '';
  return t('files.savedTo', { path }, `Saved to ${path}`);
}

/**
 * Follow the shell's save reports.
 *
 * `notify` is the function the message is shown with — injected so this module
 * stays free of the UI kit, which the unit tests have no DOM for.
 */
export function initAppFileEvents({ notify } = {}) {
  window.addEventListener('shiny:file-saved', (event) => {
    const detail = event.detail || {};
    if (!detail.path) return;
    // The Files plugin listens for this: it may be showing the very folder the
    // app just wrote into.
    window.dispatchEvent(new CustomEvent('files:external', { detail }));
    if (typeof notify === 'function') notify(fileSavedNotice(detail));
  });
}
