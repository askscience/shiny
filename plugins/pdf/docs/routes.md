# PDF — REST routes

All `auth`, scoped to the caller. Edits commit through `store::commit` and reply
with `{ pdf_id, page_count, updated_at, history }`.

| Method | Path | Body | Purpose |
|---|---|---|---|
| GET | `/api/pdfs` | — | List (`editable`, `pdf_stale` per document). |
| POST | `/api/pdfs` | `{ title, content, format, page_numbers? }` | Create. `format: "html"` = editor document (placeholder PDF, `pdf_stale`); Markdown/plain are composed. |
| POST | `/api/pdfs/preview` | same | Compose without saving → PDF bytes. |
| POST | `/api/pdfs/import` | multipart `file` | Import a PDF. |
| GET | `/api/pdfs/:id` | — | Metadata, `editor_html`, `pdf_stale`, `history`. |
| PUT | `/api/pdfs/:id` | `{ title }` | Rename. |
| DELETE | `/api/pdfs/:id` | — | Delete with history. |
| PUT | `/api/pdfs/:id/source` | `{ content, format, title? }` | Save source (`html`: text only, PDF marked stale; Markdown: regenerate). |
| POST | `/api/pdfs/:id/replace-file` | multipart `file` | Upload the shell-printed export (undoable). |
| POST | `/api/pdfs/:id/render-html` | — | Export without the shell (compose fallback). |
| GET | `/api/pdfs/:id/file` · `/export` | — | Inline bytes (viewer) · download. |
| GET | `/api/pdfs/:id/text/:page` | — | Text in reading order (PdfCraft). |
| GET | `/api/pdfs/:id/comments` | — | `{ comments: [{page,index,kind,text,author,rect,color,parent}], marks }`. |
| POST | `/api/pdfs/:id/annotate` | `{ page, kind, rect\|rects, text?, color?, size? }` | Add a comment: `highlight`, `underline`, `strikeout`, `squiggly`, `note`, `free_text`, `box`, `oval`, `link`. |
| POST | `/api/pdfs/:id/ink` | `{ page, strokes, color? }` | Freehand drawing. |
| POST | `/api/pdfs/:id/comments/edit` | `{ page, index, action: delete\|text\|move\|reply, text?, dx?, dy? }` | Change a comment. |
| POST | `/api/pdfs/:id/header-footer` | `{ slots[6], font_size? }` | Header & footer (`<<1>>`, `<<n>>`, dates). |
| POST | `/api/pdfs/:id/watermark` | `{ text }` | Watermark. |
| GET · POST | `/api/pdfs/:id/marks` · `/marks/remove` | `{ kind }` | List / remove `watermark`, `header_footer`. |
| POST | `/api/pdfs/:id/rotate` · `/reorder` · `/delete-pages` · `/duplicate-page` · `/insert-blank` · `/merge` · `/extract` | | Page organisation. |
| POST | `/api/pdfs/:id/replace-text` | `{ old, new, page?, all_pages? }` | Find & replace (foreign PDFs). |
| POST | `/api/pdfs/:id/undo` · `/redo` | — | History. |
| GET · POST | `/api/pdfs/:id/runs/:page` · `/edit-text` | | Legacy line editing (not used by the window). |
