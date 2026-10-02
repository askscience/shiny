# PDF — REST routes

All routes are `auth` and scoped to the caller. Registered in
[`src/plugin.rs`](../src/plugin.rs); handlers in [`src/routes.rs`](../src/routes.rs).

| Method | Path | Body / query | Purpose |
|---|---|---|---|
| `GET` | `/api/pdfs` | — | List documents. |
| `POST` | `/api/pdfs` | `{ title, content, format?, css? }` | Create. |
| `POST` | `/api/pdfs/import` | multipart | Import an existing `.pdf`. |
| `GET` | `/api/pdfs/:id` | — | Metadata. |
| `PUT` | `/api/pdfs/:id` | `{ title }` | Rename. |
| `DELETE` | `/api/pdfs/:id` | — | Delete. |
| `GET` | `/api/pdfs/:id/export` | — | Download (`Content-Disposition: attachment`). |
| `GET` | `/api/pdfs/:id/file` | — | **Inline** bytes (`application/pdf`, `no-store`) for the viewer. |
| `GET` | `/api/pdfs/:id/pages/:page` | `?dpi=` | Server-render a page to PNG. |
| `GET` | `/api/pdfs/:id/text/:page` | — | Extracted text of one page. |
| `GET` | `/api/pdfs/:id/runs/:page` | — | Text runs (for the inline editor). |
| `POST` | `/api/pdfs/:id/edit-text` | `{ page, old, new }` | Inline find & replace (same engine as `pdf_replace_text`). |
| `POST` | `/api/pdfs/:id/rotate` | `{ pages?, degrees, all? }` | Rotate. |
| `POST` | `/api/pdfs/:id/reorder` | `{ order }` | Reorder. |
| `POST` | `/api/pdfs/:id/delete-pages` | `{ pages }` | Delete pages. |
| `POST` | `/api/pdfs/:id/merge` | `{ other_id }` | Merge. |
| `POST` | `/api/pdfs/:id/replace-text` | `{ page, old, new }` | Replace text. |
| `POST` | `/api/pdfs/:id/annotate` | `{ page, kind, rect, text?, color? }` | Add annotation. |
| `POST` | `/api/pdfs/:id/watermark` | `{ text, pages?, all? }` | Watermark. |

## Notes

- Keep `/file` (inline) and `/export` (attachment) **distinct** — the viewer
  must never trigger a download.
- `match_text` (pdf_oxide) can synthesize a trailing space that is not a byte in
  the stream; the inline editor retries with the trimmed text when the exact
  string finds nothing.
- After any server-side edit, the viewer drops its parsed document and
  re-renders (the bytes changed).
