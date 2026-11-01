# Word — window (`web/plugin.js`, `web/page.js`)

The Word window is a page-faithful word processor in peakd's office pattern
(the same as the PDF window): a flat bar merged with the window controls, one
quiet formatting row, white paper on the bare window, and a mono status line
at the bottom. It is built only from the shared library (`web/ui/index.js`:
`toolbar`, `toolButton`, `menuButton`, `segmented`, `select`, `input`, `modal`,
`field`, `toggleRow`, `menuSurface`/`menuItem`, `placePopover`,
`controlButton`, `confirmDialog`/`promptDialog`, `toast`); the paper itself is
domain content and keeps the document's own typography.

See also: [README.md](README.md) · [routes.md](routes.md) · [tools.md](tools.md)
· [architecture.md](architecture.md) · [shared UI](../../../docs/core/ui-components.md).

## Layout

```
┌ bar (data-window-bar) ───────────────────────────────────────────────────┐
│ [Documents ▾] Title…           ↶ ↷ │ (Page|Preview) │ AI  🎨  ⤓ │ + New │
├ format row ──────────────────────────────────────────────────────────────┤
│ Normal▾ Theme font▾ Size▾ │ B I U S A ▮ │ ≡ ≡ ≡ ≡ │ • 1. ⇤ ⇥ │ 🔗 🖼 ▦ ⎘ ⌫ │ … ☰ │
├ main ────────────────────────────────────────────────────────────────────┤
│ [outline] │ desk: ruler + cover / contents / body sheets   │  (preview) │
├ status ──────────────────────────────────────────────────────────────────┤
│ PAGE 2 OF 5 · 812 WORDS · A4 · ITALIAN        SAVED 14:02   −  100%  +   │
└──────────────────────────────────────────────────────────────────────────┘
```

| Part | Elements | Notes |
|---|---|---|
| Bar | `.word-bar` | Document menu (`.word-doc-menu`, list + footer: New, Import…, Save now, Delete), editable title (`.word-title`), history cluster (undo/redo), `segmented` Page / Preview, AI menu, Design dialog, Export menu, New pill |
| Format row | `.word-format` | Paragraph style (`p`/`h1`–`h3`/`blockquote`), font (the bundled document faces, then host fonts), size (pt), bold/italic/underline/strike, text colour and highlight (swatch popovers), alignment, bullet/numbered lists and list level, link, picture, table, page break, clear formatting; table tools (`.word-cluster--table`: add/delete row/column) appear while the caret is in a table; the outline toggle sits at the end. Disabled in Preview. Scrolls as one row when narrow. |
| Outline | `aside.word-nav` | Headings 1–3, click to jump; remembered in `localStorage` (`word.nav`) |
| Desk | `.word-desk > .word-stage > .word-sheets` | The paper at its real size in mm, scaled by `zoom` |
| Preview | `.word-preview` | The exported pages, rendered by the server |
| Status | `.word-status` | Page / words / paper / language; save state with a busy dot; zoom − / label (fit to width) / + |

## The page (`web/page.js`)

`themeVars(design)` turns the stored design into CSS custom properties on
`.word-sheets`: page size and margins (`--page-w`, `--m-top`, …, in mm),
fonts, sizes in pt, colours, spacing, table and quote styling. They mirror
`engine.rs` (`Theme`, `compose`) value for value, so lines, headings and
tables in the editor are set like the export. Line height is the face's line
gap (Source Sans 3 1.326, Source Serif 4 1.371 — what the layout engine uses)
times the theme's line spacing.

`installDocumentFonts()` declares the faces the PDF embeds (Source Sans 3,
Source Serif 4) with `@font-face` rules pointing at the plugin's own public
route `/api/word-fonts/:file` — same origin, loaded only when the paper uses
them, no remote font requests.

Sheets:

- **Cover** (`.word-sheet--cover`) when `design.cover`: title on an accent
  rule at 28 % of the page, subtitle, author and date at the foot.
- **Contents** (`.word-sheet--toc`) when `design.toc` and the text has
  headings: one entry per Heading 1–3 with dotted leaders and the page it
  lands on (from the page guides); click jumps to the heading.
- **Body** (`.word-sheet--body`): the running header (`.word-hf--header`,
  title left, subtitle or author right) and page-number footer when enabled;
  the title block (`.word-titleblock`) for themed documents without a cover;
  the editor (`.word-content`, `contentEditable`); page guides; the rulers.

The cover, title block and contents are generated from the design (click
opens the Design dialog); only the body is editable.

**Page guides.** `layoutPages()` measures the body against a 100 mm probe
(`.word-probe`, so the maths holds at any zoom) and marks where the export
starts a page — every page height minus margins, or earlier at a forced page
break (a block with `break-before: page`, inserted by the toolbar or
Ctrl+Enter). The body sheet grows by whole pages; guides are labelled with
the absolute page number (after cover and contents). The exact pagination is
the Preview's.

