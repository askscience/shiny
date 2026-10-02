# PDF — window surface

[`web/plugin.js`](../web/plugin.js), class prefix `pdf-*`.

```js
export default {
  name: 'pdf',
  mount: mountPdfTile,
  unmount: unmountPdfTile,
  getElement: getPdfTileElement,
  wireEvents: wirePdfEvents,
  contextMenu: pdfContextMenu,
};
```

## How it renders

The window renders **the real document in the browser**, not a picture of it:

- Loads PDF.js from the vendored [`web/vendor/pdfjs`](../../../web/vendor/pdfjs)
  and paints each page to a `<canvas>` at `zoom × devicePixelRatio`.
- Lays PDF.js's **text layer** over the canvas, so text is selectable and
  copyable and zoom stays crisp with no server round-trip.
- `cMapUrl` / `standardFontDataUrl` point at the vendored `cmaps/` and
  `standard_fonts/`, so CID-keyed docs and PDFs without embedded fonts render.
  **Don't drop those directories.**
- Thumbnails come from the same loaded document (quick pass, then sharp), so
  the rail costs no extra network requests.
- The page bytes come from `GET /api/pdfs/:id/file` (inline, `no-store`).

Server-side rendering still exists (`pdf_oxide` `render_png`, used by
`/pages/:page`) for anything that needs a bitmap.

## Gotchas (learned the hard way)

- **Never let a flex parent size the page.** `.pdf-canvas`/`.pdf-page` set
  `align-items: flex-start`, and the viewer pins the page box from the PDF.js
  viewport. Flex's default `stretch` otherwise changes the aspect ratio —
  pages render squashed, circles become ellipses.
- **Set `--scale-factor`** on the page element (the value PDF.js positions text
  spans with); it must equal the viewport scale or the text layer drifts.
- **Annotation gesture is Shift+drag.** A plain drag belongs to text selection.
  Region selection converts to PDF points via the viewport scale and posts the
  rect in points (A4 = 595×842).
- **Capture the selection before dismissing the annotation bar**, and capture
  `annotRect` before hiding the bar — the bar's document-level `pointerdown`
  handler runs before a button's `click`, so hiding first nulls the rect and
  every annotation silently no-ops.
- **After any server-side edit, call `dropPdfDoc()` and re-render.** The bytes
  changed, so the parsed document and pages are stale. `rotate`, `reorder`,
  `delete-pages`, `merge`, `replace-text`, `annotate` and `watermark` all take
  this path — including when the agent triggers them (`agent:actions`).

## Inline text editing

The viewer edits raw content-stream text via the same `stream_edit` engine the
agent uses (`/edit-text`), so AI and human parity holds. Style controls
(bold/size/colour) are intentionally absent — see
[architecture](architecture.md#why-lopdf-for-text-replacement).

## Development

The app serves the installed copy at `data/plugins/pdf/web/`. Copy or reinstall
after editing.
