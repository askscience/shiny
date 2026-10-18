# Image window

The window is a **minimal photo editor**: a window bar, a borderless canvas
that fills the window, and a bottom dock with a filmstrip of looks and a bank
of rotary dials. The dials are the shared `dial` control from
`web/ui/index.js` (`docs/core/ui-components.md`); this plugin ships DOM only —
its dock layout comes from the core stylesheet, the dial look from the library
and its tokens.

---

## Layout

```
┌─ menu  Title ───────────────── window controls (shell) ┐
│                    [ image, borderless ]               │
├─────────────────────────────────────────────────────────┤
│ [img][img][img][img][img][img]…  ← look filmstrip, no labels │
│ ─────────────────────────────────────────────────────── │
│  (Bright) (Contrast) (Exposure) …  ← 8 dials, drag vertically │
│ [Auto tone][Auto contrast][Invert][B&W][Sepia] [Revert] │
└─────────────────────────────────────────────────────────┘
```

## DOM & classes

| Region | Classes |
|---|---|
| Tile | `section.tile.image-tile[data-plugin=image]` |
| Window bar (merged) | `.image-appbar[data-window-bar]` → `.image-appbar-title`, `.image-appbar-right`, `.image-zoom-chip` |
| Stage | `.image-canvas-area` (drop target, `is-drop`) → `canvas.image-view` (`is-panning`) |
| Empty state | `.image-empty` → core `emptyState` + `.image-empty-open` button |
| Dock | `.image-dock` → `.image-dock-body`, `.image-dock-divider` |
| Looks | `.image-look-row` (`--look-url`, `--look-n`) → `.image-look-chip` (per chip `--i`; `is-active` = the selected look) |
| Adjust | `.image-adjust` → `.image-dial-row` → shared `.ui-dial` (`role="slider"`; library classes `.ui-dial-knob`, `.ui-dial-svg/track/arc`, `.ui-dial-value`, `.ui-dial-label`; `is-set`/`is-dragging`); `.image-oneclicks` → `.image-oneclick` (`--revert`) |

Look chips carry **no visible text** — the effect name lives in `title` /
`aria-label`, and the visual is the open image with that look applied
(`background-image` slices the strip PNG using `--i` and `--look-n`).

## Exports

`web/plugin.js` is the session surface the host loads (`name`, `icon`,
`mount`, `unmount`, `getElement`, `wireEvents`, `contextMenu`); it re-exports
these `web/editor.js` functions:

| Export | Role |
|---|---|
| `mountEditor()` / `getEditorElement()` / `unmountEditor()` | Build the tile, hand it to the host, tear down listeners + object URLs (including the strip). |
| `wireEditorEvents()` | Idempotent: `agent:actions` listener + `onOpenFromFiles` registration. |
| `importFromFiles(path, name)` | Files-plugin hand-off (double-click, chat "Open"): reads the home file, uploads it as a new document. |
| `editorContextMenu()` | Entries core splices into the window's right-click menu. |

## Mount lifecycle

1. `mountEditor()` builds the workspace, wires the drop target and a
   `ResizeObserver` (refits while `S.fit`), then `openNewest()`.
2. `openNewest()` → `refreshImages()` → `openImageById()` (full metadata, so the
   look list is present) → `openImage(meta)` → `renderLooks()` (chips first,
   then the strip PNG paints in) + `reloadComposite()` (raw RGBA → canvas).
3. No documents: `paintView()` renders the empty state with an **Open image**
   button; dropping a file on the stage uploads it.

## Dock behaviour

- **Looks** preview on the open picture: `GET /api/images/:id/looks` returns one
  PNG holding all 16 tiles (rendered over the current dial settings); each chip
  shows its slice. A look is a **live, swappable selection** — clicking a chip
  sends `{op:"look", name}` (the server keeps it in `images.look`, the picture
  renders `look(settings(bytes))`), another chip **replaces** it, and clicking
  the ringed chip clears it. Only destructive edits bake selections in. The
  strip refreshes after edits, never after a look swap (the base is unchanged).
- **Dials are persistent settings** (`role="slider"`): each value lives on the
  document (`images.adjust`) and the picture renders as `look(settings(bytes))`.
  Turning a knob sends a partial settings update — live preview while moving
  (`commit=false`), one commit on release (`commit=true`) — and the dial **stays
  where you put it**, so fine-tuning a value **replaces** it instead of stacking
  deltas. Drag vertically (160 px = full range; hold Shift for fine control),
  scroll to nudge, arrow keys (Shift = ×10), `Home`/`End` jump to the range
  limits, `Delete`/`Backspace`/double-click reset one dial, Escape cancels back
  to the committed pixels. Destructive edits
  (one-clicks, rotate/flip, AI) bake the live state in and the dials snap to 0
  from the server's echoed state; the look strip follows every real commit
  (450 ms trailing debounce).
- **One-clicks** (`Auto tone`, `Auto contrast`, `Invert`, `B&W`, `Sepia`) apply
  immediately; **Revert** sends the `reset` op (back to the upload).
- **Rotate / flip** live in the Image menu and go through the operations engine
  (`rotate`, `flip_h`/`flip_v`), so previews and history stay coherent.

## Events

| Event / source | Effect |
|---|---|
| `files:open` (`onOpenFromFiles`) | `importFromFiles(path, name)` — extension allowlist mirrors `FILE_OPENERS`. |
| `agent:actions` (any `image_*` action) | Focus the plugin, refresh the doc list; open the touched document or reload the pixels + look strip. |
| Drop on the stage | Upload the file as a new document. |
| Wheel / double-click / drag | Zoom around the cursor; toggle fit ↔ 100%; drag pans. `Ctrl +/-`, `Ctrl+0` also work. |

## CSS

There is no plugin CSS. The `image-*` block lives in `web/css/tiles.css` with a
mirror in the pitch theme (`web/themes/pitch/app.css`). Keep the two in sync
when adding classes. The strip geometry is shared between the two via custom
properties: `--look-url` (the strip image), `--look-n` (tile count) and `--i`
(chip index) — the CSS slices a `64px` grid out of the `96px` tiles.

## Smoke test

`node --experimental-vm-modules plugins/image/web/plugin.smoke.mjs` mounts the
real `editor.js` against a DOM shim and drives the dock: load, the filmstrip
(chips, labels, strip paint, select / swap / clear + the active ring), dial
drag preview/commit/re-centre with arc fill, arrow-key nudges, one-clicks,
rotate, the `agent:actions` refresh and the Files import.
