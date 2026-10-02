# Image plugin — window surface

The Image window lives in `plugins/image/web/`. It is a **docked raster-editor
workspace** built entirely from core UI components; it ships **no CSS**. All
styling is the `image-*` block in the core stylesheet `web/css/tiles.css`, per
the plugin UI contract (PLUGINS.md §19). The window is a singleton: `S` is
module-level state and the tile is built/destroyed by mount/unmount.

Files:

| File | Role |
|---|---|
| `web/plugin.js` | The surface contract the plugin host loads. |
| `web/editor.js` | The workspace: DOM, state, event wiring, painting. |
| `web/api.js` | Thin REST client over `/api/images`. |
| `web/selection.js` | Selection-mask builders and marching-ants geometry. |
| `web/icon.svg` | Plugin identity glyph (mapped to shared `apps/image`). |

Imports in `editor.js` go **three levels up** to the app root
(`../../../ui/index.js`, `../../../js/contextMenu.js`, `../../../js/files.js`)
because the module lives at `plugins/image/web/`.

---

## `web/plugin.js` — surface contract

```js
export const IMAGE_PLUGIN = 'image';
export function mountImageTile();        // → editor.mountEditor()
export function unmountImageTile();      // → editor.unmountEditor()
export function getImageTileElement();   // → editor.getEditorElement()
export function wireImageEvents();       // → editor.wireEditorEvents()
export function imageContextMenu();      // → editor.editorContextMenu()
export default { name, icon, mount, unmount, getElement, wireEvents, contextMenu };
```

`default.icon = 'ui/image'`; `default.name = 'image'`. The host calls
`mount()` to get the tile element, `wireEvents()` once after mounting, and
`contextMenu()` to obtain the right-click entries core splices into the window
menu (core supplies the surrounding separators and window management).

---

## Mount lifecycle

`mountEditor()` (`editor.js:2330`):

1. `buildWorkspace()` builds the tile and all DOM, returning the root.
2. `renderOptionsBar()` paints the active tool's options.
3. `wireDropTarget(S.els.area)` listens for image drops.
4. A `ResizeObserver` on the canvas area re-fits (`paintView`) or redraws rulers.
5. `void openNewest()` opens the most recent document asynchronously.
6. Returns the tile element.

`unmountEditor()` (`editor.js:2349`) stops the marching-ants timer, removes the
keydown listener, disconnects the observer, removes the tile, clears all state
(`S.els`, docs, layers, mask, render, history) and revokes every thumbnail
`ObjectURL`. `wireEditorEvents()` is idempotent (`S.wired`).

State lives in the singleton `S` (`editor.js:161`): `docs`, `doc`, `layers`,
`activeLayerId`, `fg`/`bg` colours, `tool`, `toolOptions`, `mask`/`maskW`/`maskH`,
`zoom`/`fit`, `render`, `history`, `thumbs`, `inline`, `resizeObs`, etc.

---

## DOM structure & CSS prefix

The tile is `section.tile.image-tile` with `data-plugin="image"`; the app bar
carries `data-window-bar=""`. The class tree (all `image-*`):

```
section.tile.image-tile[data-plugin=image]
├── div.image-appbar[data-window-bar]      menu button (.image-logo), title, toggle-panels
├── div.image-options                      per-tool options bar (lead icon + fields)
├── div.image-tabs                         document tabs
├── div.image-main
│   ├── div.image-tools                    two-column tool palette (buttons .image-tool)
│   ├── div.image-workspace
│   │   ├── div.image-ruler-corner
│   │   ├── canvas.image-ruler-h
│   │   ├── canvas.image-ruler-v
│   │   └── div.image-canvas-area
│   │       └── div.image-doc
│   │           ├── canvas.image-view      painted composite
│   │           └── canvas.image-overlay   selection ants, crop, handles
│   └── div.image-dock                     panel dock
│       ├── Adjustments  (div.image-panel-section)
│       ├── Properties
│       ├── Color
│       ├── Swatches
│       ├── Layers
│       ├── Channels
│       ├── History
│       └── Navigator
└── div.image-statusbar                    tool hint, position, size, zoom
```