**Rulers.** `.word-ruler--x` (top) and `.word-ruler--y` (left of page one)
show centimetres, shade the margins and carry a handle per margin
(`wireRulerHandle`): drag to set it (5–80 mm, 0.5 mm steps, the value shows
while dragging), double-click to reset to 25 mm. Each change is applied live
and saved with `PUT /design` on release. Rulers step aside below 720 px.

**Zoom.** 30–200 %; Ctrl/⌘+wheel, the − / + buttons or a click on the label
(fit to width). Until a zoom is picked the page fits the window (capped at
100 %) and refits on resize (`word.zoom = 'fit'` in `localStorage`).

## Preview

`setView('preview')` saves pending edits, calls
`GET /api/documents/:id/pages` and lays out one placeholder per page at its
size × zoom; an `IntersectionObserver` fetches each page's PNG
(`GET /api/documents/:id/page/:n?scale=`) as it nears the viewport, at
`96/72 × zoom × devicePixelRatio` pixels per point. Pages are rendered by the
server from the composed document (cover, contents with page numbers, header,
footer, themes) with WordCraft's layout and renderer — exactly the .docx/PDF
pages. Object URLs are revoked when the preview is rebuilt or the window
unmounts.

## Editing

- `exec()` runs `document.execCommand` with `styleWithCSS` on, so colours and
  highlights become `<span style>` runs that the server's HTML reader keeps.
- Font and size wrap the selection (or the paragraph at the caret) in a span
  (`styleSelection`); nested spans lose the same property.
- The selection is remembered when a select, popover or dialog takes focus
  and restored before the command runs.
- Tables: Insert table adds a 3 × 3 grid; the table tools edit rows and
  columns at the caret.
- Pictures: the Picture button, a drop or a paste go through
  `insertPictures()` (`preparePicture()` keeps small PNG/JPEG/GIF, re-encodes
  the rest on a canvas, caps the width at 600 px ≈ the A4 text width).
- Drops on the window: pictures go in at the drop point; documents (`.odt`,
  `.docx`, `.rtf`, `.md`, `.html`, `.txt`) are imported. Files dragged from the
  Files window carry `application/x-shiny-file` (`{path, name}`) and are
  fetched with `fileFromHome`.
- Shortcuts: Ctrl+Enter page break, Ctrl+S save now; the browser's own
  Ctrl+B/I/U/Z apply.

## AI

The **AI** menu sends a request to the assistant in the chat with
`askInChat()` (`web/js/textInput.js`): *In the Word document "<title>": …*
plus the selected text when there is one. The request shows in the thread
like a typed message; the agent acts on the open document — the window marks
it with `POST /api/documents/:id/opened` whenever it opens one, which makes it
the default target of every `doc_*` tool. Actions: improve the writing,
fix spelling and grammar, make it shorter, continue writing, translate…,
make it look professional (`doc_design`), illustrate with a picture
(`generate_image` → `doc_insert_image`), add a summary. The right-click menu
offers *Improve the writing* on a selection.

`onAgentActions` reacts to every `doc_*` outcome: it surfaces the window,
saves `doc_export` files to Documents (`saveExport`), and reopens the touched
document (after flushing unsaved edits) or the newest one after a delete.

## Saving and state

- Edits and title changes mark the document dirty; `persist()` saves the HTML
  1.2 s after the last change (`PUT /api/documents/:id`), and on blur, before
  switching documents, previews, exports and AI requests.
- The status shows *Saving* (with a pulsing dot), *Unsaved* or *Saved HH:MM*.
- The design is saved separately (`PUT /api/documents/:id/design`) by the
  Design dialog and the rulers.

## API calls

| Call | Method | Purpose |
|---|---|---|
| `/api/documents` | GET / POST | list / create |
| `/api/documents/:id` | GET / PUT / DELETE | open (HTML + design) / save / delete |
| `/api/documents/:id/opened` | POST | make it the agent's default target |
| `/api/documents/:id/design` | PUT | save the design |
| `/api/documents/:id/pages` | GET | page sizes for Preview |
| `/api/documents/:id/page/:n` | GET (blob) | one rendered page (PNG) |
| `/api/documents/:id/export?format=` | GET (blob) | export (`docx`, `pdf`, `odt`, `md`) |
| `/api/documents/import` | POST | import a file |
| `/api/word-fonts/:file` | GET (CSS) | document faces |

## Files integration

`wireWordEvents()` registers `onOpenFromFiles('word', …)`: `.odt`, `.docx`
and `.rtf` open in Word from the Files window (`FILE_OPENERS` in
`web/js/files.js`). Files are draggable out of the Files window (see
[Files window](../../files/docs/window.md)).

## CSS

All rules live in `web/css/tiles.css` under the `word-` prefix. The paper is
white in every theme (it is the document, not chrome); shared controls are
sized from wrapper classes (`.word-select--font`, …), never restyled.
Narrow windows (`@container plugin-window`): below 720 px the outline floats
over the page and the rulers hide; below 520 px the bar tightens. Nothing is
removed from the toolbars — they scroll as one row.
