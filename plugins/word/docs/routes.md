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

## `POST /api/documents/import` — import a `.odt`

| | |
|---|---|
| Handler tag | `doc_import` |
| Auth | required |
| Request | `multipart/form-data` with a `file` field; optional query `?name=…` |
| Response | `{ success: true, data: { title } }` |
| Code | `doc_import()`, `src/routes.rs` |

Behaviour: parses the query into `ImportQuery { name: Option<String> }`, then
reads the multipart stream and captures the field named `file` (its bytes and
original filename). The title is `?name=` when present, otherwise the filename
stem, else `"Imported document"`, passed through `clean_title`.

Validation: `odt::odt_to_html(&data)?` is called first, so a corrupt file is
rejected before insertion. The **original bytes are stored unchanged** (no
re-encode), preserving foreign formatting.

Errors: `BadRequest("multipart error: …")`, `BadRequest("read error: …")`,
`BadRequest("missing 'file' field")`, `BadRequest` (invalid ODT/zip).

## `GET /api/documents/:id` — get

| | |
|---|---|
| Handler tag | `doc_get` |
| Auth | required |
| Request | path `:id` |
| Response | `{ success: true, data: { id, title, html, updated_at } }` |
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

## `GET /api/documents/:id/export` — export `.odt`

| | |
|---|---|
| Handler tag | `doc_export` |
| Auth | required |
| Request | path `:id` |
| Response | **raw bytes**, `Content-Type: application/vnd.oasis.opendocument.text`, `Content-Disposition: attachment; filename="<title>.odt"` |
| Code | `doc_export()`, `src/routes.rs` |

The stored `odt` blob is streamed back unmodified, so the download is byte-for-
byte the file LibreOffice would open. `filename_for_title()` sanitises the name
and strips `"` from the header. Not wrapped in the JSON envelope.

Errors: `NotFound("Document not found")`, `Internal("Export failed: …")`.

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
        _ => return None,
    })
}
```

## Related

- [README.md](README.md) · [tools.md](tools.md) · [window.md](window.md) ·
  [architecture.md](architecture.md)
