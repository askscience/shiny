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

- A sidebar of classic places (Home, Trash, the XDG folders). In a window
  up to 560px wide, and on a phone, the places become a strip that scrolls
  sideways above the content.
- The window bar (`data-window-bar`, merged with the title row by the tile
  manager), in the editorial style of the theme lab (`web/theme-lab/v4`):
  round-cornered grey square back/forward buttons, a plain breadcrumb
  ("Home / Documents": ancestors muted and clickable, the current folder in
  500 weight, a small chevron for the path actions — new folder, upload, copy
  path, hidden files), a grey search field with its glyph on the right
  (typing searches the whole home after two characters; Escape clears;
  Ctrl+F / Ctrl+L focus it), the grid/list switcher with its options chevron
  as one grey group, and the flat app menu (new folder, upload, shared links,
  select, hidden files, view, empty trash). While the bar is compact the
  view switcher steps into the menu and the search shrinks to its glyph
  until it is tapped.
- On a phone a thumb row along the bottom (`actionBar`) carries New folder
  and Upload.
- **Touch:** on a coarse pointer one tap opens an entry (a mouse keeps the
  double-click), a long press opens the item menu (core `contextMenu.js`),
  and **Select** — in the app menu and the item menu — enters a selection
  mode where taps toggle entries, each wearing a check, until Done (the bar's
  check, or Escape). The quick look's arrows stay visible on touch.
- **Grid view:** each entry is a rounded (12px) thumbnail box ~92px tall on the
  theme's tile grey with the monochrome glyph centred — folders use the icon
  catalog's `ui/folder*` names, which the Dusk/Ink themes override with their
  monochrome folders — the name below (12.5px, 500, one line with an ellipsis)
  and a mono uppercase meta line (`24 ITEMS`, `PDF · 2.4 MB`). Photos and video
  posters fill the box (`object-fit: cover`, a slight zoom under the pointer);
  PDFs show the top of page 1; text and Office files show a little sheet rising
  out of the box. A selected entry's box turns the accent with the
  accent-contrast glyph (a photo keeps its picture and wears an accent ring).
- **List view:** Name / Size / Modified columns (click to sort) with mono
  uppercase headers and meta; each row's glyph sits in a small grey box that
  turns the accent when selected.
- A **status line** along the bottom on a hairline: `8 ITEMS · 1 SELECTED` on
  the left, the selection's total size on the right (shown only when the
  selection has files — nothing is shown that the API does not report, so there
  is no free-space figure).
- When a folder's contents load they arrive through the shared
  `listReveal(nodes)` (`web/ui/motionStyle.js`), in the user's motion language;
  re-renders for a selection, sort or view switch do not animate.
- Styling lives in core CSS — the *Files plugin window* section of
  [`web/css/tiles.css`](../../../web/css/tiles.css) — from theme tokens only
  (the `--ed-*` skin tokens with generic fallbacks). The shell's file picker
  (`web/js/filePicker.js`) reuses the same grid classes.
- **Double-click** opens a file: known types open in their owning app
  (`onOpenFromFiles`/`openWithPlugin` in
  [`web/js/files.js`](../../../web/js/files.js)), otherwise the OS handler.
- **Space** opens the quick-look: images, PDF (vendored PDF.js), Office
  documents (via `/render`), text mini, and a streaming video player using
  byte-ranged `/raw`.
- The quick-look is a **compact bare lightbox**, not a window: a centered
  surface (up to 920×620) with no frame or padding, the arrows that step through
  the folder appear only under the pointer (`← →` do the same from the keyboard),
  and the details ride a gradient mixed from the theme's canvas. Clicking the
  veil outside the surface (or Escape) closes it. `openPreview`/`showPreview`
  reuse the open overlay, so stepping repaints the file instead of rebuilding
  it.
- Video and music keep their QuickTime-style bar on the accent colour
  (`--accent`/`--accent-contrast`); it follows the pointer and steps aside while
  playing (`.files-player.is-idle`).
- A **`.Trash`** view with restore / empty-trash.

## Events & integration

- `wireFilesEvents` subscribes to plugin/agent events so the window can react
  (e.g. refresh after an upload, focus a path opened from another app).
- `files:external` reloads the open folder when the shell reports a file written
  outside this plugin — an embedded web app (the Craft suite) saving through its
  own picker. That path is absolute and may live anywhere, so the listing is
  simply reloaded rather than matched against the current directory.
- `filesContextMenu(ctx)` contributes right-click entries (open, share link,
  rename, copy, paste, delete, new folder, upload, empty trash) between core's
  window menu items.
- **Share link…** (one selected file or folder) and **Shared links** (app
  menu) live in [`web/share.js`](../web/share.js). The dialog asks for expiry,
  download limit and an optional password, polls the upload job and shows the
  link with a Copy button. The list dialog shows download counts and offers
  Copy and Revoke. Turning sharing on or off and choosing Tailscale Funnel or
  a custom URL happen in Settings → Remote → File sharing; both dialogs link
  there when sharing is off. Routes are in [routes.md](routes.md#link-sharing-gokapi).
- Other plugins import [`web/js/files.js`](../../../web/js/files.js):
  `saveOrDownload(blob, { name, dir, app })` uploads via `/api/files/upload`
  and falls back to a browser download when Files is not installed;
  `pickFiles()` returns a file picker; `fileFromHome(path)` fetches bytes;
  `onOpenFromFiles`/`openWithPlugin` handle double-click routing.
- Files (not folders) are **draggable** into other windows: `dragstart` sets
  `application/x-shiny-file` (`SHINY_FILE_MIME` in `web/js/files.js`) to JSON
  `{ path, name }` plus the name as `text/plain`. The target fetches the bytes
  with `fileFromHome(path, name)`; Word inserts dropped pictures and imports
  dropped documents.

## Thumbnails & previews

- Images: server-cached PNG via `photon-rs`.
- PDFs: vendored PDF.js.
- Video: server-side `ffmpeg` frame extraction — the **same frame appears on
  macOS and Linux**, rather than relying on a host `<video>`/QuickTime decode.
- Text: a short text mini.

## Development

The app serves the **installed** copy at `data/plugins/files/web/` (or the
`PLUGINS_DIR` override, see AGENT.md), not `plugins/files/web/`. Copy or
reinstall after editing. `node --check
web/plugin.js` catches syntax errors; `node --experimental-vm-modules
web/plugin.smoke.mjs` mounts the window and drives the quick look against a DOM
shim.
