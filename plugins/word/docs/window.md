# Word — window (`web/plugin.js`)

The Word window is a single ES module served from the plugin's `web_dir`. It
renders a flat editorial surface: a hairline toolbar (document menu + title +
formatting), a `contentEditable` editor and a status line. It talks to the REST
routes in [routes.md](routes.md) and reacts to agent activity.

## Exports

| Export | Kind | Responsibility |
|---|---|---|
| `WORD_PLUGIN` | const | `'word'` |
| `mountWordTile()` | function | Build and return the tile (idempotent) |
| `unmountWordTile()` | function | Flush, tear down and null every ref |
| `getWordTileElement()` | function | The tile element or `null` |
| `wireWordEvents()` | function | Register global listeners (once) |
| `wordContextMenu()` | function | Entries for the window's right-click menu |
| `default` | object | `{ name:'word', icon:'ui/doc', mount, unmount, getElement, wireEvents, contextMenu }` |

## Mount lifecycle

`mountWordTile()` returns the existing tile early if `tileEl` is set. Otherwise
it builds the DOM, registers listeners, calls `void openNewest()` and
`updateGlow()`, and returns the element. Core mounts it when the plugin/desktop
activates the window.

`unmountWordTile()`:

1. If a document is open and `dirty`, fires `void persist()` (best-effort flush).
2. `closeDocMenu()` removes the body-level popup listeners.
3. Removes the tile and sets `tileEl`, `editorEl`, `titleInput`, `docMenuBtn`,
   `statusEl`, `saveDot`, `fontSelect`, `toolbarBtns` all to `null`, so a later
   mount builds a fresh window instead of returning a detached one.

`wireWordEvents()` is guarded by a module-level `wired` flag so the
`agent:actions` / `files:open` / `beforeunload` listeners are bound once.

## DOM structure

```
section.tile.word-tile[data-plugin="word"]
├── div.word-bar[data-window-bar]
│   ├── button.word-doc-btn            (icon ui/doc + chevron; toggles menu)
│   ├── input.word-title               (maxlength 120, placeholder "Untitled")
│   ├── button.word-tool               (bold / italic / underline / heading / list)
│   ├── div.word-font                  (the `select` component for host fonts)
│   ├── button.word-tool--secondary    (new / import / export / save / delete)
│   └── span.word-save-dot[aria-hidden]
├── div.word-editor[contenteditable="true"][role="textbox"][aria-multiline]
└── div.word-status
    └── span                           (status text)
```

The document menu popup is appended to `document.body` (the tile clips
overflow): `div.word-doc-menu` containing `button.word-doc-menu-item`
(row → `.word-doc-menu-title`, `.word-doc-menu-time`, `.word-doc-menu-check`)
and `div.word-doc-menu-foot` (new, import, save-to-Documents, delete). Empty
state uses `.word-doc-menu-empty`.

## CSS class prefix

All classes are `word-*`: `.word-tile`, `.word-bar`, `.word-doc-btn`,
`.word-doc-btn-icon`, `.word-doc-btn-chevron`, `.word-title`, `.word-tool`,
`.word-tool--secondary`, `.word-tool--danger`, `.word-font`, `.word-save-dot`,
`.word-editor`, `.word-status`, `.word-doc-menu`, `.word-doc-menu-item`,
`.word-doc-menu-title`, `.word-doc-menu-time`, `.word-doc-menu-check`,
`.word-doc-menu-foot`, `.word-doc-menu-foot-icon`, `.word-doc-menu-empty`.
Active states use `is-active` / `hidden`.

## State & autosave

Module state: `docs[]`, `currentDoc`, `dirty`, `saveTimer`, `saveSeq`.

- `markDirty()` sets `dirty`, shows "Unsaved changes", and schedules
  `persist()` after **1200 ms** (debounced).
- `persist()` clears `dirty`, PUTs `{ title, html: editorEl.innerHTML }`, and
  updates the status line (with word count) or reverts to dirty on failure.
- `setStatus('saving'|'dirty'|'saved')` drives the status text and
  `.word-save-dot.is-active`.
- `saveSeq` guards against out-of-order saves when switching documents.

## Editing

- Toolbar buttons run `document.execCommand`: `bold`, `italic`, `underline`,
  `formatBlock:<h2>` (heading toggle) and `insertUnorderedList`.
- `syncToolbar()` reads `queryCommandState`/`queryCommandValue` and marks the
  active buttons; it also refreshes the font picker.
- Fonts: `listFonts()` populates the picker from the host. A selection gets a
  wrapping `<span style="font-family…">` with `data-font`; with no selection the
  whole document default changes. `TF<n>` styles carry the family through ODT.
- Status word count = `editorEl.innerText` split on whitespace.

## Menu & glow

`openDocMenu()` positions `.word-doc-menu` under the button, renders the
document list (active row via `.is-active`) and the footer actions. Outside
pointerdown and `Escape` close it. `updateGlow()` mirrors the first embedded
`<img>` through `setTileGlowFromUrl`, else a title-seeded warm gradient from
`titleGlow()`.

## API calls

| Call | Method | Purpose |
|---|---|---|
| `/api/documents` | GET | `listDocs()` |
| `/api/documents` | POST | `createDoc()` → `{title:'Untitled', html:'<p></p>'}` |
| `/api/documents/:id` | GET | `fetchDoc(id)` |
| `/api/documents/:id` | PUT | `saveDoc(title, html)` |
| `/api/documents/:id` | DELETE | `deleteDoc(id)` |
| `/api/documents/import` | POST | `importOdt(file)` — `FormData` field `file` |
| `/api/documents/:id/export` | GET (blob) | `exportCurrent()` |

All go through `apiFetch` from `web/js/api.js`, which unwraps `{data}`.

## Save / export flow

- **Autosave**: every edit → `markDirty()` → debounced `persist()` → `PUT` HTML.
- **Save to Documents** (`exportCurrent()`): fetches the export route as a
  `Blob`, then calls `saveOrDownload(blob, { name: `${title}.odt`, dir:
  'Documents', app: 'Word' })` from `web/js/files.js`. `saveOrDownload` writes
  to the user's home via `POST /api/files/upload` and toasts
  `Word saved to <path>`, falling back to a browser download when the Files
  plugin is not installed.
- **Import**: the file picker (`pickFiles({accept:'.odt,…'})`) reads a `File`,
  posts it to `/api/documents/import`, refreshes and opens the newest document.

## Files integration & import

`wireWordEvents()` calls `onOpenFromFiles('word', …)`, so double-clicking a
`.odt` in the Files app routes through `openWithPlugin`
(`web/js/files.js` → `FILE_OPENERS.odt === 'word'`). `importFromFiles(path,
name)` mounts the tile if needed, fetches bytes with
`fileFromHome(path, name, 'application/vnd.oasis.opendocument.text')` and runs
the same `importOdt()`.

## Agent wiring

`onAgentActions(e)` filters `e.detail` for actions matching `/^doc_/`:

1. Dispatches `plugin:focus {name:'word'}` to surface the window.
2. Refreshes the list; if a `doc_delete` succeeded it opens the newest, else it
   finds the touched `data.doc_id`, flushes unsaved edits and opens it.

`beforeunload` warns when `currentDoc && dirty`. `wordContextMenu()` returns
items New document / Save now / Save to Documents / Bold / Italic / Heading /
List / Delete document, with `disabled` flags derived from whether a document
is open.

## Related

- [README.md](README.md) · [routes.md](routes.md) · [tools.md](tools.md) ·
  [files.js](../../../web/js/files.js)
