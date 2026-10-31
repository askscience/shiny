# PDF — agent tools

Page indices are 0-based; `rect` is `[x, y, width, height]` in PDF points
(bottom-left origin). Without `pdf_id`, tools target the most recently updated
document. Every modifying tool is undoable (`pdf_undo`).

| Tool | Params | Behaviour |
|---|---|---|
| `pdf_create` | `{ title?, content, format?, page_numbers? }` | Markdown (or HTML) → PDF; the user can open it in the editor later. |
| `pdf_list` · `pdf_read` | `{}` · `{ pdf_id?, page? }` | List · text in reading order. |
| `pdf_annotate` | `{ pdf_id?, page?, kind, rect, text?, color? }` | Real comment: `highlight`, `underline`, `strikeout`, `squiggly`, `note`, `free_text`, `box`, `oval`, `link`. |
| `pdf_add_note` | `{ pdf_id?, page?, text, x?, y? }` | Comment at a point. |
| `pdf_watermark` | `{ pdf_id?, text }` | Removable watermark. |
| `pdf_rotate` · `pdf_delete_pages` · `pdf_reorder` · `pdf_merge` | | Pages. |
| `pdf_replace_text` | `{ pdf_id?, page?, old, new }` | Find & replace in a PDF's own text (keeps the font). |
| `pdf_undo` | `{ pdf_id?, redo? }` | History. |
| `pdf_delete` | `{ pdf_id }` | Delete (not undoable). |
