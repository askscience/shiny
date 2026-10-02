# Image plugin — REST routes

All 25 routes are declared in `route_specs()` (`src/plugin.rs:19`) and dispatched
by tag in `src/routes.rs::handle` (`src/routes.rs:32`). Every route has
`auth: "auth"`: the handler calls `user_id(&req)` → `user_id_from_request`, and
returns `401 Unauthorized` when there is no authenticated user. Every DB query is
additionally scoped by `user_id`.

Responses use the plugin envelope `{ "success": true, "data": { … } }` unless
noted (raw PNG/RGBA bodies). Errors surface as the SDK's `AppError` shape
(`400 BadRequest`, `401 Unauthorized`, `404 NotFound`, `500 Internal`).

The **reorder literal route is registered explicitly before `:layer_id`**, so
`POST /api/images/:id/layers/reorder` does not collide with a layer id.

---

## Documents

### `GET /api/images`

- **Auth:** required.
- **Request:** none.
- **Response:** `{ "images": [ { "image_id", "title", "width", "height", "updated_at" } ], "count": N }` (limit 200, newest first).
- **Code:** `image_list`, `src/routes.rs:342`.

### `POST /api/images`

- **Auth:** required. This is the **upload** route.
- **Request:** either
  - `multipart/form-data` with a `file` field, or
  - a **raw binary body** (any image content type) with optional
    `?name=<file name>`.
- **Behaviour:** decodes the upload, `ops::fit` to `MAX_DIM=1600`, creates the
  `images` row (`format='rgba'`, `original` = current), and seeds a `Background`
  layer so the stack is never empty. Uploads are capped at **32 MB**; raw bodies
  bypass axum's 2 MB `DefaultBodyLimit`.
- **Response:** `{ "image_id", "title", "width", "height", "updated_at": "now" }`.
- **Errors:** `400` unreadable image (PNG/JPEG/GIF/WebP/BMP), `400` “image too
  large (max 32 MB)”.
- **Code:** `image_create`, `src/routes.rs:369`.

### `GET /api/images/:id`

- **Auth:** required.
- **Response:** `{ "image_id", "title", "width", "height", "updated_at", "layer_count" }`.
- **Errors:** `404 Image not found`.
- **Code:** `image_get`, `src/routes.rs:419`.

### `GET /api/images/:id/data`

- **Auth:** required.
- **Response:** `image/png`, `Cache-Control: no-store` — the flattened composite
  (`layers::rendered`), encoded on the fly.
- **Errors:** `404 Image not found`.
- **Code:** `image_data`, `src/routes.rs:446`.

### `PUT /api/images/:id`

- **Auth:** required.
- **Request (JSON):** `{ "title"?: string }` (trimmed/truncated to 120 chars;
  blank → `Untitled`, which is the literal default when omitted).
- **Response:** `{ "image_id", "title" }`.
- **Errors:** `404 Image not found`.
- **Code:** `image_rename`, `src/routes.rs:460`.

### `DELETE /api/images/:id`

- **Auth:** required.
- **Behaviour:** deletes the `images` row **and** all `image_layers` rows for it.
- **Response:** `{ "image_id", "deleted": true }`.
- **Errors:** `404 Image not found`.
- **Code:** `image_delete`, `src/routes.rs:803`.

### `POST /api/images/:id/apply`

The **shared operations entry point** for the window and the agent.

- **Auth:** required.
- **Query:** `?raw=true|false` (default `false`), `?commit=true|false` (default `true`).
- **Request (JSON):** `{ "operations"?: [ {op,…}, … ], "operation"?: {op,…}, "layer_id"?: "…" }`
  (`operations` wins; a non-empty array is required unless `operation` is given).
- **Behaviour:** resolves the target (`active_layer`, default topmost pixel
  layer), reads the document selection and crops it to that layer, runs
  `ops::apply_raw`.
  - `commit:true` → writes the layer + recomposes.
  - `commit:false` → folds the would-be pixels into the in-memory stack and
    composites **without a database write** (live preview).
  - `raw:true` → streams the composite as `application/octet-stream` with
    `x-image-width` / `x-image-height` headers and `no-store`; the window’s
    `rawApply` consumes an `ArrayBuffer`.
- **Response (JSON, when `raw` is falsy):**
  `{ "image_id", "layer_id", "width", "height", "operations_applied", "updated_at": "now" }`.
