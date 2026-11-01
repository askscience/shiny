# Word — REST routes

All routes are declared by `route_specs()` in `src/plugin.rs` and implemented in
`src/routes.rs`. Every route requires `auth` (the core authenticates the request
and injects the traveler identity; `user_id()` returns
`AppError::Unauthorized("not authenticated")` when the identity is missing).

- Success envelope (JSON routes): `{ "success": true, "data": … }`.
- Errors use the core `AppError` → HTTP status mapping (`BadRequest` 400,
  `Unauthorized` 401, `NotFound` 404, `Internal` 500).
- `:id` is the document UUID. Path params are read with `take_path()`, which
  requires exactly one parameter.
- DB access is through the synchronous `ctx.db()` (`Value::text`,
  `Value::blob`, `Value::Int`).

## `GET /api/documents` — list

| | |
|---|---|
| Handler tag | `doc_list` |
| Auth | required |
| Request | none |
| Response | `{ success: true, data: [{ id, title, updated_at }] }` |
| Code | `doc_list()`, `src/routes.rs` |

`SELECT id, title, updated_at FROM documents WHERE user_id = ?1 ORDER BY
updated_at DESC LIMIT 200`.

## `POST /api/documents` — create

| | |
|---|---|
| Handler tag | `doc_create` |
| Auth | required |
| Request | JSON `{ "title"?: string, "html"?: string }` |
| Response | `{ success: true, data: { id, title, html, updated_at: "now" } }` |
| Code | `doc_create()`, `src/routes.rs` |

Body struct `Create { title: Option<String>, html: Option<String> }`, both
defaulted. `clean_title` is applied; `html` defaults to empty. The HTML is
encoded via `odt::html_to_odt` and inserted with a fresh UUID. `updated_at` in
the response is the literal `"now"` (the DB value is authoritative on the next
`GET`).

Errors: `BadRequest("invalid JSON body: …")`, `Unauthorized`, `Internal` (codec).

## `POST /api/documents/import` — import a document

| | |
|---|---|
| Handler tag | `doc_import` |
| Auth | required |
| Request | `multipart/form-data` with a `file` field; optional query `?name=…` |
| Response | `{ success: true, data: { id, title } }` |
| Code | `doc_import()`, `src/routes.rs` |

Behaviour: parses the query into `ImportQuery { name: Option<String> }`, then
reads the multipart stream and captures the field named `file` (its bytes and
original filename). The title is `?name=` when present, otherwise the filename
stem, else `"Imported document"`, passed through `clean_title`.

Formats: `.odt`, `.docx` (also `.docm`/`.dotx`), `.rtf`, `.md`, `.html`,
`.txt`, picked by the file name's extension or, without one, sniffed from the
bytes (`engine::import`). The file is parsed first, so an unreadable file is
rejected before insertion. An `.odt` is **stored unchanged**; every other
format is converted to `.odt` content with WordCraft's ODT writer. Headers,
footers and section layout of an imported `.docx` are not kept (the design
regenerates them on export); its text, styles, lists, tables and images are.

Errors: `BadRequest("multipart error: …")`, `BadRequest("missing 'file'
field")`, `BadRequest("Unsupported file type: …")`, `BadRequest("Not a
readable Word document: …")`, `BadRequest("Could not read …")`.

## `GET /api/documents/:id` — get

| | |
|---|---|
| Handler tag | `doc_get` |
| Auth | required |
| Request | path `:id` |
| Response | `{ success: true, data: { id, title, html, updated_at, design } }` |
| Code | `doc_get()`, `src/routes.rs` |

Reads `title, odt, updated_at` scoped by `id` and `user_id`, converts the blob
with `odt::odt_to_html`.

Errors: `NotFound("Document not found")`, `BadRequest` (path), `Internal`
(codec).

## `PUT /api/documents/:id` — save

| | |
|---|---|
| Handler tag | `doc_save` |
| Auth | required |
| Request | JSON `{ "title"?: string, "html"?: string }` |
| Response | `{ success: true, data: { success: true } }` |
| Code | `doc_save()`, `src/routes.rs` |

Body struct `Save { title, html }`. Resolution of the title: if `title` is
present it is cleaned; otherwise the current DB title is reused (falling back to
`Untitled`). `html` defaults to empty, is encoded, and `title`+`odt`+
`updated_at` are updated. A zero-row update yields `NotFound`.

Errors: `NotFound("Document not found")`, `BadRequest("invalid JSON body: …")`,
`Internal` (codec).

## `DELETE /api/documents/:id` — delete

