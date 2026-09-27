/**
 * plugin.js — the Image plugin's window surface.
 *
 * The workspace itself lives in `editor.js` (DOM + state), `api.js` (routes) and
 * `selection.js` (masks). This module is only the surface contract the plugin
 * host loads: mount/unmount/getElement/wireEvents/contextMenu.
 */

import * as editor from './editor.js';

export const IMAGE_PLUGIN = 'image';

export function mountImageTile() {
  return editor.mountEditor();
}

export function unmountImageTile() {
  editor.unmountEditor();
}

export function getImageTileElement() {
  return editor.getEditorElement();
}

export function wireImageEvents() {
  editor.wireEditorEvents();
}

/** Entries core splices into this window's right-click menu (PLUGINS.md §19).
 *  Core supplies the surrounding separators and window management. */
export function imageContextMenu() {
  return editor.editorContextMenu();
}

export default {
  name: IMAGE_PLUGIN,
  icon: 'ui/image',
  mount: mountImageTile,
  unmount: unmountImageTile,
  getElement: getImageTileElement,
  wireEvents: wireImageEvents,
  contextMenu: imageContextMenu,
};
