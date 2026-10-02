# PDF — agent tools

Page indices are **0-based**. Annotation `rect` is `[x, y, width, height]` in
PDF points with a bottom-left origin (Letter `612×792`, A4 `595×842`). Without
`pdf_id`, most tools target the most recently used document.

| Tool | Params | Returns / behaviour |
|---|---|---|
| `pdf_create` | `{ title, content, format?: "html"\|"markdown"\|"plain", css? }` | Creates a PDF from semantic HTML/Markdown (built-in renderer). Returns the new `pdf_id`. |
| `pdf_list` | `{}` | `pdf_id`, `title`, `page_count`, `updated_at`. |
| `pdf_read` | `{ pdf_id?, page? }` | `{ content, page_count }`; omit `page` for all pages. |
| `pdf_rotate` | `{ pdf_id?, pages?: [n], degrees, all? }` | Rotate clockwise (`degrees` multiple of 90). |
| `pdf_delete_pages` | `{ pdf_id?, pages: [n] }` | Remove pages. |
| `pdf_reorder` | `{ pdf_id?, order: [n] }` | Document becomes exactly these pages. |
| `pdf_merge` | `{ pdf_id?, other_id }` | Append `other_id`'s pages to `pdf_id`. |
| `pdf_replace_text` | `{ pdf_id?, page?, old, new }` | Find & replace via `stream_edit` (Form XObjects + TJ arrays handled). |
| `pdf_annotate` | `{ pdf_id?, page?, kind, rect, text?, color? }` | `kind`: `highlight`/`underline`/`strikeout`/`squiggly`/`note`/`free_text`/`link`. |
| `pdf_add_note` | `{ pdf_id?, page?, text, x?, y? }` | Sticky note without a rectangle; prefer over `kind:"note"`. |
| `pdf_watermark` | `{ pdf_id?, text, pages?, all? }` | Diagonal text watermark. |
| `pdf_delete` | `{ pdf_id, confirm? }` | Delete the document. |

## `pdf_create` formatting

Structure drives the professional look: an `<h1>` title, `<h2>`/`<h3>` sections,
`<table>` for tabular data, `<ul>`/`<ol>` lists, `<strong>`/`<em>` emphasis,
`<blockquote>` callouts, `<hr>` between sections, `<pre>`/`<code>` for code.
**CSS and inline `style` are not applied** by the reliable renderer — express
importance with headings, bold, lists and tables.

## Rules

- Change a word/name/number → `pdf_replace_text`.
- Mark up a page → `pdf_annotate` (or `pdf_add_note` for a note).
- Change orientation → `pdf_rotate`; remove/reorder → `pdf_delete_pages` /
  `pdf_reorder`; combine → `pdf_merge`.
- After a server-side edit the window drops and re-renders the parsed document.

## Registration

All tools are wrapped with `bridged(..)` in
[`src/plugin.rs`](../src/plugin.rs); skills markdown is `skills/pdf.md`.
