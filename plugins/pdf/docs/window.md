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

## Layout

- **Window bar**: documents menu, title, **Write | PDF** switch (documents made
  here), in PDF view page navigation, zoom (click % = fit width), the markup
  tools — comment (C), text box (T), draw (D), shape (R), link (L) — and the
  Comments panel toggle; undo / redo; for documents, "PDF up to date / not
  updated" and **Update PDF**; the grey **New** pill (template picker).
- **Editor** (Write): formatting row (paragraph style, bold/italic/underline/
  strike, colour, highlight, alignment, lists, link, image, table + row/column
  tools, line, page break, page numbers 1/n, theme) over A4 paper with dashed
  guides where pages end. Autosaves; Ctrl+S saves, Ctrl+Enter breaks the page.
- **Mode strip**: what a click does in the armed tool, its options, Done (Esc).
- **Selection bar**: selecting text offers highlight / underline / strike /
  comment / link / copy.
- **Comments panel**: comments by page with Edit / Reply / Delete, and page
  marks (watermark, header & footer) with Remove. Clicking a comment on the page
  focuses it in the panel.
- **Rail**: lazy thumbnails, drag to reorder, `…` / right-click page menu, `+`.

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

There is no server-side rasteriser: pdf.js renders everything in the browser.

## Gotchas (learned the hard way)

- **Never let a flex parent size the page.** The viewer pins the page box from
  the pdf.js viewport.
- **Set `--scale-factor`** on the page element to the viewport scale or the text
  layer drifts.
- **All PDF↔screen maths goes through the viewport** (`convertToViewportRectangle`
  / `convertToPdfPoint`), so overlays, clicks and drags stay right on rotated
  pages and non-zero MediaBoxes.
- **Every server edit goes through `edit()`**: it applies the returned history,
  drops the pdf.js document and re-renders (agent edits too, via `agent:actions`).
- **No confirmations for undoable edits** (page delete included); the toast says
  how to undo. Deleting a whole document still confirms — it is not undoable.

## Editing text

Documents made here are edited in **Write** and exported again. PDFs from
elsewhere get find & replace (doc menu) and comments; their text is not
retyped in place.

## Development

The app serves the installed copy at `data/plugins/pdf/web/`. Copy or reinstall
after editing.