- **Errors:** `400 operations required`; engine errors as `operation …`; `404`
  image/layer not found.
- **Code:** `image_apply`, `src/routes.rs:489`.

### `GET /api/images/:id/render`

- **Auth:** required.
- **Query:** `?raw=true|false` (default PNG).
- **Response:** `image/png` by default; raw RGBA
  (`application/octet-stream` + dimension headers + `no-store`) when `raw=true`.
- **Code:** `image_render`, `src/routes.rs:575`.

---

## Selection

A selection is a **canvas-sized, 8-bit, single-channel coverage mask** (0
unselected / 255 fully selected), transported base64, row-major. The editor
builds it in `web/selection.js`; server ops confine every non-geometric
operation to it.

### `GET /api/images/:id/selection`

- **Auth:** required.
- **Response:** `{ "image_id", "width", "height", "data": <base64|null> }`.
- **Code:** `image_selection_get`, `src/routes.rs:686`.

### `PUT /api/images/:id/selection`

- **Auth:** required.
- **Request (JSON):** `{ "width"?, "height"?, "data"?: <base64>, "clear"?: bool }`.
  When `clear:true` or `data` is absent the mask is cleared (returns
  `{ "cleared": true }`); otherwise `width`/`height` must be non-zero and
  `data` must decode to at least `width × height` bytes (only the first
  `width×height` are stored).
- **Response:** `{ "image_id", "width", "height" }` or `{ "image_id", "cleared": true }`.
- **Errors:** `400 selection needs width and height`; `400 selection data is not
  valid base64`; `400 selection data is N bytes, need at least M (W×H)`.
- **Code:** `image_selection_set`, `src/routes.rs:725`.

### `DELETE /api/images/:id/selection`

- **Auth:** required.
- **Response:** `{ "image_id", "cleared": true }`.
- **Code:** `image_selection_delete`, `src/routes.rs:782`.

---

## Document geometry

