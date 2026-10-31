# PDF — architecture

```
src/
├── plugin.rs        manifest, routes, tools
├── routes.rs        /api/pdfs/* handlers
├── tools/mod.rs     pdf_* agent tools
├── store.rs         commit with undo snapshot, undo/redo, history
├── ops.rs           the API routes and tools call (delegates below)
├── craft.rs         PdfCraft bridge: comments, marks, pages, text (bytes in → bytes out)
├── compose.rs       Markdown/HTML/plain → PDF (assistant, export fallback) + Markdown → editor HTML
├── stream_edit.rs   font-aware find & replace in existing PDFs (lopdf)
├── doc.rs           lopdf helpers (page tree rebuild for subsets/repeats)
└── fonts.rs, afm.rs Base-14 metrics + WinAnsi for compose/stream_edit
web/
├── plugin.js        the window: Write | PDF, markup tools, Comments panel, rail
├── editor.js        A4 document editor + export (shell print / server fallback)
├── doc-themes.js    one stylesheet for editor and print, themes, templates
└── sanitize.js      the editor's HTML vocabulary (paste, load, save)
```

## Why documents are exported, not edited as PDFs

A PDF is positioned glyphs, not a document. Editing its text means rewriting
content streams, which works for small corrections but breaks layout as soon
as text grows — and in PDFs printed by Chromium it is unreliable even in
PdfCraft (verified). So documents made here keep their HTML as the source of
truth; the PDF is regenerated. Updating replaces comments made on the old PDF
(the window warns; Undo restores them).

## Exact export (shell print)

`editor.js` builds a standalone page (`doc-themes.js` `printHtml`) and posts
`peakd:print:{id, html, options}`. peakd writes it to a private temp dir,
loads it in an off-screen `QWebEnginePage` (own off-the-record profile,
JavaScript off, no remote or file access) and calls `printToPdf` with the page
size and margins; the bytes return as `window.__peakdPrinted({id, ok, data})`
and the window uploads them (`POST /:id/replace-file`). The shell advertises the
feature as `window.__peakdCaps.print`; without it the window uses
`POST /:id/render-html` (compose fallback).

## PdfCraft

Pinned git dependency (`plugins/pdf/Cargo.toml`) plus `[patch.crates-io]`
entries in the workspace root for PdfCraft's forks of hayro, vello and lopdf
(patches only apply at the root of a build). Each call opens the bytes,
edits, and writes a full save; the plugin's own history keeps old versions.

## Data

`pdf_documents` (bytes, title, page_count, source, source_format,
page_numbers, pdf_stale), `pdf_revisions` (undo/redo snapshots).

[README](README.md) · [tools](tools.md) · [routes](routes.md) · [window](window.md)