| | |
|---|---|
| Handler tag | `doc_delete` |
| Auth | required |
| Request | path `:id` |
| Response | `{ success: true, data: { success: true } }` |
| Code | `doc_delete()`, `src/routes.rs` |

Scoped `DELETE`; zero rows affected → `NotFound("Document not found")`.

## `GET /api/documents/:id/export` — export

| | |
|---|---|
| Handler tag | `doc_export` |
| Auth | required |
| Request | path `:id`; query `?format=odt\|docx\|pdf\|md\|html\|rtf\|txt` (default `odt`) |
| Response | **raw bytes** with the format's `Content-Type`, `Content-Disposition: attachment; filename="<title>.<ext>"` |
| Code | `doc_export()` → `export_document()`, `src/routes.rs` |

The content is composed with the document's design (`engine::compose`) and
written in the requested format (`engine::export`): `.docx` by
`wordcraft-docx`, `.pdf` by `wordcraft-pdf` (fonts embedded and subset, text
selectable, outline from headings, tagged), the rest by `wordcraft-formats`.
With a plain design an `.odt` export is the stored blob, byte for byte.
`filename_for_title()` sanitises the name. Not wrapped in the JSON envelope.

Errors: `BadRequest("Unknown export format: …")`, `NotFound("Document not
found")`, `Internal("… export failed: …")`.

## `PUT /api/documents/:id/design` — set the design

| | |
|---|---|
| Handler tag | `doc_design` |
| Auth | required |
| Request | JSON `Design`: `{ template, paper, cover, toc, header, page_numbers, subtitle, author, date, lang, accent, margin_top, margin_right, margin_bottom, margin_left }` (all optional; margins in mm, 0 = 25 mm) |
| Response | `{ success: true, data: { success: true, design } }` |
| Code | `doc_design()`, `src/routes.rs` |

Replaces the stored design after `Design::validate()` and bumps `updated_at`.
`GET /api/documents/:id` returns the current design beside the HTML.

Errors: `BadRequest` (invalid JSON, unknown template, paper, colour, or text
over 200 characters), `NotFound("Document not found")`.

## `POST /api/documents/:id/opened` — the window opened it

Handler tag `doc_opened`. Sets `opened_at` (migration `003_opened.sql`), so the
document on screen becomes the default target of the `doc_*` tools (they
order by the later of `updated_at` and `opened_at`). Response
`{ success: true }`; `NotFound` for another user's document.

## `GET /api/documents/:id/pages` — preview pages

Handler tag `doc_pages`. Composes the document with its design and lays it
out (`engine::Preview`, WordCraft layout). Response
`{ count, pages: [{ w, h }] }`, sizes in points. The laid-out document is
cached per (document, `updated_at`, title, design) — four entries — so the
page requests that follow reuse it.

## `GET /api/documents/:id/page/:n` — one rendered page

Handler tag `doc_page`. Query `?scale=` pixels per point (0.25–4, default
1.5). Renders page `n` (1-based) with `wordcraft-render` and returns
`image/png`, `Cache-Control: private, no-store`. `BadRequest` for a page
that is not a number from 1, `NotFound` past the last page.

## `GET /api/word-fonts/:name` — document faces (public)

Handler tag `doc_font`, `RouteSpec::public()`. Serves one of the fonts
WordCraft bundles (`engine::FONT_FILES`: Source Sans 3, Source Serif 4,
Inter, JetBrains Mono; OFL) as `font/ttf` with a one-year immutable cache, so
the window sets the page in the faces the PDF embeds. Unknown names are
`NotFound`; names are matched exactly against the list (no paths).

## Handler dispatch

```rust
pub fn handle(ctx: &Arc<PluginCtx>, tag: &str) -> Option<RouteHandler> {
    let ctx = ctx.clone();
    Some(match tag {
        "doc_list"   => doc_list(ctx),
        "doc_create" => doc_create(ctx),
        "doc_get"    => doc_get(ctx),
        "doc_save"   => doc_save(ctx),
        "doc_delete" => doc_delete(ctx),
        "doc_export" => doc_export(ctx),
        "doc_import" => doc_import(ctx),
        "doc_design" => doc_design(ctx),
        "doc_pages" => doc_pages(ctx),
        "doc_opened" => doc_opened(ctx),
        "doc_page" => doc_page(ctx),
        "doc_font" => doc_font(),
        _ => return None,
    })
}
```

## Related

- [README.md](README.md) · [tools.md](tools.md) · [window.md](window.md) ·
  [architecture.md](architecture.md)
