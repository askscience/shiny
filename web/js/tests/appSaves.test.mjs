/**
 * Embedded-app saves: the shell reports the file, the desktop shows it.
 *
 * The Qt shell watches the path a web app picked and reports it once the bytes
 * are on disk (`shiny:file-saved`). This pins the two reactions that make such a
 * save visible — the toast with the real path, and the `files:external` event an
 * open Files window refreshes on — plus the message itself, which has to stay
 * the catalogue's `files.savedTo` line (`{path}` placeholder intact).
 *
 * Run:  node web/js/tests/appSaves.test.mjs
 */
import './helpers/i18n.mjs';
import assert from 'node:assert/strict';
import { initAppFileEvents, fileSavedNotice } from '../appSaves.js';

/* ── A window, just enough of one ────────────────────────────── */

const listeners = new Map();
const dispatched = [];
globalThis.window = {
  addEventListener(type, handler) {
    if (!listeners.has(type)) listeners.set(type, []);
    listeners.get(type).push(handler);
  },
  dispatchEvent(event) {
    dispatched.push(event);
    return true;
  },
};
globalThis.CustomEvent = class CustomEvent {
  constructor(type, init = {}) {
    this.type = type;
    this.detail = init.detail;
  }
};

/* ── The message ─────────────────────────────────────────────── */

const detail = {
  path: '/home/eev/Documents/notes.odt',
  name: 'notes.odt',
  dir: '/home/eev/Documents',
  dirName: 'Documents',
  source: 'picker',
};
assert.equal(fileSavedNotice(detail), 'Saved to /home/eev/Documents/notes.odt',
  'the toast carries the path the file really landed in');

/* ── The reactions ───────────────────────────────────────────── */

const toasts = [];
initAppFileEvents({ notify: (message) => toasts.push(message) });
assert.equal(listeners.get('shiny:file-saved')?.length, 1, 'the shell report is followed');

const emit = (event) => listeners.get('shiny:file-saved').forEach((handler) => handler(event));
emit({ detail });
assert.deepEqual(toasts, ['Saved to /home/eev/Documents/notes.odt'], 'one toast per save');
assert.equal(dispatched.length, 1, 'and one refresh signal');
assert.equal(dispatched[0].type, 'files:external', 'the Files window listens for this one');
assert.equal(dispatched[0].detail.path, detail.path, 'the folder is handed over with it');

emit({ detail: {} });
emit({});
assert.equal(toasts.length, 1, 'a report without a path says nothing');
assert.equal(dispatched.length, 1);

// The download fallback reports its own copy the same way.
emit({ detail: { path: '/home/eev/Downloads/notes.odt', source: 'download' } });
assert.equal(toasts.length, 2, 'a downloaded copy is reported too');
assert.equal(dispatched.length, 2);

console.log('appSaves: all checks passed');