Other prefixes: `.image-icon`, `.image-menu-title`, `.image-opt-field`,
`.image-opt-number`, `.image-opt-select`, `.image-opt-modes`, `.image-adjust-*`,
`.image-props`, `.image-color`, `.image-fg-bg`, `.image-panel-*`. The dock can be
hidden by toggling `.is-dock-hidden` on the tile.

The tool palette (`TOOL_GROUPS`, `editor.js:44`) groups Move, Marquee, Lasso,
Magic Wand, Crop, Eyedropper, Clone Stamp, Brush/Pencil, Eraser, Gradient, Paint
Bucket, Blur/Sharpen/Smudge, Dodge/Burn, Type, Shape, Hand and Zoom. Each group
shows its active tool; long-press/right-click opens a fly-out
(`openToolFlyout`, `editor.js:373`). `TOOL_HINTS` feeds the status bar.

---

## `web/editor.js` exports

| Export | Purpose |
|---|---|
| `PLUGIN_NAME`, `BLEND_MODES`, `S` | Identity, canonical blend list, mutable state. |
| `rgbCss`, `hexOf`, `parseHex` | Colour helpers. |
| `iconButton` | Core-button wrapper with an icon span. |
| `buildWorkspace`, `buildToolPalette`, `selectTool`, `renderOptionsBar`, `findTool` | Workspace + palette + options. |
| `buildDock`, `refreshPanels`, `buildLayersBody`, `renderLayerPanel` | Panel dock. |
| `withActiveLayer` | Runs an async action only when a layer is active. |
| `plotRaw`, `paintView`, `setZoom`, `zoomToFit`, `updateStatus`, `setStatusPos` | Canvas painting + status. |
| `applySelectionMask`, `clearSelectionState`, `loadSelection`, `selectAll`, `deselect`, `inverseSelection`, `drawOverlay` | Selection. |
| `base64FromRgba`, `queueOp`, `applyOp`, `documentCrop`, `reloadComposite` | Operations pipeline from the window. |
| `refreshImages`, `openImage`, `openNewest`, `renderTabs` | Document tabs. |
| `pickFile`, `uploadFile`, `placeEmbedded`, `downloadCurrent`, `deleteCurrent`, `resetCurrent` | File actions. |
| `renderHistory`, `refreshHistory` | History panel. |
| `mountEditor`, `getEditorElement`, `unmountEditor` | Lifecycle. |
| `openMainMenu`, `openAdjustment`, `openFilter`, `closeInlinePanel` | Menus + inline panels. |
| `wireEditorEvents`, `importFromFiles`, `editorContextMenu`, `wireDropTarget` | Host integration. |

The main menu (`openMainMenu`, `editor.js:2374`) is a flat core context menu with
the group headings **File, Edit, Image, Layer, Select, Filter, View, Window,
Help**. Core menus support one submenu level, so Filter and Layer keep theirs but
groups are not nested.

`undo()` / `redo()` (`editor.js:2320`) and history restore are **stubs** that
toast “not available in this build”.

---

## `web/api.js` — REST client

All calls go through `apiFetch` from `../../js/api.js` and unwrap the
`{success,data}` envelope. `enc = encodeURIComponent`.

| Function | Route |
|---|---|
| `listImages` / `createImage` | `GET` / `POST /api/images` (`createImage` posts the `File` as a raw body with `?name=`). |
| `fetchImage` / `renameImage` / `deleteImage` | `GET` / `PUT` / `DELETE /api/images/:id`. |
| `fetchRender(id)` | `GET /api/images/:id/render?raw=true` → `{ w, h, buf: ArrayBuffer }` via auth headers + `x-image-*`. |
| `rawApply(id, ops, {commit, layerId})` | `POST /api/images/:id/apply?raw=true&commit=…` → raw RGBA. |
| `applyOps(id, ops, layerId)` | `POST /api/images/:id/apply` (JSON, no pixel return). |
| `listLayers` / `createLayer` | `GET` / `POST /api/images/:id/layers`. |
| `updateLayer` / `deleteLayer` | `PUT` / `DELETE …/layers/:layer_id`. |
| `duplicateLayer` / `mergeLayer` | `POST …/layers/:layer_id/duplicate` / `/merge`. |
| `reorderLayers` / `flattenImage` | `POST …/layers/reorder` / `…/flatten`. |
| `uploadLayer` / `replaceLayerImage` | import/replace bytes (raw body). |
| `layerThumbUrl` / `fetchLayerThumb` | `…/layers/:layer_id/thumb`. |
| `getSelection` / `setSelection` / `clearSelection` | `GET` / `PUT` / `DELETE …/selection` (`setSelection` base64s a `Uint8Array` in 32 KP chunks). |
| `cropDocument` / `resizeDocument` / `rotateDocument` / `flipDocument` | document geometry POSTs. |
| `fetchPng` | `GET /api/images/:id/data` as a blob (export). |
| `blobFromHome(path)` | `GET /api/files/raw?path=…` (Files plugin) for “open from Files”. |

