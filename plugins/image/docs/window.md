# Image window

The window is a **minimal photo editor**: a window bar (the title in display
type over a mono meta line), a borderless canvas that fills the window, and a
flat bottom dock on a hairline with a filmstrip of looks and a bank of rotary
dials, each under a mono eyebrow (`LOOKS`, `ADJUST`). The dials are the shared `dial` control from
`web/ui/index.js` (`docs/core/ui-components.md`); this plugin ships DOM only —
its dock layout comes from the core stylesheet, the dial look from the library
and its tokens.

---

## Layout

```
┌─ menu  Title ──────────────────────────────── (1:1) ───┐
│        4032×3024 · 63% · LOOK: NOIR                     │
│                    [ image, borderless ]               │
├─────────────────────────────────────────────────────────┤
│ LOOKS                                                   │
│ [img][img][img][img][img][img]…  ← grey square tiles    │
│ PORTRA FADED NOIR …              ← names in mono below  │
│ ─────────────────────────────────────────────────────── │
│ ADJUST                                                  │
│  (Bright) (Contrast) (Exposure) …  ← 8 dials, drag vertically │
│ (Auto tone)(Auto contrast)(Invert)(B&W)(Sepia) (Revert) │
└─────────────────────────────────────────────────────────┘
```

## DOM & classes

| Region | Classes |
|---|---|
| Tile | `section.tile.image-tile[data-plugin=image]` |
| Window bar (merged) | `.image-appbar[data-window-bar]` → `.image-appbar-head` (`.image-appbar-title`, `.ui-meta.image-appbar-meta`), `.image-appbar-right` → `.image-zoom-chip` (mono pill: `1:1` while fitting, `Fit` otherwise) |
| Stage | `.image-canvas-area` (drop target, `is-drop`) → `canvas.image-view` (`is-panning`) |
| Empty state | `.image-empty` → core `emptyState` + `.image-empty-open` button |
| Dock | `.image-dock` → `.image-dock-body`, `.ui-eyebrow.image-dock-eyebrow`, `.image-dock-divider` |
| Looks | `.image-look-row` (`--look-url`, `--look-n`) → `.image-look-chip` (per chip `--i`; `is-active` = the selected look) → `.image-look-thumb`, `.image-look-name` |
| Adjust | `.image-adjust` → `.image-dial-row` → shared `.ui-dial` (`role="slider"`; library classes `.ui-dial-knob`, `.ui-dial-svg/track/arc`, `.ui-dial-value`, `.ui-dial-label`; `is-set`/`is-dragging`); `.image-oneclicks` → `.ui-btn.image-oneclick` (`--revert`) |

Each look chip is a grey square tile showing the open image with that look
applied (`.image-look-thumb`'s `background-image` slices the strip PNG using
`--i` and `--look-n`) with the name in mono capitals below; the name is also
the `title` / `aria-label`. The selected look gets an ink outline and an
underlined ink name — no accent ring, no lift shadow. One-clicks are grey
pills (`button` variant `ghost`; Revert `quiet`); the empty state's
**Open image** is the ink pill with a trailing arrow.

The meta line reads `W×H · ZOOM% · LOOK: NAME` and updates on every paint and
look change. Motion: the look tiles cascade in (`listReveal`) once per
document when the strip first arrives; a new image's first paint enters with
`cardEnter`. The canvas has a hairline outline instead of a drop shadow.

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

There is no plugin CSS. The `image-*` block lives in `web/css/tiles.css`; the
themes only recolour it through tokens. The strip geometry is driven by custom
properties: `--look-url` (the strip image), `--look-n` (tile count) and `--i`
(chip index, inherited by `.image-look-thumb`) — the CSS slices a `64px` grid
out of the `96px` tiles.

## Smoke test

`node --experimental-vm-modules plugins/image/web/plugin.smoke.mjs` mounts the
real `editor.js` against a DOM shim and drives the dock: load, the filmstrip
(chips, labels, strip paint, select / swap / clear + the active ring), dial
drag preview/commit/re-centre with arc fill, arrow-key nudges, one-clicks,
rotate, the `agent:actions` refresh and the Files import.
