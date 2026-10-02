# Files — window surface

[`web/plugin.js`](../web/plugin.js), class prefix `files-*`. Registered like
every window plugin:

```js
export default {
  name: 'files',
  mount: mountFilesTile,      // returns the section.tile element
  unmount: unmountFilesTile,
  getElement: getFilesTileElement,
  wireEvents: wireFilesEvents,
  contextMenu: filesContextMenu,
};
```

## Layout

- A single top bar with navigation (back/forward/up), a breadcrumb, view
  toggles (grid/list), search, and file actions.
- A main pane listing entries with lazy thumbnails (server `/thumb`), a
  sidebar of classic places, and a selection model.
- **Double-click** opens a file: known types open in their owning app
  (`onOpenFromFiles`/`openWithPlugin` in
  [`web/js/files.js`](../../../web/js/files.js)), otherwise the OS handler.
- **Space** opens the quick-look: images, PDF (vendored PDF.js), Office
  documents (via `/render`), text mini, and a streaming video player using
  byte-ranged `/raw`.
- A **`.Trash`** view with restore / empty-trash.

## Events & integration

- `wireFilesEvents` subscribes to plugin/agent events so the window can react
  (e.g. refresh after an upload, focus a path opened from another app).
- `filesContextMenu(ctx)` contributes right-click entries (open, rename, copy,
  paste, delete, new folder, upload, empty trash) between core's window menu
  items.
- Other plugins import [`web/js/files.js`](../../../web/js/files.js):
  `saveOrDownload(blob, { name, dir, app })` uploads via `/api/files/upload`
  and falls back to a browser download when Files is not installed;
  `pickFiles()` returns a file picker; `fileFromHome(path)` fetches bytes;
  `onOpenFromFiles`/`openWithPlugin` handle double-click routing.

## Thumbnails & previews

- Images: server-cached PNG via `photon-rs`.
- PDFs: vendored PDF.js.
- Video: server-side `ffmpeg` frame extraction — the **same frame appears on
  macOS and Linux**, rather than relying on a host `<video>`/QuickTime decode.
- Text: a short text mini.

## Development

The app serves the **installed** copy at `data/plugins/files/web/`, not
`plugins/files/web/`. Copy or reinstall after editing. `node --check
web/plugin.js` catches syntax errors.