`authHeaders()` attaches `Authorization: Bearer <getToken()>` to the `fetch`
calls that bypass `apiFetch` (raw pixel streams). Errors are surfaced via
`errorText(res)`, which prefers the JSON `error`/`message`.

---

## `web/selection.js` — mask maths

| Export | Purpose |
|---|---|
| `newMask` / `maskAll` / `invertMask` / `maskIsEmpty` / `maskBounds` | Mask creation + queries. |
| `maskShape(w,h,{shape,x,y,width,height})` | Rect/ellipse with 3× supersampled edges. |
| `maskPolygon(w,h,points)` | Lasso fill (2× supersampled, even-odd). |
| `maskWand(rgba,w,h,{x,y,tolerance,contiguous})` | Magic wand over RGBA (contiguous flood or global). |
| `combineMask(base,next,w,h,mode)` | `new` / `add` / `subtract` / `intersect`. |
| `featherMask(mask,w,h,radius)` | Separable box-blur feather. |
| `maskEdges(mask,w,h)` | Boundary segments for the marching-ants overlay. |

Masks are uploaded once per change through `api.setSelection`; the server then
confines every operation. `uploadSelection()` (`editor.js:1358`) debounces
uploads via `selectionUploadTimer`.

---

## Events

| Event | Direction | Handler |
|---|---|---|
| `agent:actions` | window ← core | `onAgentActions` (`editor.js:3240`): filters `/^image_/`, dispatches `plugin:focus`, refreshes the image list, and either opens the touched document, reopens the newest after a delete, or reloads the composite/layers/selection for the current document. |
| `plugin:focus` | window → core | Dispatched by `onAgentActions` to raise the Image window. |
| `onOpenFromFiles` | Files plugin hook | `importFromFiles(path, name)` (`editor.js:3222`) downloads from home and uploads. |
| keydown (window) | — | `onKeyDown` (`editor.js:2087`) with `TOOL_KEYS` shortcuts; removed on unmount. |

`editorContextMenu()` (`editor.js:3261`) returns: Open image, New layer/group,
Duplicate, Merge down, Flatten, an **Adjustments** submenu, Select all /
Deselect, Revert to original, Delete layer/image.

`wireDropTarget(area)` (`editor.js:3284`) drags an image file onto the canvas:
with a document open it is a **place embedded**; with none it uploads a new
document.

---

## Documented gotchas

- **No plugin CSS.** Every class is `image-*`; the styling is core-owned. If a
  new control needs a look, edit `web/css/tiles.css`, not the plugin.
- **The window and the AI share one engine.** The window never mutates pixels
  locally; it calls `rawApply`/`applyOps`, and the same `src/ops.rs` backs both
  the UI and the agent tools. Any new operation must be added server-side.
- **Selection is per document and server-confined.** The window only builds the
  mask; the server crops it to the target layer and confines operations. A
  geometry operation clears the selection, so a batch that resizes then paints is
  not masked after the resize.
- **Raw pixel path is not JSON.** `fetchRender`/`rawApply` read dimensions from
  `x-image-width`/`x-image-height` headers and the body as an `ArrayBuffer`; they
  bypass `apiFetch` and must carry the Bearer token themselves.
- **Module-level singleton.** All state is in `S`; unmount must clear it or a
  later mount reuses stale docs/layers. Thumbnails are `ObjectURL`s and are
  revoked on unmount.
- **Legacy documents.** Migration 003 wraps a pre-layer row into a
  `Background` layer; `ensure_base_layer` is the runtime backstop, so the window
  can always expect a non-empty stack.
- **Undo/redo/history-restore are stubs** that toast an “not available” message;
  do not advertise them.
- **`../../../` imports** are intentional: the module is three directories deep.
