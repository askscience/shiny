# PDF plugin

Write documents and read PDFs — two jobs, two surfaces:

- **Write** — documents made here are HTML on A4 pages (`web/editor.js`), with
  themes (Modern, Editorial, Classic: Inter, Inter Display, Instrument Serif,
  P052), templates (letter, invoice, report, notes) and a formatting bar. The
  PDF is an **export**: the PEAK'D shell prints the same HTML with Chromium
  (`peakd:print:`), so the PDF is exactly what was on screen. A PDF's text is
  never edited in place for these documents — change the text, update the PDF.
- **PDF** — any PDF (imported, exported, or made by the assistant) opens in
  pdf.js. Markup adds **real PDF comments** through PdfCraft on the server:
  highlight / underline / strikethrough on a text selection, comments, text
  boxes, freehand drawing, boxes and ovals, links. The Comments panel edits,
  answers or deletes them; header & footer and watermark are removable marks;
  the rail organises pages. Every change is undoable (Ctrl+Z).

Category: **Office**. See the [plugin docs index](../../../docs/plugins/README.md).

## Engines

| Job | Code |
|---|---|
| Document → PDF (exact) | Shell print: `crates/peakd` `peakd_qt_print_html` (Chromium `printToPdf`) |
| Document → PDF (no shell) | `src/compose.rs` (HTML → laid-out Base-14 PDF; approximate) |
| Comments, marks, pages, text | PdfCraft (`src/craft.rs`, MIT OR Apache-2.0, pinned git dependency — see [NOTICE](../NOTICE.md)) |
| Find & replace in foreign PDFs | `src/stream_edit.rs` (font-aware content-stream editing on lopdf) |
| Assistant `pdf_create` | `src/compose.rs` (Markdown/HTML → PDF; opens later in the editor) |
| History | `src/store.rs` (`pdf_revisions`: 25 undo / redo steps per document) |

## Manifest

`plugin.toml` — `name = "pdf"`, `version = "0.3.0"`, `api_level = 1`,
`skills_dir = "skills"`, `web_dir = "web"`, `category = "Office"`. Migrations in
`migrations/` (001 documents · 002 revisions · 003 source · 004 `pdf_stale`).

## Surface

- **13 agent tools** — [tools.md](tools.md).
- **REST routes** under `/api/pdfs` — [routes.md](routes.md).
- **Window** — [window.md](window.md); styles in core `web/css/tiles.css`
  ("PDF plugin").

## Build, test, install

```bash
cargo test -p shiny-pdf-plugin
cargo build --release -p shiny-pdf-plugin -p peakd   # peakd for exact export
node plugins/pdf/scripts/verify_pdfjs.mjs <dir>      # independent pdf.js check
```

Install = `plugin.toml`, `migrations/`, `skills/`, `web/`,
`libshiny_pdf_plugin.so` in a plugins directory. Replace an installed copy by
**renaming** the directory, never by overwriting the `.so` of a running server.

Fonts: the document themes use installed system fonts (Inter, Inter Display,
Instrument Serif, P052); where one is missing the theme falls back to a
similar face.