These four routes reflow every layer (see
[architecture.md](architecture.md#document-geometry-documentrs)); each also
clears any active selection.

### `POST /api/images/:id/crop`

- **Request (JSON):** `{ "x", "y", "width", "height" }` (defaults 0/0/0/0).
- **Response:** `{ "image_id", "width", "height" }`.
- **Errors:** `400 crop needs positive width and height`.
- **Code:** `image_crop`, `src/routes.rs:608`; engine `src/document.rs:77`.

### `POST /api/images/:id/resize`

- **Request (JSON):** `{ "width", "height" }`.
- **Response:** `{ "image_id", "width", "height" }` (each side capped at 8192).
- **Errors:** `400 resize needs positive width and height`.
- **Code:** `image_resize`, `src/routes.rs:629`; engine `src/document.rs:93`.

### `POST /api/images/:id/rotate`

- **Request (JSON):** `{ "angle" }` (snapped to the nearest multiple of 90°).
- **Response:** `{ "image_id", "width", "height" }` (w/h swap for 90/270).
- **Code:** `image_rotate`, `src/routes.rs:649`; engine `src/document.rs:124`.

### `POST /api/images/:id/flip`

- **Request (JSON):** `{ "axis"?: "horizontal"|"vertical" }` (default `horizontal`).
- **Response:** `{ "image_id", "width", "height" }`.
- **Code:** `image_flip`, `src/routes.rs:669`; engine `src/document.rs:160`.

---

## Layer stack

### `GET /api/images/:id/layers`

- **Auth:** required.
- **Response:** `{ "layers": [ { "layer_id", "name", "position", "visible",
  "opacity", "blend_mode", "x", "y", "width", "height", "is_group", "group_id",
  "mask": bool, "mask_enabled" } ], "count": N }`.
- **Behaviour:** `ensure_base_layer` first, so the stack is never empty.
- **Code:** `layer_list`, `src/routes.rs:827`.

### `POST /api/images/:id/layers`

Tri-modal create/import route.

- **JSON body** (`application/json`): `{ "name"?, "group_id"?, "width"?,
  "height"?, "is_group"?: bool }`. `is_group:true` creates a folder; else a
  transparent layer sized to the document (or to `width`/`height`).
  Response: `{ "layer_id", "name", "width", "height" }` or
  `{ "layer_id", "name", "is_group": true }`.
- **Multipart** (`multipart/form-data`, `file` + optional `name`/`group_id
  fields`) **or any non-JSON body** (raw image bytes, optional `?name=` &
  `?group_id=`): imports the image as a new layer, sizing the canvas if the
  document has none yet.
  Response: `{ "layer_id", "name", "width", "height" }`.
- **Errors:** `400 missing 'file' field`; `400 image too large (max 32 MB)`;
  `400` unreadable image.
- **Code:** `layer_create`, `src/routes.rs:854`; import helper
  `insert_uploaded_layer`, `src/routes.rs:254`.

### `POST /api/images/:id/layers/reorder`

- **Request (JSON) — one of:**
  - `{ "ids": ["…","…"] }` — assign positions `0..n` (bottom-to-top) for these
    siblings; or
  - `{ "layer_id": "…", "before_id": "…" }` / `{ "layer_id": "…", "after_id": "…" }`
    — move one layer next to a sibling.
- **Response:** `{ "width", "height" }` (the recomposited canvas size).
- **Errors:** `400 pass `ids` or `layer_id` + `before_id``.
- **Code:** `layer_reorder`, `src/routes.rs:1073`; `set_positions`,
  `src/layers.rs:590`.

### `PUT /api/images/:id/layers/:layer_id`

- **Request (JSON):** `{ "name"?, "visible"?, "opacity"?, "blend_mode"?, "x"?,
  "y"?, "group_id"?: string|null }`. `group_id:null` moves to the root; a string
  moves it into that folder. `opacity` is clamped and bound as text then
  `CAST(… AS REAL)`.
- **Response:** `{ "layer_id", "width", "height" }`.
- **Errors:** `404 Layer not found`.
- **Code:** `layer_update`, `src/routes.rs:928`.

### `DELETE /api/images/:id/layers/:layer_id`

- **Behaviour:** deleting a folder reparents its children to the folder's parent.
- **Response:** `{ "layer_id", "deleted": true }`.
- **Code:** `layer_delete`, `src/routes.rs:959`.

### `GET /api/images/:id/layers/:layer_id/thumb`

- **Auth:** required.
- **Response:** `image/png`, `no-store` — a 44px-longest-side preview that
  preserves transparency (groups return a 1×1 transparent PNG).
- **Code:** `layer_thumb`, `src/routes.rs:1006`; `ops::thumbnail_png`.

### `POST /api/images/:id/layers/:layer_id/image`

- **Request:** multipart `file` **or** raw image bytes.
- **Behaviour:** decodes (`ops::fit` to 1600), replaces the layer's pixels and
  `original`, keeping the layer's existing `x`/`y`.
- **Response:** `{ "layer_id", "width", "height" }`.
- **Code:** `layer_image`, `src/routes.rs:975`.

### `POST /api/images/:id/layers/:layer_id/duplicate`

- **Response:** `{ "layer_id": <new>, "duplicated_from": <old> }`.
- **Behaviour:** copies every column (pixels, mask, blend, opacity, placement),
  names it `"<name> copy"`, appends it as the last sibling.
- **Code:** `layer_duplicate`, `src/routes.rs:1030`; `src/layers.rs:606`.

### `POST /api/images/:id/layers/:layer_id/merge`

- **Response:** `{ "merged": "<layer_id>", "width", "height" }`.
- **Errors:** same as the tool (folder / bottom-most / not found).
- **Code:** `layer_merge`, `src/routes.rs:1046`; `src/layers.rs:657`.

### `POST /api/images/:id/flatten`

- **Response:** `{ "image_id", "flattened": true, "width", "height" }`.
- **Code:** `image_flatten`, `src/routes.rs:1117`; `src/layers.rs:714`.

---

## Notes on request plumbing

- **Path params** are read via `path_params_from_request`; `path_param(req, name)`
  falls back to the first positional value for older `:id`-only routes
  (`src/routes.rs:88`).
- **Bodies** are captured with small axum extractors that preserve the request
  for downstream use: `take_query`, `take_json` (`src/routes.rs:107`).
- **Raw uploads** are read with `axum::body::to_bytes(…, MAX_UPLOAD + 1)` so the
  2 MB default body limit does not apply (`read_raw_upload`, `src/routes.rs:143`).
- **`is_multipart` / `is_json`** switch the create/replace paths by content type
  (`src/routes.rs:186`).
