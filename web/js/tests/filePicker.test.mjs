/**
 * The shell file dialog: `accept` filtering and realm-correct `File` objects.
 *
 * The dialog itself is UI (exercised on the kiosk); what is pinned here is the
 * part every caller depends on: which files an `<input accept>` admits, and that
 * the bytes read through the Files plugin become `File` objects of the *caller's*
 * realm (a web app's `input.files` rejects objects from another window).
 *
 * Run:  node web/js/tests/filePicker.test.mjs
 */

import './helpers/i18n.mjs';
import './helpers/window-events.mjs';
import assert from 'node:assert/strict';

import { acceptMatcher, filesFromEntries } from '../filePicker.js';

const dir = { name: 'Pictures', path: 'Pictures', kind: 'dir' };
const png = { name: 'logo.png', path: 'Pictures/logo.png', kind: 'file', ext: 'png', size: 120, is_image: true };
const docx = { name: 'notes.docx', path: 'Documents/notes.docx', kind: 'file', ext: 'docx', size: 300 };
const zip = { name: 'bundle.zip', path: 'Downloads/bundle.zip', kind: 'file', ext: 'zip', size: 900, is_archive: true };

/* A dialog always shows folders, whatever the filter. */
for (const accept of ['', '.docx', 'image/*']) {
  assert.equal(acceptMatcher(accept)(dir), true, `folders stay visible for '${accept}'`);
}

/* No filter: everything. */
assert.equal(acceptMatcher('')(png), true);
assert.equal(acceptMatcher('')(docx), true);

/* Extensions are exact, case-insensitive matches. */
const documents = acceptMatcher('.docx,.pdf');
assert.equal(documents(docx), true);
assert.equal(documents(png), false, 'an extension filter hides other files');
assert.equal(acceptMatcher('.PDF')({ ...docx, ext: 'pdf' }), true, 'extensions compare lowercased');

/* Wildcard types follow the Files plugin's own flags. */
const images = acceptMatcher('image/*');
assert.equal(images(png), true);
assert.equal(images(docx), false);
assert.equal(acceptMatcher('application/zip')(zip), true, 'common exact types are understood');

/* An unknown MIME rule must not hide a file the app may accept. */
assert.equal(acceptMatcher('application/x-weird')(docx), true);
assert.equal(acceptMatcher('application/x-weird')(png), true);

/* ── Bytes become the caller's Files ─────────────────────────── */

const realm = {
  File: class File {
    constructor(parts, name, options = {}) {
      this.parts = parts;
      this.name = name;
      this.type = options.type || '';
    }
  },
};

globalThis.fetch = async (url) => {
  assert.match(String(url), /^\/api\/files\/raw\?path=/);
  if (String(url).includes('logo.png')) {
    return new Response(new Uint8Array([1, 2, 3]), {
      status: 200,
      headers: { 'Content-Type': 'image/png' },
    });
  }
  return new Response('nope', { status: 404 });
};

const files = await filesFromEntries(realm, [png]);
assert.equal(files.length, 1);
assert.equal(files[0].name, 'logo.png');
assert.ok(files[0] instanceof realm.File, 'the File belongs to the caller realm');
assert.equal(files[0].type, 'image/png');
assert.equal(files[0].parts.length, 1);

await assert.rejects(filesFromEntries(realm, [docx]), /nope|not found|404/i);

console.log('\nfilePicker: all checks passed');
